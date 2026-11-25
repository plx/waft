//! Verified kernel: plan accounting, the dry-run summary, and the dry-run
//! exit status.
//!
//! # What it decides
//!
//! * [`tally`] counts a plan's entries by kind for the `--dry-run` summary
//!   (`planner::render_dry_run`).
//! * [`planning_failures`] decides whether a dry run exits non-zero and with
//!   what `(failed, total)` (`planner::planning_failures`).
//! * [`shows_entry`] and [`dry_run_summary`] decide which dry-run lines are
//!   printed under `--quiet` and which optional summary clauses appear.
//!
//! # Theorems
//!
//! * [`tally`]: every counter is the number of entries of its kind
//!   (`count`); the six counters sum to the number of entries, so each
//!   entry is counted exactly once and no counter can overflow.
//! * [`planning_failures`]: `None` exactly when no entry is a failure;
//!   otherwise `(failed, total)` with `failed` the number of failures,
//!   `total` the failures plus the copies of every kind, and
//!   `1 <= failed <= total <= entries`.
//! * `theorem_dry_run_agrees_with_executor`: the pair equals what
//!   `executor::report_has_failures` computes from the counters the
//!   executor's dry-run branch produces for the same plan (modelled by
//!   `dry_run_report`). This is the invariant `planning_failures` documents:
//!   a dry run reports and exits exactly like the run it describes.
//! * `theorem_real_run_fails_if_dry_run_fails`: for *every* assignment of
//!   execution outcomes to the plan's copies, the real run's total equals
//!   the dry run's total and its failure count is at least the dry run's. So
//!   for a fixed plan, a failing dry run implies a failing real run.
//! * [`shows_entry`]: `--quiet` hides every line except failures, and never
//!   hides a failure. [`dry_run_summary`]: the summary is printed exactly
//!   when not quiet, with the replace, repair and failed clauses exactly
//!   when their counters are non-zero.
//!
//! # Trust assumptions
//!
//! * Production maps each `PlannedEntry` (with its `DestinationExpectation`)
//!   to its [`EntryKind`] one-to-one (an exhaustive match, unit-tested).
//! * The executor model (`dry_run_report`, `real_run_failed`,
//!   `real_run_succeeded`, `report_failure_summary`) describes
//!   `executor::execute` and `executor::report_has_failures`, which another
//!   kernel verifies; this module only relies on the counting they document.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// The shape of a planned entry (mirrors `model::PlannedEntry` and, for a
/// copy, its `DestinationExpectation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// A copy to a missing destination.
    CopyMissing,
    /// A copy replacing an existing destination.
    CopyReplace,
    /// A permission repair of an existing destination.
    CopyRepair,
    /// Already up to date.
    NoOp,
    /// Skipped.
    Skip,
    /// A per-file planning failure.
    Failure,
}

/// Per-kind entry counts of a plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tally {
    /// Copies to a missing destination.
    pub copies: usize,
    /// Replacements of an existing destination.
    pub replacements: usize,
    /// Permission repairs.
    pub repairs: usize,
    /// Skips.
    pub skips: usize,
    /// Up-to-date entries.
    pub noops: usize,
    /// Planning failures.
    pub failures: usize,
}

/// Which optional parts of the dry-run summary line appear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DryRunSummary {
    /// The ", N to replace" clause.
    pub replacements: bool,
    /// The ", N permission repair(s)" clause.
    pub repairs: bool,
    /// The ", N failed" clause.
    pub failures: bool,
}

/// The number of entries of kind `k`.
pub open spec fn count(kinds: Seq<EntryKind>, k: EntryKind) -> nat
    decreases kinds.len(),
{
    if kinds.len() == 0 {
        0
    } else {
        count(kinds.drop_last(), k) + if kinds.last() == k {
            1nat
        } else {
            0nat
        }
    }
}

/// The number of copies of every kind.
pub open spec fn copy_count(kinds: Seq<EntryKind>) -> nat {
    count(kinds, EntryKind::CopyMissing) + count(kinds, EntryKind::CopyReplace) + count(
        kinds,
        EntryKind::CopyRepair,
    )
}

impl Tally {
    /// Every counter is the number of entries of its kind in `kinds`.
    pub open spec fn counts(&self, kinds: Seq<EntryKind>) -> bool {
        &&& self.copies == count(kinds, EntryKind::CopyMissing)
        &&& self.replacements == count(kinds, EntryKind::CopyReplace)
        &&& self.repairs == count(kinds, EntryKind::CopyRepair)
        &&& self.skips == count(kinds, EntryKind::Skip)
        &&& self.noops == count(kinds, EntryKind::NoOp)
        &&& self.failures == count(kinds, EntryKind::Failure)
    }

    /// The sum of all counters.
    pub open spec fn total(&self) -> nat {
        self.copies as nat + self.replacements as nat + self.repairs as nat + self.skips as nat
            + self.noops as nat + self.failures as nat
    }
}

/// `(failed, total)` for a plan, or `None` when nothing failed.
pub open spec fn planning_failures_spec(kinds: Seq<EntryKind>) -> Option<(nat, nat)> {
    let failed = count(kinds, EntryKind::Failure);
    if failed == 0 {
        None
    } else {
        Some((failed, failed + copy_count(kinds)))
    }
}

/// Count a plan's entries by kind.
pub fn tally(kinds: &[EntryKind]) -> (t: Tally)
    ensures
        t.counts(kinds@),
        t.total() == kinds@.len(),
{
    let mut t = Tally { copies: 0, replacements: 0, repairs: 0, skips: 0, noops: 0, failures: 0 };
    let mut i: usize = 0;
    while i < kinds.len()
        invariant
            i <= kinds@.len(),
            t.counts(kinds@.subrange(0, i as int)),
            t.total() == i,
        decreases kinds.len() - i,
    {
        assert(kinds@.subrange(0, i + 1).drop_last() == kinds@.subrange(0, i as int));
        match kinds[i] {
            EntryKind::CopyMissing => t.copies += 1,
            EntryKind::CopyReplace => t.replacements += 1,
            EntryKind::CopyRepair => t.repairs += 1,
            EntryKind::NoOp => t.noops += 1,
            EntryKind::Skip => t.skips += 1,
            EntryKind::Failure => t.failures += 1,
        }
        i += 1;
    }
    assert(kinds@.subrange(0, kinds@.len() as int) == kinds@);
    t
}

/// Whether a dry run of these entries fails, as `(failed, total)`.
pub fn planning_failures(kinds: &[EntryKind]) -> (r: Option<(usize, usize)>)
    ensures
        r is None <==> planning_failures_spec(kinds@) is None,
        r matches Some((f, t)) ==> planning_failures_spec(kinds@) == Some((f as nat, t as nat)),
        r matches Some((f, t)) ==> 1 <= f <= t <= kinds@.len(),
{
    let t = tally(kinds);
    // The counters sum to the number of entries, which fits in usize.
    let _n = kinds.len();
    if t.failures == 0 {
        None
    } else {
        Some((t.failures, t.failures + t.copies + t.replacements + t.repairs))
    }
}

/// Whether a dry run prints the line for an entry of kind `kind`.
pub fn shows_entry(kind: EntryKind, quiet: bool) -> (r: bool)
    ensures
        r == (!quiet || kind == EntryKind::Failure),
{
    !quiet || matches!(kind, EntryKind::Failure)
}

/// Which dry-run summary clauses appear, or `None` when the summary is not
/// printed at all (under `--quiet`).
pub fn dry_run_summary(t: &Tally, quiet: bool) -> (r: Option<DryRunSummary>)
    ensures
        r is None <==> quiet,
        r matches Some(s) ==> s.replacements == (t.replacements > 0) && s.repairs == (t.repairs > 0)
            && s.failures == (t.failures > 0),
{
    if quiet {
        return None;
    }
    Some(
        DryRunSummary {
            replacements: t.replacements > 0,
            repairs: t.repairs > 0,
            failures: t.failures > 0,
        },
    )
}

// ---------------------------------------------------------------------------
// The executor's counting, as documented by `executor::execute`.
// ---------------------------------------------------------------------------

/// What executing one planned copy did in a real run.
pub enum RunOutcome {
    /// Published to a missing destination.
    Created,
    /// Replaced an existing destination.
    Replaced,
    /// Repaired an existing destination's permissions.
    Repaired,
    /// Failed.
    Failed,
}

/// The executor's dry-run counters for a plan, in the order
/// `(copied, replaced, permissions_repaired, failed, skipped, up_to_date)`:
/// each copy is counted by its expectation, each other entry by its kind.
pub open spec fn dry_run_report(kinds: Seq<EntryKind>) -> (nat, nat, nat, nat, nat, nat) {
    (
        count(kinds, EntryKind::CopyMissing),
        count(kinds, EntryKind::CopyReplace),
        count(kinds, EntryKind::CopyRepair),
        count(kinds, EntryKind::Failure),
        count(kinds, EntryKind::Skip),
        count(kinds, EntryKind::NoOp),
    )
}

/// `executor::report_has_failures`: `(failed, succeeded + failed)` when
/// anything failed, where `succeeded = copied + replaced +
/// permissions_repaired` (`CopyReport::succeeded`).
pub open spec fn report_failure_summary(
    copied: nat,
    replaced: nat,
    repaired: nat,
    failed: nat,
) -> Option<(nat, nat)> {
    if failed > 0 {
        Some((failed, copied + replaced + repaired + failed))
    } else {
        None
    }
}

/// Whether an entry kind is a planned copy of any kind.
pub open spec fn is_copy(k: EntryKind) -> bool {
    k == EntryKind::CopyMissing || k == EntryKind::CopyReplace || k == EntryKind::CopyRepair
}

/// Real-run failures: every planning failure, plus every copy whose
/// execution failed. `outcomes[i]` is the outcome of entry `i` when it is a
/// copy and is ignored otherwise.
pub open spec fn real_run_failed(kinds: Seq<EntryKind>, outcomes: Seq<RunOutcome>) -> nat
    decreases kinds.len(),
{
    if kinds.len() == 0 || outcomes.len() != kinds.len() {
        0
    } else {
        real_run_failed(kinds.drop_last(), outcomes.drop_last()) + if kinds.last() == EntryKind::Failure
            || (is_copy(kinds.last()) && outcomes.last() is Failed) {
            1nat
        } else {
            0nat
        }
    }
}

/// Real-run successes (`CopyReport::succeeded`): every copy whose execution
/// published or repaired.
pub open spec fn real_run_succeeded(kinds: Seq<EntryKind>, outcomes: Seq<RunOutcome>) -> nat
    decreases kinds.len(),
{
    if kinds.len() == 0 || outcomes.len() != kinds.len() {
        0
    } else {
        real_run_succeeded(kinds.drop_last(), outcomes.drop_last()) + if is_copy(kinds.last())
            && !(outcomes.last() is Failed) {
            1nat
        } else {
            0nat
        }
    }
}

/// The dry run's failure decision is the executor's, applied to the
/// executor's dry-run counters for the same plan.
pub proof fn theorem_dry_run_agrees_with_executor(kinds: Seq<EntryKind>)
    ensures
        planning_failures_spec(kinds) == report_failure_summary(
            dry_run_report(kinds).0,
            dry_run_report(kinds).1,
            dry_run_report(kinds).2,
            dry_run_report(kinds).3,
        ),
{
}

/// Successes plus failures of a real run are the plan's copies plus its
/// planning failures, and a real run fails at least as often as the plan
/// does, whatever each copy's execution outcome.
pub proof fn lemma_real_run_accounting(kinds: Seq<EntryKind>, outcomes: Seq<RunOutcome>)
    requires
        outcomes.len() == kinds.len(),
    ensures
        real_run_succeeded(kinds, outcomes) + real_run_failed(kinds, outcomes) == copy_count(kinds)
            + count(kinds, EntryKind::Failure),
        real_run_failed(kinds, outcomes) >= count(kinds, EntryKind::Failure),
    decreases kinds.len(),
{
    if kinds.len() > 0 {
        lemma_real_run_accounting(kinds.drop_last(), outcomes.drop_last());
    }
}

/// For a fixed plan, a failing dry run implies a failing real run with the
/// same total and at least as many failures, for every assignment of
/// execution outcomes to the plan's copies.
pub proof fn theorem_real_run_fails_if_dry_run_fails(
    kinds: Seq<EntryKind>,
    outcomes: Seq<RunOutcome>,
)
    requires
        outcomes.len() == kinds.len(),
        planning_failures_spec(kinds) is Some,
    ensures
        report_failure_summary(
            real_run_succeeded(kinds, outcomes),
            0,
            0,
            real_run_failed(kinds, outcomes),
        ) matches Some((f, t)) && f >= planning_failures_spec(kinds)->Some_0.0 && t
            == planning_failures_spec(kinds)->Some_0.1,
{
    lemma_real_run_accounting(kinds, outcomes);
}

} // verus!
