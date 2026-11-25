//! Verified kernel: list/info output selection.
//!
//! `waft list -v` and `waft info` describe what `waft copy` would do with a
//! path without doing it. Production (`src/subcommands/list.rs`,
//! `src/subcommands/info.rs`) makes the filesystem and Git observations, in
//! the same order and with the same laziness as before, converts them to the
//! plain data below, and prints the label this kernel selects through a
//! per-command string table. This kernel decides:
//!
//! - [`source_kind`]: the `source_kind:` label `info` prints for one `lstat`
//!   result, and from it whether the source is a regular file (which gates
//!   destination classification);
//! - [`predicted_action`]: the action predicted for a regular source from the
//!   destination's classification, including whether the `--overwrite`
//!   remedy is named;
//! - [`list_action`]: the `action:` line of `list -v`, if any;
//! - [`info_destination`]: the `destination:` and `planned_action:` lines of
//!   `info`.
//!
//! # The planner model
//!
//! `spec_plan_outcome` restates, as a specification, the per-file decision
//! `plan_with_overwrite_support` in `src/planner.rs` makes from its
//! observations: the `lstat` source examination, the destination
//! classification, the second parent-symlink check for a missing
//! destination, the source and destination snapshots, `--overwrite`, and the
//! platform's replacement support. It is a mirror kept here so the
//! prediction theorems have something independent to agree with; the planner
//! itself is verified (or not) separately, and a change to the planner's
//! decision must be mirrored here.
//!
//! # Theorems
//!
//! - `theorem_prediction_matches_planner`: for every destination state, the
//!   action `list -v` and `info` predict for a regular source is the outcome
//!   the planner reaches for `copy` without `--overwrite` when nothing changes
//!   between the observations (the parent re-check agrees with the
//!   classification and the source snapshot succeeds), whatever the
//!   destination snapshot would have done.
//! - `theorem_remedy_is_actionable`: the `--overwrite` remedy is named
//!   exactly when re-running with `--overwrite` would plan a write (under the
//!   same no-race conditions), so no output advertises a fix that would fail
//!   or skip.
//! - `theorem_tracked_destination_never_written`: neither the prediction nor
//!   the planner ever writes a tracked, type-conflicting or unsafe
//!   destination, whatever the flags and observations.
//! - `theorem_source_kind_matches_planner`: the `source_kind` label is
//!   `file` exactly when the planner's `lstat` examination of the same
//!   metadata finds a regular file, `missing` exactly when it cannot examine
//!   the source, and the other labels exactly when it skips the source as an
//!   unsupported type. So `info` classifies the destination exactly for the
//!   sources the planner would.
//! - `theorem_list_unsupported_source_matches_planner`: for an existing
//!   source that is not a regular file, `list -v` predicts
//!   `skip (unsupported source type)`, which is what the planner does for a
//!   symlink or other source.
//! - `theorem_info_view`: `planned_action` is printed exactly when the path
//!   is eligible and its destination was classified; it equals the
//!   `list -v` prediction for the same state; the destination label names the
//!   classification one to one (or `exists`/`missing` for an unclassified
//!   destination); and `planned_action: copy` appears only with
//!   `destination: missing`.
//!
//! # Trust assumptions
//!
//! - Production converts `DestinationState` and the `lstat` file type to
//!   [`DestState`] and [`LstatType`] with exhaustive matches, makes the I/O
//!   observations in the same order as before (`list -v`: `is_symlink`, then
//!   `is_file`, then either `exists` or the destination classification;
//!   `info`: the classification only for a regular source, even with
//!   `--quiet`, and the destination `lstat` only for other sources and only
//!   without `--quiet`), and maps each label to today's exact text.
//! - `crate::planner::classify_destination` is the same function the planner
//!   calls; its I/O is outside this kernel.
//!
//! # Quirks encoded faithfully (current behavior, not fixed here)
//!
//! - `list -v` prints no `action:` line for a source that is not a regular
//!   file and does not exist at all (for example a dangling symlink, since
//!   `Path::exists` follows links, or a source that vanished). The planner
//!   would report `skip (unsupported source type)` for the dangling symlink
//!   and a per-file failure for the vanished source. Reachable only through
//!   races, because eligibility requires a regular file.
//! - `list -v` never names the `--overwrite` remedy for an untracked
//!   conflict, although the label it receives carries the remedy flag; its
//!   string table drops it. `info` and `copy --dry-run` name it.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// Classification of a destination path (`model::DestinationState`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DestState {
    /// The destination does not exist.
    Missing,
    /// Content and relevant permissions match the source.
    UpToDate,
    /// Content matches but permission bits differ.
    PermissionsDiffer,
    /// An untracked destination whose content differs (or could not be
    /// compared).
    UntrackedConflict,
    /// The destination path is tracked in the destination worktree.
    TrackedConflict,
    /// The destination exists but is not a regular file.
    TypeConflict,
    /// A parent of the destination is a symlink.
    UnsafePath,
}

/// Why a file is skipped (`model::SkipReason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipKind {
    /// An untracked destination differs.
    UntrackedConflict,
    /// Content equal, permissions differ.
    PermissionsDiffer,
    /// The destination is tracked.
    TrackedConflict,
    /// The destination is not a regular file.
    TypeConflict,
    /// A parent of the destination is a symlink.
    UnsafePath,
    /// The source is not a regular file.
    UnsupportedSourceType,
}

/// A predicted action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionLabel {
    /// The file would be copied.
    Copy,
    /// Nothing to do.
    NoOp,
    /// The file would be skipped.
    Skip {
        /// Why.
        reason: SkipKind,
        /// Whether to name `--overwrite` as the remedy.
        remedy: bool,
    },
}

/// What the planner's `lstat` examination of the source found
/// (`fs::FileSystem::source_kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceExam {
    /// A regular file.
    Regular,
    /// A symlink, whatever it points at.
    Symlink,
    /// Something else that exists.
    Other,
    /// The source could not be examined.
    Unreadable,
}

/// The planner's per-file outcome (`model::PlannedEntry`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanOutcome {
    /// A copy (create, replace, or repair) is planned.
    Copy,
    /// Nothing to do.
    NoOp,
    /// Skipped.
    Skip(SkipKind),
    /// A per-file failure.
    Failure,
}

/// The planner's decision for one path, restated from
/// `plan_with_overwrite_support` in `src/planner.rs`.
///
/// `recheck_unsafe` is the second parent-symlink check the planner makes for
/// a missing destination; `source_snapshot_ok` and `dest_snapshot_ok` say
/// whether the snapshots it takes succeed.
pub open spec fn spec_plan_outcome(
    source: SourceExam,
    dest: DestState,
    recheck_unsafe: bool,
    source_snapshot_ok: bool,
    dest_snapshot_ok: bool,
    overwrite: bool,
    overwrite_supported: bool,
) -> PlanOutcome {
    match source {
        SourceExam::Symlink | SourceExam::Other => PlanOutcome::Skip(SkipKind::UnsupportedSourceType),
        SourceExam::Unreadable => PlanOutcome::Failure,
        SourceExam::Regular => match dest {
            DestState::Missing => if recheck_unsafe {
                PlanOutcome::Skip(SkipKind::UnsafePath)
            } else if !source_snapshot_ok {
                PlanOutcome::Failure
            } else {
                PlanOutcome::Copy
            },
            DestState::UpToDate => PlanOutcome::NoOp,
            DestState::UntrackedConflict | DestState::PermissionsDiffer => if !overwrite {
                PlanOutcome::Skip(
                    if dest is PermissionsDiffer {
                        SkipKind::PermissionsDiffer
                    } else {
                        SkipKind::UntrackedConflict
                    },
                )
            } else if !overwrite_supported || !source_snapshot_ok || !dest_snapshot_ok {
                PlanOutcome::Failure
            } else {
                PlanOutcome::Copy
            },
            DestState::TrackedConflict => PlanOutcome::Skip(SkipKind::TrackedConflict),
            DestState::TypeConflict => PlanOutcome::Skip(SkipKind::TypeConflict),
            DestState::UnsafePath => PlanOutcome::Skip(SkipKind::UnsafePath),
        },
    }
}

/// The planner outcome a label announces.
pub open spec fn spec_label_outcome(label: ActionLabel) -> PlanOutcome {
    match label {
        ActionLabel::Copy => PlanOutcome::Copy,
        ActionLabel::NoOp => PlanOutcome::NoOp,
        ActionLabel::Skip { reason, remedy: _ } => PlanOutcome::Skip(reason),
    }
}

/// Skip reasons that `--overwrite` acts on.
pub open spec fn spec_overwrite_acts_on(reason: SkipKind) -> bool {
    reason is PermissionsDiffer || reason is UntrackedConflict
}

/// The action predicted for a regular source whose destination is
/// classified as `state`: the planner's outcome without `--overwrite`, with
/// the remedy named exactly when `--overwrite` acts on the reason and the
/// platform supports it.
pub open spec fn spec_predicted_action(state: DestState, overwrite_supported: bool) -> ActionLabel {
    match spec_plan_outcome(SourceExam::Regular, state, false, true, true, false, overwrite_supported) {
        PlanOutcome::Copy => ActionLabel::Copy,
        PlanOutcome::NoOp => ActionLabel::NoOp,
        PlanOutcome::Skip(reason) => ActionLabel::Skip {
            reason,
            remedy: overwrite_supported && spec_overwrite_acts_on(reason),
        },
        // Unreachable: the planner without `--overwrite`, a regular source and
        // successful observations never fails (see `theorem_prediction_matches_planner`).
        PlanOutcome::Failure => ActionLabel::NoOp,
    }
}

/// The action predicted for a regular source.
pub fn predicted_action(state: DestState, overwrite_supported: bool) -> (r: ActionLabel)
    ensures
        r == spec_predicted_action(state, overwrite_supported),
{
    match state {
        DestState::Missing => ActionLabel::Copy,
        DestState::UpToDate => ActionLabel::NoOp,
        DestState::PermissionsDiffer => ActionLabel::Skip {
            reason: SkipKind::PermissionsDiffer,
            remedy: overwrite_supported,
        },
        DestState::UntrackedConflict => ActionLabel::Skip {
            reason: SkipKind::UntrackedConflict,
            remedy: overwrite_supported,
        },
        DestState::TrackedConflict => ActionLabel::Skip {
            reason: SkipKind::TrackedConflict,
            remedy: false,
        },
        DestState::TypeConflict => ActionLabel::Skip { reason: SkipKind::TypeConflict, remedy: false },
        DestState::UnsafePath => ActionLabel::Skip { reason: SkipKind::UnsafePath, remedy: false },
    }
}

/// The prediction is the planner's outcome for `copy` without `--overwrite`
/// when nothing changes between observations, whatever the destination
/// snapshot (which that run never takes) would have returned.
pub proof fn theorem_prediction_matches_planner(
    state: DestState,
    overwrite_supported: bool,
    dest_snapshot_ok: bool,
)
    ensures
        spec_label_outcome(spec_predicted_action(state, overwrite_supported)) == spec_plan_outcome(
            SourceExam::Regular,
            state,
            false,
            true,
            dest_snapshot_ok,
            false,
            overwrite_supported,
        ),
        !(spec_plan_outcome(
            SourceExam::Regular,
            state,
            false,
            true,
            dest_snapshot_ok,
            false,
            overwrite_supported,
        ) is Failure),
{
}

/// The `--overwrite` remedy is named exactly when re-running with
/// `--overwrite` would plan a write for the same observations (both
/// snapshots succeeding, the parent re-check agreeing).
pub proof fn theorem_remedy_is_actionable(state: DestState, overwrite_supported: bool)
    ensures
        spec_predicted_action(state, overwrite_supported) matches ActionLabel::Skip {
            reason: _,
            remedy,
        } ==> (remedy <==> spec_plan_outcome(
            SourceExam::Regular,
            state,
            false,
            true,
            true,
            true,
            overwrite_supported,
        ) is Copy),
        spec_predicted_action(state, overwrite_supported) matches ActionLabel::Skip {
            reason: _,
            remedy,
        } ==> (remedy ==> overwrite_supported),
{
}

/// Tracked, type-conflicting and unsafe destinations are never written, by
/// the planner under any flags and observations, or by the prediction.
pub proof fn theorem_tracked_destination_never_written(
    source: SourceExam,
    state: DestState,
    recheck_unsafe: bool,
    source_snapshot_ok: bool,
    dest_snapshot_ok: bool,
    overwrite: bool,
    overwrite_supported: bool,
)
    requires
        state is TrackedConflict || state is TypeConflict || state is UnsafePath,
    ensures
        !(spec_plan_outcome(
            source,
            state,
            recheck_unsafe,
            source_snapshot_ok,
            dest_snapshot_ok,
            overwrite,
            overwrite_supported,
        ) is Copy),
        spec_predicted_action(state, overwrite_supported) matches ActionLabel::Skip {
            reason,
            remedy,
        } && !remedy && !spec_overwrite_acts_on(reason),
{
}

/// The file type one `lstat` reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LstatType {
    /// `FileType::is_symlink`.
    pub is_symlink: bool,
    /// `FileType::is_file`.
    pub is_file: bool,
    /// `FileType::is_dir`.
    pub is_dir: bool,
}

/// The `source_kind:` label of `info`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKindLabel {
    /// `lstat` failed.
    Missing,
    /// A symlink.
    Symlink,
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// Anything else.
    Other,
}

/// The planner's examination of the source from one `lstat` result
/// (`RealFs::source_kind`): a symlink first, then a regular file, else other;
/// a failed `lstat` cannot be examined.
pub open spec fn spec_source_exam(lstat: Option<LstatType>) -> SourceExam {
    match lstat {
        None => SourceExam::Unreadable,
        Some(t) => if t.is_symlink {
            SourceExam::Symlink
        } else if t.is_file {
            SourceExam::Regular
        } else {
            SourceExam::Other
        },
    }
}

/// The specification of [`source_kind`].
pub open spec fn spec_source_kind(lstat: Option<LstatType>) -> SourceKindLabel {
    match lstat {
        None => SourceKindLabel::Missing,
        Some(t) => if t.is_symlink {
            SourceKindLabel::Symlink
        } else if t.is_file {
            SourceKindLabel::File
        } else if t.is_dir {
            SourceKindLabel::Directory
        } else {
            SourceKindLabel::Other
        },
    }
}

/// The `source_kind:` label for one `lstat` result: a symlink first, then a
/// regular file, then a directory, else other; `missing` when `lstat` failed.
pub fn source_kind(lstat: Option<LstatType>) -> (r: SourceKindLabel)
    ensures
        r == spec_source_kind(lstat),
{
    match lstat {
        None => SourceKindLabel::Missing,
        Some(t) => if t.is_symlink {
            SourceKindLabel::Symlink
        } else if t.is_file {
            SourceKindLabel::File
        } else if t.is_dir {
            SourceKindLabel::Directory
        } else {
            SourceKindLabel::Other
        },
    }
}

impl SourceKindLabel {
    /// Whether the label is `file`: the only source `info` classifies a
    /// destination for.
    pub fn is_regular_file(self) -> (r: bool)
        ensures
            r == (self is File),
    {
        matches!(self, SourceKindLabel::File)
    }

    /// Whether the label is `directory`, which `info` passes to the
    /// worktreeinclude evaluation.
    pub fn is_directory(self) -> (r: bool)
        ensures
            r == (self is Directory),
    {
        matches!(self, SourceKindLabel::Directory)
    }

    /// Whether the source exists (`lstat` succeeded).
    pub fn exists(self) -> (r: bool)
        ensures
            r == !(self is Missing),
    {
        !matches!(self, SourceKindLabel::Missing)
    }
}

/// `info`'s source label agrees with the planner's examination of the same
/// `lstat` result.
pub proof fn theorem_source_kind_matches_planner(lstat: Option<LstatType>, label: SourceKindLabel)
    requires
        label == spec_source_kind(lstat),
    ensures
        label is File <==> spec_source_exam(lstat) is Regular,
        label is Missing <==> spec_source_exam(lstat) is Unreadable,
        (label is Symlink || label is Directory || label is Other) <==> (spec_source_exam(
            lstat,
        ) is Symlink || spec_source_exam(lstat) is Other),
{
}

/// What `list -v` observed about a source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListSource {
    /// Not a symlink and a regular file; the destination was classified.
    Regular(DestState),
    /// A symlink or not a regular file; `exists` is `Path::exists`.
    NonRegular {
        /// Whether the source path exists (following symlinks).
        exists: bool,
    },
}

/// The `action:` line `list -v` prints, if any.
pub open spec fn spec_list_action(source: ListSource, overwrite_supported: bool) -> Option<
    ActionLabel,
> {
    match source {
        ListSource::Regular(state) => Some(spec_predicted_action(state, overwrite_supported)),
        ListSource::NonRegular { exists } => if exists {
            Some(
                ActionLabel::Skip { reason: SkipKind::UnsupportedSourceType, remedy: false },
            )
        } else {
            None
        },
    }
}

/// The `action:` line `list -v` prints for a path, if any.
pub fn list_action(source: ListSource, overwrite_supported: bool) -> (r: Option<ActionLabel>)
    ensures
        r == spec_list_action(source, overwrite_supported),
{
    match source {
        ListSource::Regular(state) => Some(predicted_action(state, overwrite_supported)),
        ListSource::NonRegular { exists } => if exists {
            Some(ActionLabel::Skip { reason: SkipKind::UnsupportedSourceType, remedy: false })
        } else {
            None
        },
    }
}

/// For an existing source that is not a regular file, `list -v` predicts the
/// planner's skip for a symlink or other source.
pub proof fn theorem_list_unsupported_source_matches_planner(
    source: SourceExam,
    dest: DestState,
    recheck_unsafe: bool,
    source_snapshot_ok: bool,
    dest_snapshot_ok: bool,
    overwrite: bool,
    overwrite_supported: bool,
)
    requires
        source is Symlink || source is Other,
    ensures
        spec_list_action(ListSource::NonRegular { exists: true }, overwrite_supported) matches Some(
            label,
        ) && spec_label_outcome(label) == spec_plan_outcome(
            source,
            dest,
            recheck_unsafe,
            source_snapshot_ok,
            dest_snapshot_ok,
            overwrite,
            overwrite_supported,
        ),
{
}

/// What `info` knows about the destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoDestination {
    /// The source is a regular file and the destination was classified.
    Classified(DestState),
    /// The source is not a regular file; `present` is whether `lstat` of
    /// the destination succeeded.
    Unclassified {
        /// Whether the destination path exists (without following a final
        /// symlink).
        present: bool,
    },
}

/// The `destination:` label of `info`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DestLabel {
    /// `missing`.
    Missing,
    /// `up-to-date`.
    UpToDate,
    /// `permissions-differ`.
    PermissionsDiffer,
    /// `untracked-conflict`.
    UntrackedConflict,
    /// `tracked-conflict`.
    TrackedConflict,
    /// `type-conflict`.
    TypeConflict,
    /// `unsafe-path`.
    UnsafePath,
    /// `exists` (an unclassified destination that is present).
    Exists,
}

/// The `destination:` and `planned_action:` lines of `info`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InfoDestView {
    /// The destination label.
    pub destination: DestLabel,
    /// The planned action, printed only when present.
    pub planned_action: Option<ActionLabel>,
}

/// The destination label naming a classification.
pub open spec fn spec_dest_label(state: DestState) -> DestLabel {
    match state {
        DestState::Missing => DestLabel::Missing,
        DestState::UpToDate => DestLabel::UpToDate,
        DestState::PermissionsDiffer => DestLabel::PermissionsDiffer,
        DestState::UntrackedConflict => DestLabel::UntrackedConflict,
        DestState::TrackedConflict => DestLabel::TrackedConflict,
        DestState::TypeConflict => DestLabel::TypeConflict,
        DestState::UnsafePath => DestLabel::UnsafePath,
    }
}

/// The specification of [`info_destination`].
pub open spec fn spec_info_destination(
    dest: InfoDestination,
    eligible: bool,
    overwrite_supported: bool,
) -> InfoDestView {
    match dest {
        InfoDestination::Classified(state) => InfoDestView {
            destination: spec_dest_label(state),
            planned_action: if eligible {
                Some(spec_predicted_action(state, overwrite_supported))
            } else {
                None
            },
        },
        InfoDestination::Unclassified { present } => InfoDestView {
            destination: if present {
                DestLabel::Exists
            } else {
                DestLabel::Missing
            },
            planned_action: None,
        },
    }
}

/// The `destination:` and `planned_action:` lines `info` prints.
pub fn info_destination(dest: InfoDestination, eligible: bool, overwrite_supported: bool) -> (r:
    InfoDestView)
    ensures
        r == spec_info_destination(dest, eligible, overwrite_supported),
{
    match dest {
        InfoDestination::Classified(state) => {
            let destination = match state {
                DestState::Missing => DestLabel::Missing,
                DestState::UpToDate => DestLabel::UpToDate,
                DestState::PermissionsDiffer => DestLabel::PermissionsDiffer,
                DestState::UntrackedConflict => DestLabel::UntrackedConflict,
                DestState::TrackedConflict => DestLabel::TrackedConflict,
                DestState::TypeConflict => DestLabel::TypeConflict,
                DestState::UnsafePath => DestLabel::UnsafePath,
            };
            let planned_action = if eligible {
                Some(predicted_action(state, overwrite_supported))
            } else {
                None
            };
            InfoDestView { destination, planned_action }
        },
        InfoDestination::Unclassified { present } => InfoDestView {
            destination: if present {
                DestLabel::Exists
            } else {
                DestLabel::Missing
            },
            planned_action: None,
        },
    }
}

/// Properties of `info`'s destination lines.
pub proof fn theorem_info_view(dest: InfoDestination, eligible: bool, overwrite_supported: bool)
    ensures
        ({
            let v = spec_info_destination(dest, eligible, overwrite_supported);
            &&& v.planned_action is Some <==> (eligible && dest is Classified)
            &&& dest matches InfoDestination::Classified(state) ==> {
                &&& v.destination == spec_dest_label(state)
                &&& eligible ==> v.planned_action == spec_list_action(
                    ListSource::Regular(state),
                    overwrite_supported,
                )
            }
            &&& v.planned_action == Some(ActionLabel::Copy) ==> v.destination is Missing
            &&& v.destination is Exists ==> dest is Unclassified
            &&& !(v.planned_action matches Some(ActionLabel::Skip { reason, remedy: _ })
                && reason is UnsupportedSourceType)
        }),
        // The destination label is a one-to-one naming of the classification.
        forall|a: DestState, b: DestState| spec_dest_label(a) == spec_dest_label(b) ==> a == b,
{
}

} // verus!
