//! Destination classification as a staged decision procedure.
//!
//! `planner::classify_destination` asks the filesystem up to five questions
//! about a destination path, in a fixed order, and stops as soon as the
//! answers so far decide the [`DestState`]. This module owns that decision.
//! Production keeps the questions themselves: it calls [`classify_step`] with
//! what it has observed so far ([`DestObs`]), performs the one query the
//! kernel names ([`DestQuery`]), records the answer, and repeats until the
//! kernel answers [`DestStep::Done`].
//!
//! # Specification
//!
//! * `classify_spec` is the precedence rule over a complete set of answers
//!   ([`DestWorld`]): a tracked destination is a tracked conflict whatever the
//!   filesystem says, then a symlinked parent is an unsafe path, then a
//!   missing entry is missing, then anything that is a symlink or not a
//!   regular file is a type conflict, and only then does the comparison
//!   decide. A comparison error is an untracked conflict, never "up to date".
//! * `dest_queries` is the lazy evaluation order: the exact sequence of
//!   queries a short-circuiting evaluation of that rule performs.
//!
//! # Theorems
//!
//! * [`classify_step`]'s `ensures`: an answer given early is sound — it is
//!   `classify_spec(w)` for every complete world `w` that agrees with what was
//!   observed, so the skipped queries could not have changed it — and a query
//!   it asks for is the first unanswered query of `dest_queries(w)`. Driving
//!   the kernel from an empty observation therefore performs exactly
//!   `dest_queries(w)`, in order, at most five times, and returns
//!   `classify_spec(w)` (`theorem_driver_follows_spec`).
//! * `theorem_tracked_precedence`, `theorem_unsafe_parent_precedence`,
//!   `theorem_missing_soundness`, `theorem_fail_closed_comparison`: the
//!   safety facts a reviewer needs from the precedence rule.
//!
//! # Trust assumptions
//!
//! The production driver performs the named query against the destination
//! (and, for [`DestQuery::Compare`], the source) and stores the answer in the
//! matching field. `exists` is whatever `FileSystem::exists` reports; on the
//! real filesystem that follows symlinks, so a dangling symlink at the
//! destination is observed as "does not exist" and classified
//! [`DestState::Missing`] (a known quirk this kernel mirrors faithfully).

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// What `FileSystem::compare_files` reported (mirrors `fs::FileComparison`,
/// with an I/O error as its own case).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompareObs {
    /// Byte-identical content and identical relevant permission bits.
    Equal,
    /// Byte-identical content, differing permission bits.
    PermissionsDiffer,
    /// Differing content.
    ContentDiffers,
    /// The comparison failed with an I/O error.
    Error,
}

/// Classification of a destination path (mirrors `model::DestinationState`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DestState {
    /// Destination does not exist.
    Missing,
    /// Destination content and relevant permissions match the source.
    UpToDate,
    /// Destination content matches the source but its permission bits differ.
    PermissionsDiffer,
    /// Destination has an untracked file whose content differs.
    UntrackedConflict,
    /// Destination path is tracked.
    TrackedConflict,
    /// Destination exists but is not a regular file.
    TypeConflict,
    /// Destination parent contains a symlink.
    UnsafePath,
}

/// One filesystem query the classification may need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DestQuery {
    /// `FileSystem::parent_has_symlink(destination)`.
    ParentHasSymlink,
    /// `FileSystem::exists(destination)`.
    Exists,
    /// `FileSystem::is_symlink(destination)`.
    IsSymlink,
    /// `FileSystem::is_file(destination)`.
    IsFile,
    /// `FileSystem::compare_files(source, destination)`.
    Compare,
}

/// The answers observed so far. `None` means "not asked yet".
#[derive(Clone, Copy, Debug)]
pub struct DestObs {
    /// Whether the path is tracked in the destination worktree (known before
    /// any filesystem query).
    pub tracked_in_destination: bool,
    /// Answer to [`DestQuery::ParentHasSymlink`].
    pub parent_has_symlink: Option<bool>,
    /// Answer to [`DestQuery::Exists`].
    pub exists: Option<bool>,
    /// Answer to [`DestQuery::IsSymlink`].
    pub is_symlink: Option<bool>,
    /// Answer to [`DestQuery::IsFile`].
    pub is_file: Option<bool>,
    /// Answer to [`DestQuery::Compare`].
    pub compare: Option<CompareObs>,
}

/// What to do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DestStep {
    /// Perform this query, record its answer, and ask again.
    Query(DestQuery),
    /// The classification is decided.
    Done(DestState),
}

/// A complete set of answers: what the filesystem would report to every
/// query (specification only).
pub struct DestWorld {
    /// Tracked in the destination worktree.
    pub tracked_in_destination: bool,
    /// Answer to [`DestQuery::ParentHasSymlink`].
    pub parent_has_symlink: bool,
    /// Answer to [`DestQuery::Exists`].
    pub exists: bool,
    /// Answer to [`DestQuery::IsSymlink`].
    pub is_symlink: bool,
    /// Answer to [`DestQuery::IsFile`].
    pub is_file: bool,
    /// Answer to [`DestQuery::Compare`].
    pub compare: CompareObs,
}

/// The state a comparison result decides for an existing regular file.
/// `ContentDiffers` and a comparison error both mean "untracked conflict":
/// waft never calls a destination up to date unless a comparison said so.
pub open spec fn compared_state(c: CompareObs) -> DestState {
    match c {
        CompareObs::Equal => DestState::UpToDate,
        CompareObs::PermissionsDiffer => DestState::PermissionsDiffer,
        CompareObs::ContentDiffers => DestState::UntrackedConflict,
        CompareObs::Error => DestState::UntrackedConflict,
    }
}

/// The precedence rule: tracked, then unsafe parent, then missing, then type
/// conflict, then the comparison.
///
/// Quirk mirrored from production: `exists` follows symlinks on the real
/// filesystem, so a dangling destination symlink reaches the `Missing` arm.
pub open spec fn classify_spec(w: DestWorld) -> DestState {
    if w.tracked_in_destination {
        DestState::TrackedConflict
    } else if w.parent_has_symlink {
        DestState::UnsafePath
    } else if !w.exists {
        DestState::Missing
    } else if w.is_symlink || !w.is_file {
        DestState::TypeConflict
    } else {
        compared_state(w.compare)
    }
}

/// The queries a short-circuiting evaluation of `classify_spec` performs,
/// in order. `is_symlink || !is_file` only asks `is_file` when the entry is
/// not a symlink.
pub open spec fn dest_queries(w: DestWorld) -> Seq<DestQuery> {
    if w.tracked_in_destination {
        seq![]
    } else if w.parent_has_symlink {
        seq![DestQuery::ParentHasSymlink]
    } else if !w.exists {
        seq![DestQuery::ParentHasSymlink, DestQuery::Exists]
    } else if w.is_symlink {
        seq![DestQuery::ParentHasSymlink, DestQuery::Exists, DestQuery::IsSymlink]
    } else if !w.is_file {
        seq![DestQuery::ParentHasSymlink, DestQuery::Exists, DestQuery::IsSymlink, DestQuery::IsFile]
    } else {
        seq![
            DestQuery::ParentHasSymlink,
            DestQuery::Exists,
            DestQuery::IsSymlink,
            DestQuery::IsFile,
            DestQuery::Compare,
        ]
    }
}

/// `opt` is unobserved, or observed with value `v`.
pub open spec fn agrees_with<T>(opt: Option<T>, v: T) -> bool {
    match opt {
        Some(x) => x == v,
        None => true,
    }
}

impl DestObs {
    /// No filesystem query has been answered yet.
    pub open spec fn is_initial(&self) -> bool {
        &&& self.parent_has_symlink is None
        &&& self.exists is None
        &&& self.is_symlink is None
        &&& self.is_file is None
        &&& self.compare is None
    }

    /// Whether query `q` has been answered.
    pub open spec fn answered(&self, q: DestQuery) -> bool {
        match q {
            DestQuery::ParentHasSymlink => self.parent_has_symlink is Some,
            DestQuery::Exists => self.exists is Some,
            DestQuery::IsSymlink => self.is_symlink is Some,
            DestQuery::IsFile => self.is_file is Some,
            DestQuery::Compare => self.compare is Some,
        }
    }

    /// Every answer recorded so far is the one world `w` gives.
    pub open spec fn agrees(&self, w: DestWorld) -> bool {
        &&& self.tracked_in_destination == w.tracked_in_destination
        &&& agrees_with(self.parent_has_symlink, w.parent_has_symlink)
        &&& agrees_with(self.exists, w.exists)
        &&& agrees_with(self.is_symlink, w.is_symlink)
        &&& agrees_with(self.is_file, w.is_file)
        &&& agrees_with(self.compare, w.compare)
    }

    /// Record world `w`'s answer to `q`.
    pub open spec fn answer(self, q: DestQuery, w: DestWorld) -> DestObs {
        match q {
            DestQuery::ParentHasSymlink => DestObs { parent_has_symlink: Some(w.parent_has_symlink), ..self },
            DestQuery::Exists => DestObs { exists: Some(w.exists), ..self },
            DestQuery::IsSymlink => DestObs { is_symlink: Some(w.is_symlink), ..self },
            DestQuery::IsFile => DestObs { is_file: Some(w.is_file), ..self },
            DestQuery::Compare => DestObs { compare: Some(w.compare), ..self },
        }
    }

    /// The observation before any filesystem query.
    pub fn new(tracked_in_destination: bool) -> (o: DestObs)
        ensures
            o.tracked_in_destination == tracked_in_destination,
            o.is_initial(),
    {
        DestObs {
            tracked_in_destination,
            parent_has_symlink: None,
            exists: None,
            is_symlink: None,
            is_file: None,
            compare: None,
        }
    }
}

/// Position of each query in the fixed evaluation order.
pub open spec fn query_rank(q: DestQuery) -> int {
    match q {
        DestQuery::ParentHasSymlink => 0,
        DestQuery::Exists => 1,
        DestQuery::IsSymlink => 2,
        DestQuery::IsFile => 3,
        DestQuery::Compare => 4,
    }
}

/// `q` is the first query of `qs` that `o` has not answered.
pub open spec fn first_unanswered(o: DestObs, qs: Seq<DestQuery>, q: DestQuery) -> bool {
    exists|i: int|
        0 <= i < qs.len() && #[trigger] qs[i] == q && forall|j: int| 0 <= j < i ==> o.answered(#[trigger] qs[j])
}

/// Every query of `qs` has been answered.
pub open spec fn all_answered(o: DestObs, qs: Seq<DestQuery>) -> bool {
    forall|i: int| 0 <= i < qs.len() ==> o.answered(#[trigger] qs[i])
}

/// The contract of one step, for every world consistent with `o`.
pub open spec fn step_sound(o: DestObs, s: DestStep) -> bool {
    match s {
        DestStep::Query(q) => !o.answered(q) && forall|w: DestWorld|
            #[trigger] o.agrees(w) ==> first_unanswered(o, dest_queries(w), q),
        DestStep::Done(d) => forall|w: DestWorld|
            #[trigger] o.agrees(w) ==> d == classify_spec(w) && all_answered(o, dest_queries(w)),
    }
}

/// Decide the destination state from the answers so far, or name the next
/// query.
pub fn classify_step(o: &DestObs) -> (s: DestStep)
    ensures
        step_sound(*o, s),
{
    if o.tracked_in_destination {
        return DestStep::Done(DestState::TrackedConflict);
    }
    let ps = match o.parent_has_symlink {
        None => {
            proof {
                assert forall|w: DestWorld| #[trigger] o.agrees(w) implies first_unanswered(
                    *o,
                    dest_queries(w),
                    DestQuery::ParentHasSymlink,
                ) by {
                    assert(dest_queries(w)[0] == DestQuery::ParentHasSymlink);
                }
            }
            return DestStep::Query(DestQuery::ParentHasSymlink);
        },
        Some(v) => v,
    };
    if ps {
        return DestStep::Done(DestState::UnsafePath);
    }
    let exists = match o.exists {
        None => {
            proof {
                assert forall|w: DestWorld| #[trigger] o.agrees(w) implies first_unanswered(
                    *o,
                    dest_queries(w),
                    DestQuery::Exists,
                ) by {
                    assert(dest_queries(w)[1] == DestQuery::Exists);
                }
            }
            return DestStep::Query(DestQuery::Exists);
        },
        Some(v) => v,
    };
    if !exists {
        return DestStep::Done(DestState::Missing);
    }
    let symlink = match o.is_symlink {
        None => {
            proof {
                assert forall|w: DestWorld| #[trigger] o.agrees(w) implies first_unanswered(
                    *o,
                    dest_queries(w),
                    DestQuery::IsSymlink,
                ) by {
                    assert(dest_queries(w)[2] == DestQuery::IsSymlink);
                }
            }
            return DestStep::Query(DestQuery::IsSymlink);
        },
        Some(v) => v,
    };
    if symlink {
        return DestStep::Done(DestState::TypeConflict);
    }
    let file = match o.is_file {
        None => {
            proof {
                assert forall|w: DestWorld| #[trigger] o.agrees(w) implies first_unanswered(
                    *o,
                    dest_queries(w),
                    DestQuery::IsFile,
                ) by {
                    assert(dest_queries(w)[3] == DestQuery::IsFile);
                }
            }
            return DestStep::Query(DestQuery::IsFile);
        },
        Some(v) => v,
    };
    if !file {
        return DestStep::Done(DestState::TypeConflict);
    }
    match o.compare {
        None => {
            proof {
                assert forall|w: DestWorld| #[trigger] o.agrees(w) implies first_unanswered(
                    *o,
                    dest_queries(w),
                    DestQuery::Compare,
                ) by {
                    assert(dest_queries(w)[4] == DestQuery::Compare);
                }
            }
            DestStep::Query(DestQuery::Compare)
        },
        Some(CompareObs::Equal) => DestStep::Done(DestState::UpToDate),
        Some(CompareObs::PermissionsDiffer) => DestStep::Done(DestState::PermissionsDiffer),
        Some(CompareObs::ContentDiffers) => DestStep::Done(DestState::UntrackedConflict),
        Some(CompareObs::Error) => DestStep::Done(DestState::UntrackedConflict),
    }
}

// ---------------------------------------------------------------------------
// The driver: what production's loop does with the step function.
// ---------------------------------------------------------------------------

/// How many filesystem queries `o` has answered.
pub open spec fn query_count(o: DestObs) -> nat {
    (if o.parent_has_symlink is Some { 1nat } else { 0nat }) + (if o.exists is Some {
        1nat
    } else {
        0nat
    }) + (if o.is_symlink is Some { 1nat } else { 0nat }) + (if o.is_file is Some {
        1nat
    } else {
        0nat
    }) + (if o.compare is Some { 1nat } else { 0nat })
}

/// Answering an unanswered query adds exactly that query to the answered set
/// and keeps the observation consistent with the world.
pub proof fn lemma_answer_extends(o: DestObs, q: DestQuery, w: DestWorld)
    requires
        o.agrees(w),
        !o.answered(q),
    ensures
        o.answer(q, w).agrees(w),
        o.answer(q, w).answered(q),
        forall|p: DestQuery| #[trigger] o.answered(p) ==> o.answer(q, w).answered(p),
        forall|p: DestQuery| p != q ==> (#[trigger] o.answer(q, w).answered(p) == o.answered(p)),
        query_count(o.answer(q, w)) == query_count(o) + 1,
        o.answer(q, w).tracked_in_destination == o.tracked_in_destination,
{
}

/// The driver theorem. Suppose production starts from an initial observation
/// that agrees with `w` and has answered exactly the first `k` queries of
/// `dest_queries(w)`; then the kernel's next step either asks query `k`
/// (the next one in spec order) or, once all of them are answered, returns
/// `classify_spec(w)`. By induction from `k = 0`, the run performs exactly
/// `dest_queries(w)` in order (at most five queries) and ends with
/// `classify_spec(w)`.
pub proof fn theorem_driver_follows_spec(o: DestObs, w: DestWorld, k: int, s: DestStep)
    requires
        o.agrees(w),
        0 <= k <= dest_queries(w).len(),
        forall|i: int| 0 <= i < k ==> o.answered(#[trigger] dest_queries(w)[i]),
        forall|q: DestQuery| #[trigger] o.answered(q) ==> exists|i: int| 0 <= i < k && dest_queries(w)[i] == q,
        step_sound(o, s),
    ensures
        k < dest_queries(w).len() ==> s == DestStep::Query(dest_queries(w)[k]),
        k == dest_queries(w).len() ==> s == DestStep::Done(classify_spec(w)),
{
    let qs = dest_queries(w);
    // Query `i` of the spec order is the query of rank `i`, so the order has
    // no repeats.
    assert forall|i: int| 0 <= i < qs.len() implies query_rank(#[trigger] qs[i]) == i by {}
    match s {
        DestStep::Query(q) => {
            assert(first_unanswered(o, qs, q));
            let i = choose|i: int|
                0 <= i < qs.len() && #[trigger] qs[i] == q && forall|j: int|
                    0 <= j < i ==> o.answered(#[trigger] qs[j]);
            // q is unanswered, so i >= k; every query before i is answered, so i <= k.
            if i < k {
                assert(o.answered(qs[i]));
            }
            if i > k {
                assert(o.answered(qs[k]));
                let j = choose|j: int| 0 <= j < k && qs[j] == qs[k];
                assert(query_rank(qs[j]) == j);
            }
            assert(i == k);
        },
        DestStep::Done(d) => {
            assert(d == classify_spec(w));
            if k < qs.len() {
                assert(o.answered(qs[k]));
                let j = choose|j: int| 0 <= j < k && qs[j] == qs[k];
                assert(query_rank(qs[j]) == j);
            }
        },
    }
}

/// Every observation agrees with some complete world, so the "for every world
/// that agrees" contracts above are never vacuous.
pub proof fn lemma_some_world_agrees(o: DestObs)
    ensures
        exists|w: DestWorld| #[trigger] o.agrees(w),
{
    let w = DestWorld {
        tracked_in_destination: o.tracked_in_destination,
        parent_has_symlink: match o.parent_has_symlink {
            Some(v) => v,
            None => false,
        },
        exists: match o.exists {
            Some(v) => v,
            None => false,
        },
        is_symlink: match o.is_symlink {
            Some(v) => v,
            None => false,
        },
        is_file: match o.is_file {
            Some(v) => v,
            None => false,
        },
        compare: match o.compare {
            Some(v) => v,
            None => CompareObs::Error,
        },
    };
    assert(o.agrees(w));
}

// ---------------------------------------------------------------------------
// Safety facts about the precedence rule.
// ---------------------------------------------------------------------------

/// A destination tracked in the destination worktree is a tracked conflict,
/// whatever the filesystem reports, and no filesystem query is made for it.
pub proof fn theorem_tracked_precedence(w: DestWorld)
    requires
        w.tracked_in_destination,
    ensures
        classify_spec(w) == DestState::TrackedConflict,
        dest_queries(w).len() == 0,
{
}

/// An untracked destination below a symlinked parent is an unsafe path,
/// whether or not anything exists there.
pub proof fn theorem_unsafe_parent_precedence(w: DestWorld)
    requires
        !w.tracked_in_destination,
        w.parent_has_symlink,
    ensures
        classify_spec(w) == DestState::UnsafePath,
{
}

/// "Missing" is only ever said of an untracked path with no symlinked parent
/// that `exists` reported absent.
pub proof fn theorem_missing_soundness(w: DestWorld)
    requires
        classify_spec(w) == DestState::Missing,
    ensures
        !w.tracked_in_destination,
        !w.parent_has_symlink,
        !w.exists,
{
}

/// Fail-closed comparison: "up to date" and "permissions differ" are only
/// said of an untracked, existing, non-symlink regular file whose comparison
/// returned exactly that answer; a comparison error never yields either.
pub proof fn theorem_fail_closed_comparison(w: DestWorld)
    ensures
        classify_spec(w) == DestState::UpToDate ==> {
            &&& !w.tracked_in_destination
            &&& !w.parent_has_symlink
            &&& w.exists
            &&& !w.is_symlink
            &&& w.is_file
            &&& w.compare == CompareObs::Equal
        },
        classify_spec(w) == DestState::PermissionsDiffer ==> {
            &&& !w.tracked_in_destination
            &&& !w.parent_has_symlink
            &&& w.exists
            &&& !w.is_symlink
            &&& w.is_file
            &&& w.compare == CompareObs::PermissionsDiffer
        },
        w.compare == CompareObs::Error ==> classify_spec(w) != DestState::UpToDate
            && classify_spec(w) != DestState::PermissionsDiffer,
{
}

} // verus!
