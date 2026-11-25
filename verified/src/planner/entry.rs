//! The planned entry for one eligible path, as a staged decision procedure.
//!
//! For each eligible path the planner looks at the source (one `lstat`),
//! classifies the destination ([`super::destination`]), may re-check the
//! destination's parents for symlinks, and may snapshot the source and then
//! the destination. Which of those it does depends on the earlier answers,
//! on `--overwrite`, and on whether the platform can replace an existing
//! destination. This module decides both what to ask next and what the
//! answers mean.
//!
//! Production calls [`plan_entry_step`] with what it has observed so far
//! ([`EntryObs`]). The kernel either hands the observation back with the one
//! query to perform next ([`EntryStep::Query`]) or consumes it and returns the
//! [`EntryPlan`] ([`EntryStep::Done`]). Snapshots and I/O errors travel through
//! the kernel as opaque payloads (`S`, `E`), so a planned copy *moves* the
//! very snapshot that was observed into the plan; production never re-derives
//! it and never needs an `unwrap`.
//!
//! # Specification
//!
//! * `plan_entry_spec` is the decision table over a complete set of answers
//!   ([`EntryWorld`]): a source that cannot be examined is a failure, a
//!   source that is not a regular file is skipped, and then the destination
//!   state decides; conflicts are skipped without `--overwrite`, are a
//!   failure where the platform cannot replace, and otherwise become a
//!   replacement pinned by both snapshots — or a permission repair, but only
//!   if the two pinned snapshots agree on content.
//! * `entry_queries` is the lazy evaluation order of that table.
//!
//! # Theorems
//!
//! * [`plan_entry_step`]'s `ensures` (`entry_step_sound`): an answer is
//!   `plan_entry_spec(w)` for every complete world `w` that agrees with the
//!   observation, after every query of `entry_queries(w)` was answered; a
//!   query is the first unanswered one of `entry_queries(w)`; a planned copy
//!   carries exactly the observed source snapshot (and, for a replacement or
//!   repair, the observed destination snapshot), and a failure carries the
//!   observed error. `theorem_driver_follows_spec` turns this into: the run
//!   performs exactly `entry_queries(w)`, in order, and returns
//!   `plan_entry_spec(w)`.
//! * `theorem_tracked_never_written`: a destination tracked in the
//!   destination worktree is never planned for writing and never
//!   snapshotted, whatever `--overwrite` and the platform say.
//! * `theorem_no_clobber_without_overwrite`: without `--overwrite` the only
//!   copy is to a missing destination and the destination is never
//!   snapshotted.
//! * `theorem_copy_missing_iff`, `theorem_copy_carries_source_snapshot`,
//!   `theorem_overwrite_requirements`: when each kind of copy happens.
//! * `theorem_repair_soundness`: a permission repair is only planned for a
//!   destination classified "permissions differ" whose pinned snapshot has
//!   the source snapshot's length and fingerprint; when they disagree it is
//!   an ordinary replacement.
//! * `theorem_unsupported_platform`: where replacement is unsupported, an
//!   `--overwrite` conflict is a failure and nothing existing is replaced.
//! * `theorem_lost_source_fails`: a source that cannot be examined is a
//!   failure, never a skip, and a non-regular source is skipped; neither
//!   looks at the destination.
//! * `theorem_skip_reason_map`: the one-to-one map from destination state to
//!   skip reason.
//! * `theorem_snapshot_order`: the destination is snapshotted only after a
//!   successful source snapshot, and only under `--overwrite` on a platform
//!   that supports it.
//! * `theorem_end_to_end_*`: the same facts stated against the destination
//!   world, through `super::destination::classify_spec`.
//!
//! # Trust assumptions
//!
//! Production performs the query the kernel names and records its answer in
//! the matching field: `FileSystem::source_kind` for the source,
//! `classify_destination` (itself driven by [`super::destination`]) for the
//! destination, `FileSystem::parent_has_symlink` for the re-check, and
//! `FileSystem::file_snapshot` for the snapshots, with [`Snapshot::key`] set
//! to the snapshot's `(len, content_fingerprint)`. Content identity of two
//! snapshots is judged by that key (a 64-bit FNV-1a fingerprint plus the
//! length).

#[allow(unused_imports)]
use vstd::prelude::*;

use super::destination::DestState;
#[cfg(verus_keep_ghost)]
use super::destination::{CompareObs, DestWorld, agrees_with, classify_spec};

verus! {

/// What `FileSystem::source_kind` reported when it could examine the source
/// (mirrors `fs::SourceKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKindObs {
    /// A regular file.
    RegularFile,
    /// A symlink, whatever it points at.
    Symlink,
    /// Anything else that is there: a directory, socket, device, or FIFO.
    Other,
}

/// What identifies a snapshot's content: its length and its 64-bit content
/// fingerprint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentKey {
    /// File length in bytes.
    pub len: u64,
    /// Content fingerprint.
    pub fingerprint: u64,
}

/// Whether two content keys describe the same bytes: the check behind
/// [`plan_entry_step`]'s permission-repair guard, which keeps a repair only
/// while the pinned destination snapshot has the source snapshot's key and
/// otherwise plans a replacement. Production does not call it directly;
/// `FileSnapshot::content_key` supplies the keys it compares.
pub fn same_content(a: &ContentKey, b: &ContentKey) -> (r: bool)
    ensures
        r == (a.len == b.len && a.fingerprint == b.fingerprint),
{
    a.len == b.len && a.fingerprint == b.fingerprint
}

/// A snapshot as the kernel sees it: its content key, plus the production
/// snapshot it was taken from, which the kernel only moves.
pub struct Snapshot<S> {
    /// The snapshot's `(len, content_fingerprint)`.
    pub key: ContentKey,
    /// The production snapshot.
    pub payload: S,
}

/// The answers observed so far for one path. `None` means "not asked yet";
/// an `Err` carries the production I/O error.
pub struct EntryObs<S, E> {
    /// Answer to [`EntryQuery::SourceKind`].
    pub source_kind: Option<Result<SourceKindObs, E>>,
    /// Answer to [`EntryQuery::Destination`].
    pub destination: Option<DestState>,
    /// Answer to [`EntryQuery::ParentHasSymlink`].
    pub parent_has_symlink: Option<bool>,
    /// Answer to [`EntryQuery::SourceSnapshot`].
    pub source_snapshot: Option<Result<Snapshot<S>, E>>,
    /// Answer to [`EntryQuery::DestinationSnapshot`].
    pub destination_snapshot: Option<Result<Snapshot<S>, E>>,
}

/// One query planning may need for a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryQuery {
    /// `FileSystem::source_kind(source)`.
    SourceKind,
    /// `classify_destination(..)`.
    Destination,
    /// `FileSystem::parent_has_symlink(destination)`, the re-check made
    /// before copying to a missing destination.
    ParentHasSymlink,
    /// `FileSystem::file_snapshot(source)`.
    SourceSnapshot,
    /// `FileSystem::file_snapshot(destination)`.
    DestinationSnapshot,
}

/// What a planned copy expects at the destination (mirrors
/// `model::DestinationExpectation`).
pub enum Expectation<S> {
    /// The destination did not exist.
    Missing,
    /// Replace the existing destination pinned by this snapshot.
    ReplaceExisting(S),
    /// Repair the permissions of the destination pinned by this snapshot.
    RepairPermissions(S),
}

/// Why a path is skipped (mirrors `model::SkipReason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipKind {
    /// Untracked destination with different content.
    UntrackedConflict,
    /// Destination content equal, permission bits differ.
    PermissionsDiffer,
    /// Destination path is tracked in the destination worktree.
    TrackedConflict,
    /// Destination exists but is not a regular file.
    TypeConflict,
    /// Destination parent path contains a symlink.
    UnsafePath,
    /// Source is not a regular file.
    UnsupportedSourceType,
}

/// Why a path could not be planned, with the I/O error where there is one.
pub enum Failure<E> {
    /// The source could not be examined at all.
    ExamineSource(E),
    /// The source could not be snapshotted.
    SnapshotSource(E),
    /// The destination could not be snapshotted.
    SnapshotDestination(E),
    /// `--overwrite` asked to replace or repair, and the platform cannot.
    OverwriteUnsupported,
}

/// The planned entry for one path (mirrors `model::PlannedEntry`).
pub enum EntryPlan<S, E> {
    /// Copy, pinned by the source snapshot and the destination expectation.
    Copy {
        /// The source snapshot observed while planning.
        source: S,
        /// What must still be at the destination.
        destination: Expectation<S>,
    },
    /// Already up to date.
    NoOp,
    /// Left alone.
    Skip(SkipKind),
    /// A per-file planning failure.
    Fail(Failure<E>),
}

/// What to do next for one path.
pub enum EntryStep<S, E> {
    /// Perform this query, record its answer in the returned observation,
    /// and ask again.
    Query(EntryObs<S, E>, EntryQuery),
    /// The plan for this path.
    Done(EntryPlan<S, E>),
}

impl<S, E> EntryObs<S, E> {
    /// The observation before any query.
    pub fn new() -> (o: EntryObs<S, E>)
        ensures
            o.is_initial(),
    {
        EntryObs {
            source_kind: None,
            destination: None,
            parent_has_symlink: None,
            source_snapshot: None,
            destination_snapshot: None,
        }
    }

    /// No query has been answered yet.
    pub open spec fn is_initial(&self) -> bool {
        &&& self.source_kind is None
        &&& self.destination is None
        &&& self.parent_has_symlink is None
        &&& self.source_snapshot is None
        &&& self.destination_snapshot is None
    }

    /// Whether query `q` has been answered.
    pub open spec fn answered(&self, q: EntryQuery) -> bool {
        match q {
            EntryQuery::SourceKind => self.source_kind is Some,
            EntryQuery::Destination => self.destination is Some,
            EntryQuery::ParentHasSymlink => self.parent_has_symlink is Some,
            EntryQuery::SourceSnapshot => self.source_snapshot is Some,
            EntryQuery::DestinationSnapshot => self.destination_snapshot is Some,
        }
    }

    /// Every answer recorded so far is the one world `w` gives.
    pub open spec fn agrees(&self, w: EntryWorld) -> bool {
        &&& match self.source_kind {
            Some(Ok(k)) => w.source == source_answer(k),
            Some(Err(_)) => w.source == SourceAnswer::Error,
            None => true,
        }
        &&& agrees_with(self.destination, w.destination)
        &&& agrees_with(self.parent_has_symlink, w.parent_has_symlink)
        &&& snapshot_agrees(self.source_snapshot, w.source_snapshot)
        &&& snapshot_agrees(self.destination_snapshot, w.destination_snapshot)
    }
}

impl<S, E> Default for EntryObs<S, E> {
    fn default() -> (o: Self)
        ensures
            o.is_initial(),
    {
        EntryObs::new()
    }
}

// ---------------------------------------------------------------------------
// Specification.
// ---------------------------------------------------------------------------

/// The answer to [`EntryQuery::SourceKind`] in a complete world.
pub enum SourceAnswer {
    /// A regular file.
    RegularFile,
    /// A symlink.
    Symlink,
    /// Something else.
    Other,
    /// The source could not be examined.
    Error,
}

/// The answer to a snapshot query in a complete world.
pub enum SnapAnswer {
    /// A snapshot with this content key.
    Taken(ContentKey),
    /// The snapshot failed.
    Error,
}

/// A complete set of answers for one path (specification only).
pub struct EntryWorld {
    /// Answer to [`EntryQuery::SourceKind`].
    pub source: SourceAnswer,
    /// Answer to [`EntryQuery::Destination`].
    pub destination: DestState,
    /// Answer to [`EntryQuery::ParentHasSymlink`].
    pub parent_has_symlink: bool,
    /// Answer to [`EntryQuery::SourceSnapshot`].
    pub source_snapshot: SnapAnswer,
    /// Answer to [`EntryQuery::DestinationSnapshot`].
    pub destination_snapshot: SnapAnswer,
}

/// The kind of a planned copy.
pub enum CopyKind {
    /// To a missing destination.
    Missing,
    /// Replacing an existing destination.
    ReplaceExisting,
    /// Repairing an existing destination's permissions.
    RepairPermissions,
}

/// The kind of a planning failure.
pub enum FailKind {
    /// [`Failure::ExamineSource`].
    ExamineSource,
    /// [`Failure::SnapshotSource`].
    SnapshotSource,
    /// [`Failure::SnapshotDestination`].
    SnapshotDestination,
    /// [`Failure::OverwriteUnsupported`].
    OverwriteUnsupported,
}

/// A planned entry without its payloads.
pub enum Decision {
    /// A copy of this kind.
    Copy(CopyKind),
    /// Up to date.
    NoOp,
    /// Skipped for this reason.
    Skip(SkipKind),
    /// Failed for this reason.
    Fail(FailKind),
}

/// The world's answer for an examined source.
pub open spec fn source_answer(k: SourceKindObs) -> SourceAnswer {
    match k {
        SourceKindObs::RegularFile => SourceAnswer::RegularFile,
        SourceKindObs::Symlink => SourceAnswer::Symlink,
        SourceKindObs::Other => SourceAnswer::Other,
    }
}

/// A recorded snapshot answer agrees with the world's.
pub open spec fn snapshot_agrees<S, E>(o: Option<Result<Snapshot<S>, E>>, a: SnapAnswer) -> bool {
    match o {
        Some(Ok(s)) => a == SnapAnswer::Taken(s.key),
        Some(Err(_)) => a == SnapAnswer::Error,
        None => true,
    }
}

/// The skip reason for a destination conflict state.
pub open spec fn conflict_skip(d: DestState) -> SkipKind {
    match d {
        DestState::PermissionsDiffer => SkipKind::PermissionsDiffer,
        DestState::TrackedConflict => SkipKind::TrackedConflict,
        DestState::TypeConflict => SkipKind::TypeConflict,
        DestState::UnsafePath => SkipKind::UnsafePath,
        _ => SkipKind::UntrackedConflict,
    }
}

/// A destination state `--overwrite` may act on.
pub open spec fn is_overwritable(d: DestState) -> bool {
    d == DestState::UntrackedConflict || d == DestState::PermissionsDiffer
}

/// A destination state that is always skipped.
pub open spec fn is_hard_conflict(d: DestState) -> bool {
    d == DestState::TrackedConflict || d == DestState::TypeConflict || d == DestState::UnsafePath
}

/// The decision table for one path.
pub open spec fn plan_entry_spec(w: EntryWorld, overwrite: bool, supported: bool) -> Decision {
    match w.source {
        SourceAnswer::Error => Decision::Fail(FailKind::ExamineSource),
        SourceAnswer::Symlink | SourceAnswer::Other => Decision::Skip(SkipKind::UnsupportedSourceType),
        SourceAnswer::RegularFile => {
            if w.destination == DestState::UpToDate {
                Decision::NoOp
            } else if is_hard_conflict(w.destination) {
                Decision::Skip(conflict_skip(w.destination))
            } else if w.destination == DestState::Missing {
                if w.parent_has_symlink {
                    Decision::Skip(SkipKind::UnsafePath)
                } else if w.source_snapshot is Error {
                    Decision::Fail(FailKind::SnapshotSource)
                } else {
                    Decision::Copy(CopyKind::Missing)
                }
            } else if !overwrite {
                Decision::Skip(conflict_skip(w.destination))
            } else if !supported {
                Decision::Fail(FailKind::OverwriteUnsupported)
            } else {
                match (w.source_snapshot, w.destination_snapshot) {
                    (SnapAnswer::Error, _) => Decision::Fail(FailKind::SnapshotSource),
                    (_, SnapAnswer::Error) => Decision::Fail(FailKind::SnapshotDestination),
                    (SnapAnswer::Taken(s), SnapAnswer::Taken(d)) => if w.destination
                        == DestState::PermissionsDiffer && s == d {
                        Decision::Copy(CopyKind::RepairPermissions)
                    } else {
                        Decision::Copy(CopyKind::ReplaceExisting)
                    },
                }
            }
        },
    }
}

/// The queries a lazy evaluation of `plan_entry_spec` performs, in order.
pub open spec fn entry_queries(w: EntryWorld, overwrite: bool, supported: bool) -> Seq<EntryQuery> {
    if w.source != SourceAnswer::RegularFile {
        seq![EntryQuery::SourceKind]
    } else if w.destination == DestState::Missing {
        if w.parent_has_symlink {
            seq![EntryQuery::SourceKind, EntryQuery::Destination, EntryQuery::ParentHasSymlink]
        } else {
            seq![
                EntryQuery::SourceKind,
                EntryQuery::Destination,
                EntryQuery::ParentHasSymlink,
                EntryQuery::SourceSnapshot,
            ]
        }
    } else if is_overwritable(w.destination) && overwrite && supported {
        if w.source_snapshot is Error {
            seq![EntryQuery::SourceKind, EntryQuery::Destination, EntryQuery::SourceSnapshot]
        } else {
            seq![
                EntryQuery::SourceKind,
                EntryQuery::Destination,
                EntryQuery::SourceSnapshot,
                EntryQuery::DestinationSnapshot,
            ]
        }
    } else {
        seq![EntryQuery::SourceKind, EntryQuery::Destination]
    }
}

/// A planned entry without its payloads.
pub open spec fn decision_of<S, E>(p: EntryPlan<S, E>) -> Decision {
    match p {
        EntryPlan::Copy { destination, .. } => Decision::Copy(
            match destination {
                Expectation::Missing => CopyKind::Missing,
                Expectation::ReplaceExisting(_) => CopyKind::ReplaceExisting,
                Expectation::RepairPermissions(_) => CopyKind::RepairPermissions,
            },
        ),
        EntryPlan::NoOp => Decision::NoOp,
        EntryPlan::Skip(k) => Decision::Skip(k),
        EntryPlan::Fail(f) => Decision::Fail(
            match f {
                Failure::ExamineSource(_) => FailKind::ExamineSource,
                Failure::SnapshotSource(_) => FailKind::SnapshotSource,
                Failure::SnapshotDestination(_) => FailKind::SnapshotDestination,
                Failure::OverwriteUnsupported => FailKind::OverwriteUnsupported,
            },
        ),
    }
}

/// Every payload in the plan is the one that was observed: a copy carries
/// the observed source snapshot, a replacement or repair carries the observed
/// destination snapshot, and a failure carries the observed error.
pub open spec fn payloads_observed<S, E>(o: EntryObs<S, E>, p: EntryPlan<S, E>) -> bool {
    match p {
        EntryPlan::Copy { source, destination } => {
            &&& (o.source_snapshot matches Some(Ok(ss)) && ss.payload == source)
            &&& match destination {
                Expectation::Missing => true,
                Expectation::ReplaceExisting(d) => (o.destination_snapshot matches Some(Ok(ds))
                    && ds.payload == d),
                Expectation::RepairPermissions(d) => (o.destination_snapshot matches Some(Ok(ds))
                    && ds.payload == d),
            }
        },
        EntryPlan::Fail(Failure::ExamineSource(e)) => o.source_kind == Some(Err::<SourceKindObs, E>(e)),
        EntryPlan::Fail(Failure::SnapshotSource(e)) => o.source_snapshot == Some(
            Err::<Snapshot<S>, E>(e),
        ),
        EntryPlan::Fail(Failure::SnapshotDestination(e)) => o.destination_snapshot == Some(
            Err::<Snapshot<S>, E>(e),
        ),
        _ => true,
    }
}

/// Position of each query in the fixed evaluation order.
pub open spec fn query_rank(q: EntryQuery) -> int {
    match q {
        EntryQuery::SourceKind => 0,
        EntryQuery::Destination => 1,
        EntryQuery::ParentHasSymlink => 2,
        EntryQuery::SourceSnapshot => 3,
        EntryQuery::DestinationSnapshot => 4,
    }
}

/// `q` is the first query of `qs` that `o` has not answered.
pub open spec fn first_unanswered<S, E>(o: EntryObs<S, E>, qs: Seq<EntryQuery>, q: EntryQuery) -> bool {
    exists|i: int|
        0 <= i < qs.len()
            && #[trigger] qs[i] == q && forall|j: int| 0 <= j < i ==> o.answered(#[trigger] qs[j])
}

/// Every query of `qs` has been answered.
pub open spec fn all_answered<S, E>(o: EntryObs<S, E>, qs: Seq<EntryQuery>) -> bool {
    forall|i: int| 0 <= i < qs.len() ==> o.answered(#[trigger] qs[i])
}

/// The contract of one step, for every world consistent with `o`.
pub open spec fn entry_step_sound<S, E>(
    o: EntryObs<S, E>,
    overwrite: bool,
    supported: bool,
    s: EntryStep<S, E>,
) -> bool {
    match s {
        EntryStep::Query(o2, q) => {
            &&& o2 == o
            &&& !o.answered(q)
            &&& forall|w: EntryWorld| #[trigger] o.agrees(w) ==> first_unanswered(
                o,
                entry_queries(w, overwrite, supported),
                q,
            )
        },
        EntryStep::Done(p) => {
            &&& payloads_observed(o, p)
            &&& forall|w: EntryWorld| #[trigger] o.agrees(w) ==> decision_of(p) == plan_entry_spec(
                w,
                overwrite,
                supported,
            ) && all_answered(o, entry_queries(w, overwrite, supported))
        },
    }
}

// ---------------------------------------------------------------------------
// The step function.
// ---------------------------------------------------------------------------

/// Decide the plan for one path from the answers so far, or hand the
/// observation back with the next query to perform.
pub fn plan_entry_step<S, E>(o: EntryObs<S, E>, overwrite: bool, overwrite_supported: bool) -> (s:
    EntryStep<S, E>)
    ensures
        entry_step_sound(o, overwrite, overwrite_supported, s),
{
    let ghost o0 = o;
    let ghost qs = |w: EntryWorld| entry_queries(w, overwrite, overwrite_supported);
    let source_kind = o.source_kind;
    let destination = o.destination;
    let parent_has_symlink = o.parent_has_symlink;
    let source_snapshot = o.source_snapshot;
    let destination_snapshot = o.destination_snapshot;
    let kind = match source_kind {
        None => {
            proof {
                assert forall|w: EntryWorld| #[trigger] o0.agrees(w) implies first_unanswered(
                    o0,
                    qs(w),
                    EntryQuery::SourceKind,
                ) by {
                    assert(qs(w)[0] == EntryQuery::SourceKind);
                }
            }
            return EntryStep::Query(
                EntryObs {
                    source_kind: None,
                    destination,
                    parent_has_symlink,
                    source_snapshot,
                    destination_snapshot,
                },
                EntryQuery::SourceKind,
            );
        },
        Some(Err(e)) => {
            proof {
                assert forall|w: EntryWorld| #[trigger] o0.agrees(w)
                    implies all_answered(o0, qs(w)) by {
                    assert(qs(w) =~= seq![EntryQuery::SourceKind]);
                }
            }
            return EntryStep::Done(EntryPlan::Fail(Failure::ExamineSource(e)));
        },
        Some(Ok(k)) => k,
    };
    match kind {
        SourceKindObs::RegularFile => {},
        SourceKindObs::Symlink | SourceKindObs::Other => {
            proof {
                assert forall|w: EntryWorld| #[trigger] o0.agrees(w)
                    implies all_answered(o0, qs(w)) by {
                    assert(qs(w) =~= seq![EntryQuery::SourceKind]);
                }
            }
            return EntryStep::Done(EntryPlan::Skip(SkipKind::UnsupportedSourceType));
        },
    }
    let dest = match destination {
        None => {
            proof {
                assert forall|w: EntryWorld| #[trigger] o0.agrees(w) implies first_unanswered(
                    o0,
                    qs(w),
                    EntryQuery::Destination,
                ) by {
                    assert(qs(w)[0] == EntryQuery::SourceKind);
                    assert(qs(w)[1] == EntryQuery::Destination);
                }
            }
            return EntryStep::Query(
                EntryObs {
                    source_kind: Some(Ok(kind)),
                    destination: None,
                    parent_has_symlink,
                    source_snapshot,
                    destination_snapshot,
                },
                EntryQuery::Destination,
            );
        },
        Some(d) => d,
    };
    proof {
        assert forall|w: EntryWorld| #[trigger] o0.agrees(w) implies {
            &&& w.source == SourceAnswer::RegularFile
            &&& w.destination == dest
            &&& qs(w)[0] == EntryQuery::SourceKind
            &&& qs(w)[1] == EntryQuery::Destination
        } by {}
    }
    match dest {
        DestState::Missing => {
            let ps = match parent_has_symlink {
                None => {
                    proof {
                        assert forall|w: EntryWorld| #[trigger] o0.agrees(w) implies first_unanswered(
                            o0,
                            qs(w),
                            EntryQuery::ParentHasSymlink,
                        ) by {
                            assert(qs(w)[2] == EntryQuery::ParentHasSymlink);
                        }
                    }
                    return EntryStep::Query(
                        EntryObs {
                            source_kind: Some(Ok(kind)),
                            destination: Some(dest),
                            parent_has_symlink: None,
                            source_snapshot,
                            destination_snapshot,
                        },
                        EntryQuery::ParentHasSymlink,
                    );
                },
                Some(v) => v,
            };
            if ps {
                proof {
                    assert forall|w: EntryWorld| #[trigger] o0.agrees(w) implies all_answered(
                        o0,
                        qs(w),
                    ) by {
                        assert(qs(w) =~= seq![
                            EntryQuery::SourceKind,
                            EntryQuery::Destination,
                            EntryQuery::ParentHasSymlink,
                        ]);
                    }
                }
                return EntryStep::Done(EntryPlan::Skip(SkipKind::UnsafePath));
            }
            match source_snapshot {
                None => {
                    proof {
                        assert forall|w: EntryWorld| #[trigger] o0.agrees(w) implies first_unanswered(
                            o0,
                            qs(w),
                            EntryQuery::SourceSnapshot,
                        ) by {
                            assert(qs(w)[2] == EntryQuery::ParentHasSymlink);
                            assert(qs(w)[3] == EntryQuery::SourceSnapshot);
                        }
                    }
                    EntryStep::Query(
                        EntryObs {
                            source_kind: Some(Ok(kind)),
                            destination: Some(dest),
                            parent_has_symlink: Some(ps),
                            source_snapshot: None,
                            destination_snapshot,
                        },
                        EntryQuery::SourceSnapshot,
                    )
                },
                Some(Err(e)) => {
                    proof {
                        assert forall|w: EntryWorld| #[trigger] o0.agrees(w) implies all_answered(
                            o0,
                            qs(w),
                        ) by {
                            assert(qs(w)[2] == EntryQuery::ParentHasSymlink);
                            assert(qs(w)[3] == EntryQuery::SourceSnapshot);
                        }
                    }
                    EntryStep::Done(EntryPlan::Fail(Failure::SnapshotSource(e)))
                },
                Some(Ok(snapshot)) => {
                    proof {
                        assert forall|w: EntryWorld| #[trigger] o0.agrees(w) implies all_answered(
                            o0,
                            qs(w),
                        ) by {
                            assert(qs(w)[2] == EntryQuery::ParentHasSymlink);
                            assert(qs(w)[3] == EntryQuery::SourceSnapshot);
                        }
                    }
                    EntryStep::Done(
                        EntryPlan::Copy { source: snapshot.payload, destination: Expectation::Missing },
                    )
                },
            }
        },
        DestState::UpToDate => {
            proof {
                assert forall|w: EntryWorld| #[trigger] o0.agrees(w)
                    implies all_answered(o0, qs(w)) by {
                    assert(qs(w).len() == 2);
                }
            }
            EntryStep::Done(EntryPlan::NoOp)
        },
        DestState::UntrackedConflict | DestState::PermissionsDiffer => {
            let permissions_only = matches!(dest, DestState::PermissionsDiffer);
            if !overwrite {
                proof {
                    assert forall|w: EntryWorld| #[trigger] o0.agrees(w) implies all_answered(
                        o0,
                        qs(w),
                    ) by {
                        assert(qs(w).len() == 2);
                    }
                }
                return EntryStep::Done(
                    EntryPlan::Skip(
                        if permissions_only {
                            SkipKind::PermissionsDiffer
                        } else {
                            SkipKind::UntrackedConflict
                        },
                    ),
                );
            }
            if !overwrite_supported {
                proof {
                    assert forall|w: EntryWorld| #[trigger] o0.agrees(w) implies all_answered(
                        o0,
                        qs(w),
                    ) by {
                        assert(qs(w).len() == 2);
                    }
                }
                return EntryStep::Done(EntryPlan::Fail(Failure::OverwriteUnsupported));
            }
            let source = match source_snapshot {
                None => {
                    proof {
                        assert forall|w: EntryWorld| #[trigger] o0.agrees(w) implies first_unanswered(
                            o0,
                            qs(w),
                            EntryQuery::SourceSnapshot,
                        ) by {
                            assert(qs(w)[2] == EntryQuery::SourceSnapshot);
                        }
                    }
                    return EntryStep::Query(
                        EntryObs {
                            source_kind: Some(Ok(kind)),
                            destination: Some(dest),
                            parent_has_symlink,
                            source_snapshot: None,
                            destination_snapshot,
                        },
                        EntryQuery::SourceSnapshot,
                    );
                },
                Some(Err(e)) => {
                    proof {
                        assert forall|w: EntryWorld| #[trigger] o0.agrees(w) implies all_answered(
                            o0,
                            qs(w),
                        ) by {
                            assert(qs(w)[2] == EntryQuery::SourceSnapshot);
                            assert(qs(w).len() == 3);
                        }
                    }
                    return EntryStep::Done(EntryPlan::Fail(Failure::SnapshotSource(e)));
                },
                Some(Ok(snapshot)) => snapshot,
            };
            let destination = match destination_snapshot {
                None => {
                    proof {
                        assert forall|w: EntryWorld| #[trigger] o0.agrees(w) implies first_unanswered(
                            o0,
                            qs(w),
                            EntryQuery::DestinationSnapshot,
                        ) by {
                            assert(qs(w)[2] == EntryQuery::SourceSnapshot);
                            assert(qs(w)[3] == EntryQuery::DestinationSnapshot);
                        }
                    }
                    return EntryStep::Query(
                        EntryObs {
                            source_kind: Some(Ok(kind)),
                            destination: Some(dest),
                            parent_has_symlink,
                            source_snapshot: Some(Ok(source)),
                            destination_snapshot: None,
                        },
                        EntryQuery::DestinationSnapshot,
                    );
                },
                Some(Err(e)) => {
                    proof {
                        assert forall|w: EntryWorld| #[trigger] o0.agrees(w) implies all_answered(
                            o0,
                            qs(w),
                        ) by {
                            assert(qs(w)[2] == EntryQuery::SourceSnapshot);
                            assert(qs(w)[3] == EntryQuery::DestinationSnapshot);
                        }
                    }
                    return EntryStep::Done(EntryPlan::Fail(Failure::SnapshotDestination(e)));
                },
                Some(Ok(snapshot)) => snapshot,
            };
            proof {
                assert forall|w: EntryWorld| #[trigger] o0.agrees(w)
                    implies all_answered(o0, qs(w)) by {
                    assert(qs(w)[2] == EntryQuery::SourceSnapshot);
                    assert(qs(w)[3] == EntryQuery::DestinationSnapshot);
                }
            }
            // Classification proved byte equality, but that proof and the two
            // snapshots are separate reads: keep the repair intent only if
            // the pinned snapshots still agree on content.
            if permissions_only && same_content(&destination.key, &source.key) {
                EntryStep::Done(
                    EntryPlan::Copy {
                        source: source.payload,
                        destination: Expectation::RepairPermissions(destination.payload),
                    },
                )
            } else {
                EntryStep::Done(
                    EntryPlan::Copy {
                        source: source.payload,
                        destination: Expectation::ReplaceExisting(destination.payload),
                    },
                )
            }
        },
        DestState::TrackedConflict => {
            proof {
                assert forall|w: EntryWorld| #[trigger] o0.agrees(w)
                    implies all_answered(o0, qs(w)) by {
                    assert(qs(w).len() == 2);
                }
            }
            EntryStep::Done(EntryPlan::Skip(SkipKind::TrackedConflict))
        },
        DestState::TypeConflict => {
            proof {
                assert forall|w: EntryWorld| #[trigger] o0.agrees(w)
                    implies all_answered(o0, qs(w)) by {
                    assert(qs(w).len() == 2);
                }
            }
            EntryStep::Done(EntryPlan::Skip(SkipKind::TypeConflict))
        },
        DestState::UnsafePath => {
            proof {
                assert forall|w: EntryWorld| #[trigger] o0.agrees(w)
                    implies all_answered(o0, qs(w)) by {
                    assert(qs(w).len() == 2);
                }
            }
            EntryStep::Done(EntryPlan::Skip(SkipKind::UnsafePath))
        },
    }
}

// ---------------------------------------------------------------------------
// The driver and non-vacuity.
// ---------------------------------------------------------------------------

/// The driver theorem. Suppose production starts from an observation that
/// agrees with `w` and has answered exactly the first `k` queries of
/// `entry_queries(w, ..)`. Then the kernel's next step asks query `k` (the
/// next one in spec order) or, once all of them are answered, returns a plan
/// whose decision is `plan_entry_spec(w, ..)`. By induction from `k = 0`, the
/// run performs exactly `entry_queries(w, ..)`, in order, and at most five
/// queries.
pub proof fn theorem_driver_follows_spec<S, E>(
    o: EntryObs<S, E>,
    w: EntryWorld,
    overwrite: bool,
    supported: bool,
    k: int,
    s: EntryStep<S, E>,
)
    requires
        o.agrees(w),
        0 <= k <= entry_queries(w, overwrite, supported).len(),
        forall|i: int| 0 <= i < k ==> o.answered(#[trigger] entry_queries(w, overwrite, supported)[i]),
        forall|q: EntryQuery| #[trigger] o.answered(q) ==> exists|i: int|
            0 <= i < k && entry_queries(w, overwrite, supported)[i] == q,
        entry_step_sound(o, overwrite, supported, s),
    ensures
        k < entry_queries(w, overwrite, supported).len() ==> (s matches EntryStep::Query(o2, q) && o2
            == o && q == entry_queries(w, overwrite, supported)[k]),
        k == entry_queries(w, overwrite, supported).len() ==> (s matches EntryStep::Done(p)
            && decision_of(p) == plan_entry_spec(w, overwrite, supported)),
{
    let qs = entry_queries(w, overwrite, supported);
    // The spec order is strictly increasing in rank, so it has no repeats.
    assert forall|i: int, j: int| 0 <= i < j < qs.len()
        implies query_rank(#[trigger] qs[i]) < query_rank(#[trigger] qs[j]) by {}
    match s {
        EntryStep::Query(o2, q) => {
            assert(first_unanswered(o, qs, q));
            let i = choose|i: int|
                0 <= i < qs.len() && #[trigger] qs[i] == q && forall|j: int|
                    0 <= j < i ==> o.answered(#[trigger] qs[j]);
            if i < k {
                assert(o.answered(qs[i]));
            }
            if i > k {
                assert(o.answered(qs[k]));
                let j = choose|j: int| 0 <= j < k && qs[j] == qs[k];
                assert(query_rank(qs[j]) < query_rank(qs[k]));
            }
            assert(i == k);
        },
        EntryStep::Done(p) => {
            if k < qs.len() {
                assert(o.answered(qs[k]));
                let j = choose|j: int| 0 <= j < k && qs[j] == qs[k];
                assert(query_rank(qs[j]) < query_rank(qs[k]));
            }
        },
    }
}

/// Every observation agrees with some complete world, so the "for every world
/// that agrees" contracts are never vacuous.
pub proof fn lemma_some_world_agrees<S, E>(o: EntryObs<S, E>)
    ensures
        exists|w: EntryWorld| #[trigger] o.agrees(w),
{
    let w = EntryWorld {
        source: match o.source_kind {
            Some(Ok(k)) => source_answer(k),
            Some(Err(_)) => SourceAnswer::Error,
            None => SourceAnswer::Error,
        },
        destination: match o.destination {
            Some(d) => d,
            None => DestState::Missing,
        },
        parent_has_symlink: match o.parent_has_symlink {
            Some(v) => v,
            None => false,
        },
        source_snapshot: match o.source_snapshot {
            Some(Ok(s)) => SnapAnswer::Taken(s.key),
            _ => SnapAnswer::Error,
        },
        destination_snapshot: match o.destination_snapshot {
            Some(Ok(s)) => SnapAnswer::Taken(s.key),
            _ => SnapAnswer::Error,
        },
    };
    assert(o.agrees(w));
}

// ---------------------------------------------------------------------------
// Safety theorems about the decision table.
// ---------------------------------------------------------------------------

/// A planned write of any kind.
pub open spec fn is_copy(d: Decision) -> bool {
    d is Copy
}

/// A planned write to a destination that already exists.
pub open spec fn writes_existing(d: Decision) -> bool {
    d == Decision::Copy(CopyKind::ReplaceExisting) || d == Decision::Copy(CopyKind::RepairPermissions)
}

/// A destination tracked in the destination worktree is never planned for
/// writing, and neither file is ever snapshotted for it, whatever
/// `--overwrite` and the platform say. A regular source gets exactly the
/// tracked-conflict skip.
pub proof fn theorem_tracked_never_written(w: EntryWorld, overwrite: bool, supported: bool)
    requires
        w.destination == DestState::TrackedConflict,
    ensures
        !is_copy(plan_entry_spec(w, overwrite, supported)),
        w.source == SourceAnswer::RegularFile ==> plan_entry_spec(w, overwrite, supported)
            == Decision::Skip(SkipKind::TrackedConflict),
        !entry_queries(w, overwrite, supported).contains(EntryQuery::SourceSnapshot),
        !entry_queries(w, overwrite, supported).contains(EntryQuery::DestinationSnapshot),
{
    let qs = entry_queries(w, overwrite, supported);
    assert(forall|i: int| 0 <= i < qs.len() ==> #[trigger] qs[i] != EntryQuery::SourceSnapshot);
    assert(forall|i: int| 0 <= i < qs.len() ==> #[trigger] qs[i] != EntryQuery::DestinationSnapshot);
}

/// No clobbering without `--overwrite`: the only copy is to a missing
/// destination, and the destination is never snapshotted.
pub proof fn theorem_no_clobber_without_overwrite(w: EntryWorld, supported: bool)
    ensures
        !writes_existing(plan_entry_spec(w, false, supported)),
        is_copy(plan_entry_spec(w, false, supported)) ==> w.destination == DestState::Missing,
        !entry_queries(w, false, supported).contains(EntryQuery::DestinationSnapshot),
{
    let qs = entry_queries(w, false, supported);
    assert(forall|i: int| 0 <= i < qs.len() ==> #[trigger] qs[i] != EntryQuery::DestinationSnapshot);
}

/// A copy to a missing destination happens exactly when the source is a
/// regular file, the destination was classified missing, the re-check found
/// no symlinked parent, and the source snapshot succeeded.
pub proof fn theorem_copy_missing_iff(w: EntryWorld, overwrite: bool, supported: bool)
    ensures
        (plan_entry_spec(w, overwrite, supported) == Decision::Copy(CopyKind::Missing)) <==> {
            &&& w.source == SourceAnswer::RegularFile
            &&& w.destination == DestState::Missing
            &&& !w.parent_has_symlink
            &&& w.source_snapshot is Taken
        },
{
}

/// Every copy is of a regular source with a successful source snapshot, and
/// that snapshot query was made. (With `entry_step_sound`, the snapshot the
/// copy carries is exactly the one that query returned.)
pub proof fn theorem_copy_carries_source_snapshot(w: EntryWorld, overwrite: bool, supported: bool)
    requires
        is_copy(plan_entry_spec(w, overwrite, supported)),
    ensures
        w.source == SourceAnswer::RegularFile,
        w.source_snapshot is Taken,
        entry_queries(w, overwrite, supported).contains(EntryQuery::SourceSnapshot),
{
    let qs = entry_queries(w, overwrite, supported);
    if w.destination == DestState::Missing {
        assert(qs[3] == EntryQuery::SourceSnapshot);
    } else {
        assert(qs[2] == EntryQuery::SourceSnapshot);
    }
}

/// Replacing or repairing an existing destination needs `--overwrite`, a
/// platform that supports it, an untracked conflict or permission
/// difference, and successful snapshots of both files.
pub proof fn theorem_overwrite_requirements(w: EntryWorld, overwrite: bool, supported: bool)
    requires
        writes_existing(plan_entry_spec(w, overwrite, supported)),
    ensures
        overwrite,
        supported,
        w.source == SourceAnswer::RegularFile,
        is_overwritable(w.destination),
        w.source_snapshot is Taken,
        w.destination_snapshot is Taken,
        entry_queries(w, overwrite, supported).contains(EntryQuery::DestinationSnapshot),
{
    assert(entry_queries(w, overwrite, supported)[3] == EntryQuery::DestinationSnapshot);
}

/// Repair soundness: a permission repair is only planned for a destination
/// classified "permissions differ" whose pinned snapshot has the source
/// snapshot's content key. If the pinned keys disagree, the same situation
/// is an ordinary replacement; an untracked conflict is never repaired.
pub proof fn theorem_repair_soundness(w: EntryWorld, overwrite: bool, supported: bool)
    ensures
        plan_entry_spec(w, overwrite, supported) == Decision::Copy(CopyKind::RepairPermissions) ==> {
            &&& w.destination == DestState::PermissionsDiffer
            &&& w.source_snapshot is Taken
            &&& w.destination_snapshot is Taken
            &&& w.source_snapshot->Taken_0 == w.destination_snapshot->Taken_0
        },
        ({
            &&& overwrite
            &&& supported
            &&& w.source == SourceAnswer::RegularFile
            &&& w.destination == DestState::PermissionsDiffer
            &&& w.source_snapshot is Taken
            &&& w.destination_snapshot is Taken
            &&& w.source_snapshot->Taken_0 != w.destination_snapshot->Taken_0
        }) ==> plan_entry_spec(w, overwrite, supported) == Decision::Copy(CopyKind::ReplaceExisting),
        w.destination == DestState::UntrackedConflict ==> plan_entry_spec(w, overwrite, supported)
            != Decision::Copy(CopyKind::RepairPermissions),
{
}

/// Where the platform cannot replace an existing destination, nothing
/// existing is written or snapshotted, and an `--overwrite` conflict is a
/// per-file failure rather than a silent skip or a promise that fails later.
pub proof fn theorem_unsupported_platform(w: EntryWorld, overwrite: bool)
    ensures
        !writes_existing(plan_entry_spec(w, overwrite, false)),
        !entry_queries(w, overwrite, false).contains(EntryQuery::DestinationSnapshot),
        overwrite && w.source == SourceAnswer::RegularFile && is_overwritable(w.destination)
            ==> plan_entry_spec(w, overwrite, false) == Decision::Fail(FailKind::OverwriteUnsupported),
{
    let qs = entry_queries(w, overwrite, false);
    assert(forall|i: int| 0 <= i < qs.len() ==> #[trigger] qs[i] != EntryQuery::DestinationSnapshot);
}

/// A source that cannot be examined is a per-file failure, never a skip; a
/// source that is not a regular file is skipped. Either way the destination
/// is not looked at.
pub proof fn theorem_lost_source_fails(w: EntryWorld, overwrite: bool, supported: bool)
    ensures
        w.source == SourceAnswer::Error ==> plan_entry_spec(w, overwrite, supported) == Decision::Fail(
            FailKind::ExamineSource,
        ),
        (w.source == SourceAnswer::Symlink || w.source == SourceAnswer::Other) ==> plan_entry_spec(
            w,
            overwrite,
            supported,
        ) == Decision::Skip(SkipKind::UnsupportedSourceType),
        w.source != SourceAnswer::RegularFile ==> entry_queries(w, overwrite, supported) == seq![
            EntryQuery::SourceKind,
        ],
{
}

/// The skip reason for a destination conflict, as a function of the state
/// alone. Tracked, type and unsafe-path conflicts are skipped whatever the
/// flags; untracked and permission conflicts are skipped exactly without
/// `--overwrite`.
pub proof fn theorem_skip_reason_map(w: EntryWorld, overwrite: bool, supported: bool)
    requires
        w.source == SourceAnswer::RegularFile,
    ensures
        is_hard_conflict(w.destination) ==> plan_entry_spec(w, overwrite, supported) == Decision::Skip(
            conflict_skip(w.destination),
        ),
        is_overwritable(w.destination) && !overwrite ==> plan_entry_spec(w, overwrite, supported)
            == Decision::Skip(conflict_skip(w.destination)),
        w.destination == DestState::PermissionsDiffer ==> conflict_skip(w.destination)
            == SkipKind::PermissionsDiffer,
        w.destination == DestState::UntrackedConflict ==> conflict_skip(w.destination)
            == SkipKind::UntrackedConflict,
        w.destination == DestState::TrackedConflict ==> conflict_skip(w.destination)
            == SkipKind::TrackedConflict,
        w.destination == DestState::TypeConflict ==> conflict_skip(w.destination) == SkipKind::TypeConflict,
        w.destination == DestState::UnsafePath ==> conflict_skip(w.destination) == SkipKind::UnsafePath,
        w.destination == DestState::UpToDate ==> plan_entry_spec(w, overwrite, supported) == Decision::NoOp,
{
}

/// The skip map is one-to-one on the conflict states.
pub proof fn lemma_conflict_skip_injective(a: DestState, b: DestState)
    requires
        is_hard_conflict(a) || is_overwritable(a),
        is_hard_conflict(b) || is_overwritable(b),
        conflict_skip(a) == conflict_skip(b),
    ensures
        a == b,
{
}

/// The destination is snapshotted only after a successful source snapshot,
/// and only under `--overwrite` on a platform that supports it, for an
/// untracked or permission conflict.
pub proof fn theorem_snapshot_order(w: EntryWorld, overwrite: bool, supported: bool)
    requires
        entry_queries(w, overwrite, supported).contains(EntryQuery::DestinationSnapshot),
    ensures
        overwrite,
        supported,
        w.source == SourceAnswer::RegularFile,
        is_overwritable(w.destination),
        w.source_snapshot is Taken,
        entry_queries(w, overwrite, supported) == seq![
            EntryQuery::SourceKind,
            EntryQuery::Destination,
            EntryQuery::SourceSnapshot,
            EntryQuery::DestinationSnapshot,
        ],
{
    let qs = entry_queries(w, overwrite, supported);
    let i = choose|i: int| 0 <= i < qs.len() && qs[i] == EntryQuery::DestinationSnapshot;
    assert(qs[i] == EntryQuery::DestinationSnapshot);
}

// ---------------------------------------------------------------------------
// End to end: the entry decision over the destination classification.
// ---------------------------------------------------------------------------

/// The entry world whose destination answer is the classification of `dw`,
/// which is what production's `classify_destination` returns when driven by
/// [`super::destination`] against `dw`.
pub open spec fn with_destination(w: EntryWorld, dw: DestWorld) -> EntryWorld {
    EntryWorld { destination: classify_spec(dw), ..w }
}

/// A path tracked in the destination worktree is never written, through both
/// kernels, for every filesystem answer and flag.
pub proof fn theorem_end_to_end_tracked_never_written(
    w: EntryWorld,
    dw: DestWorld,
    overwrite: bool,
    supported: bool,
)
    requires
        dw.tracked_in_destination,
    ensures
        !is_copy(plan_entry_spec(with_destination(w, dw), overwrite, supported)),
{
    theorem_tracked_never_written(with_destination(w, dw), overwrite, supported);
}

/// A copy to a missing destination means the path was untracked, had no
/// symlinked destination parent at either check, and `exists` reported it
/// absent.
pub proof fn theorem_end_to_end_copy_missing(w: EntryWorld, dw: DestWorld, overwrite: bool, supported: bool)
    requires
        plan_entry_spec(with_destination(w, dw), overwrite, supported) == Decision::Copy(CopyKind::Missing),
    ensures
        !dw.tracked_in_destination,
        !dw.parent_has_symlink,
        !dw.exists,
        !w.parent_has_symlink,
        w.source == SourceAnswer::RegularFile,
        w.source_snapshot is Taken,
{
}

/// Replacing or repairing an existing destination means it was untracked,
/// had no symlinked parent, existed as a regular non-symlink file, and the
/// comparison did not say "equal".
pub proof fn theorem_end_to_end_writes_existing(
    w: EntryWorld,
    dw: DestWorld,
    overwrite: bool,
    supported: bool,
)
    requires
        writes_existing(plan_entry_spec(with_destination(w, dw), overwrite, supported)),
    ensures
        overwrite,
        supported,
        !dw.tracked_in_destination,
        !dw.parent_has_symlink,
        dw.exists,
        !dw.is_symlink,
        dw.is_file,
        dw.compare != CompareObs::Equal,
{
    theorem_overwrite_requirements(with_destination(w, dw), overwrite, supported);
}

} // verus!
