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
//!   `1 <= failed <= total <= entries`. The pair is computed by
//!   [`crate::execute::failure_summary`], the same verified function behind
//!   `executor::report_has_failures` and so behind a real run's exit status.
//! * `theorem_dry_run_agrees_with_executor` (and the last `ensures` of
//!   [`planning_failures`]): the counters handed to `failure_summary` are
//!   exactly the counters [`crate::execute::run_plan`] returns when it
//!   dry-runs the same plan, whatever the destination root and publication
//!   results (`theorem_run_plan_tally_is_executor_counts` proves that every
//!   tally satisfying `run_plan`'s postcondition satisfies
//!   `executor_counts`).
//!   So `planning_failures` returns exactly what the executor kernel's own
//!   exit decision returns for the executor kernel's own dry run of the plan.
//!   (A real run can fail more often, when a publication fails; see the next
//!   theorem.)
//! * `theorem_real_run_fails_if_dry_run_fails`: for *every* real run of the
//!   executor kernel over the same plan (every publication result, with or
//!   without a destination root), the run's total equals the dry run's total
//!   and its failure count is at least the dry run's. So for a fixed plan, a
//!   failing dry run implies a failing real run.
//! * [`shows_entry`]: `--quiet` hides every line except failures, and never
//!   hides a failure. [`dry_run_summary`]: the summary is printed exactly
//!   when not quiet, with the replace, repair and failed clauses exactly
//!   when their counters are non-zero.
//!
//! # Trust assumptions
//!
//! * Production maps each `PlannedEntry` (with its `DestinationExpectation`)
//!   to its [`EntryKind`] one-to-one (an exhaustive match, unit-tested).
//! * The executor theorems hold for the plan the executor kernel sees, which
//!   `executor.rs` builds from the same `PlannedEntry` values. That the two
//!   conversions agree (`executor_kind` of `planner::entry_kind` is the shape
//!   of `executor::kernel_entry`) is glue: two exhaustive matches, pinned by
//!   a unit test that dry-runs a plan of every kind through
//!   `executor::execute`.

#[allow(unused_imports)]
use vstd::prelude::*;

use crate::execute;

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
///
/// The pair is the executor kernel's own exit decision
/// ([`execute::failure_summary`]) applied to the executor kernel's own
/// dry-run counters for the same plan: the last `ensures` holds for every
/// tally [`execute::run_plan`] can return from a dry run of this plan.
pub fn planning_failures(kinds: &[EntryKind]) -> (r: Option<(usize, usize)>)
    ensures
        r is None <==> planning_failures_spec(kinds@) is None,
        r matches Some((f, t)) ==> planning_failures_spec(kinds@) == Some((f as nat, t as nat)),
        r matches Some((f, t)) ==> 1 <= f <= t <= kinds@.len(),
        forall|et: execute::Tally, has_root: bool, pubs: Seq<execute::Counter>|
            #[trigger] executor_counts(et, kinds@, true, has_root, pubs) ==> {
                &&& (r is Some <==> et.failed > 0)
                &&& r matches Some(p) ==> p.0 == et.failed && p.1 == et.copied + et.replaced
                    + et.permissions_repaired + et.failed
            },
{
    let t = tally(kinds);
    // The counters sum to the number of entries, which fits in usize.
    let _n = kinds.len();
    let dry_run = execute::Tally {
        copied: t.copies,
        replaced: t.replacements,
        permissions_repaired: t.repairs,
        failed: t.failures,
        skipped: t.skips,
        up_to_date: t.noops,
    };
    let r = execute::failure_summary(&dry_run);
    proof {
        assert forall|et: execute::Tally, has_root: bool, pubs: Seq<execute::Counter>|
            #[trigger] executor_counts(et, kinds@, true, has_root, pubs) implies {
                &&& (r is Some <==> et.failed > 0)
                &&& r matches Some(p) ==> p.0 == et.failed && p.1 == et.copied + et.replaced
                    + et.permissions_repaired + et.failed
            } by {
            theorem_dry_run_agrees_with_executor(kinds@, has_root, pubs, et);
        }
    }
    r
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
// Agreement with the executor kernel (`crate::execute`).
// ---------------------------------------------------------------------------

/// The executor kernel's shape of an entry kind: the `execute::EntryKind`
/// that `executor.rs` builds from the same `PlannedEntry`.
pub open spec fn executor_kind(k: EntryKind) -> execute::EntryKind {
    match k {
        EntryKind::CopyMissing => execute::EntryKind::Copy(execute::DestExpectation::Missing),
        EntryKind::CopyReplace => execute::EntryKind::Copy(
            execute::DestExpectation::ReplaceExisting,
        ),
        EntryKind::CopyRepair => execute::EntryKind::Copy(
            execute::DestExpectation::RepairPermissions,
        ),
        EntryKind::NoOp => execute::EntryKind::NoOp,
        EntryKind::Skip => execute::EntryKind::Skip,
        EntryKind::Failure => execute::EntryKind::Failure,
    }
}

/// The class the executor kernel gives each entry of the plan, where
/// `pubs[i]` is the class of entry `i`'s publication result
/// ([`execute::entry_counter`] entry by entry; see
/// `theorem_run_plan_tally_is_executor_counts` for the link to
/// [`execute::entry_classes`]).
pub open spec fn executor_classes(
    kinds: Seq<EntryKind>,
    dry_run: bool,
    has_root: bool,
    pubs: Seq<execute::Counter>,
) -> Seq<execute::Counter> {
    Seq::new(
        kinds.len(),
        |i: int| execute::entry_counter(executor_kind(kinds[i]), dry_run, has_root, pubs[i]),
    )
}

/// The classes a publication can end in (the range of
/// [`execute::publish_result_counter`]).
pub open spec fn is_publish_counter(c: execute::Counter) -> bool {
    c == execute::Counter::Copied || c == execute::Counter::Replaced || c
        == execute::Counter::PermissionsRepaired || c == execute::Counter::Failed
}

/// The class of each publication result, in plan order.
pub open spec fn result_classes<E>(results: Seq<Result<execute::PublishKind, E>>) -> Seq<
    execute::Counter,
> {
    Seq::new(results.len(), |i: int| execute::publish_result_counter(results[i]))
}

/// `executor_counts` is exactly what [`execute::run_plan`]'s postcondition
/// says about its tally. For any plan entries whose shapes are this plan's
/// kinds, and any publication results, a tally whose counters count
/// [`execute::entry_classes`] (as `run_plan` ensures for the results it
/// witnesses) satisfies `executor_counts` with `pubs` the classes of those
/// results. So every statement below about `executor_counts` is a statement
/// about every tally `run_plan` can return.
pub proof fn theorem_run_plan_tally_is_executor_counts<C, S, F, E>(
    entries: Seq<execute::PlanEntry<C, S, F>>,
    kinds: Seq<EntryKind>,
    dry_run: bool,
    has_root: bool,
    results: Seq<Result<execute::PublishKind, E>>,
    et: execute::Tally,
)
    requires
        entries.len() == kinds.len(),
        results.len() == entries.len(),
        forall|i: int| 0 <= i < entries.len() ==> #[trigger] entries[i].kind() == executor_kind(kinds[i]),
        forall|c: execute::Counter|
            #[trigger] et.get(c) == execute::count(
                execute::entry_classes(entries, dry_run, has_root, results),
                c,
            ),
    ensures
        executor_counts(et, kinds, dry_run, has_root, result_classes(results)),
{
    let pubs = result_classes(results);
    assert(execute::entry_classes(entries, dry_run, has_root, results) =~= executor_classes(
        kinds,
        dry_run,
        has_root,
        pubs,
    )) by {
        assert forall|i: int| 0 <= i < entries.len() implies execute::entry_classes(
            entries,
            dry_run,
            has_root,
            results,
        )[i] == #[trigger] executor_classes(kinds, dry_run, has_root, pubs)[i] by {
            assert(entries[i].kind() == executor_kind(kinds[i]));
        }
    }
    assert forall|i: int| 0 <= i < pubs.len() implies is_publish_counter(#[trigger] pubs[i]) by {
        match results[i] {
            Ok(p) => {
                match p {
                    execute::PublishKind::Created => {},
                    execute::PublishKind::Replaced => {},
                    execute::PublishKind::PermissionsRepaired => {},
                }
            },
            Err(_) => {},
        }
    }
}

/// `et` is a tally [`execute::run_plan`] can return for this plan: its
/// postcondition about the counters, for a run that is or is not a dry run
/// (`dry_run`), with or without a destination root (`has_root`), where
/// `pubs[i]` is the class of entry `i`'s publication result.
pub open spec fn executor_counts(
    et: execute::Tally,
    kinds: Seq<EntryKind>,
    dry_run: bool,
    has_root: bool,
    pubs: Seq<execute::Counter>,
) -> bool {
    &&& pubs.len() == kinds.len()
    &&& forall|i: int| 0 <= i < pubs.len() ==> is_publish_counter(#[trigger] pubs[i])
    &&& forall|c: execute::Counter|
        #[trigger] et.get(c) == execute::count(executor_classes(kinds, dry_run, has_root, pubs), c)
}

/// A count never exceeds the number of entries.
proof fn lemma_count_le_len(kinds: Seq<EntryKind>, k: EntryKind)
    ensures
        count(kinds, k) <= kinds.len(),
    decreases kinds.len(),
{
    if kinds.len() > 0 {
        lemma_count_le_len(kinds.drop_last(), k);
    }
}

/// `executor_counts` is satisfiable for every plan that fits in memory, so
/// the agreement `ensures` of [`planning_failures`] is not vacuous.
pub proof fn lemma_dry_run_tally_exists(kinds: Seq<EntryKind>, has_root: bool)
    requires
        kinds.len() <= usize::MAX,
    ensures
        exists|et: execute::Tally, pubs: Seq<execute::Counter>|
            executor_counts(et, kinds, true, has_root, pubs),
{
    let pubs = Seq::new(kinds.len(), |i: int| execute::Counter::Copied);
    lemma_count_le_len(kinds, EntryKind::CopyMissing);
    lemma_count_le_len(kinds, EntryKind::CopyReplace);
    lemma_count_le_len(kinds, EntryKind::CopyRepair);
    lemma_count_le_len(kinds, EntryKind::Failure);
    lemma_count_le_len(kinds, EntryKind::Skip);
    lemma_count_le_len(kinds, EntryKind::NoOp);
    let et = execute::Tally {
        copied: count(kinds, EntryKind::CopyMissing) as usize,
        replaced: count(kinds, EntryKind::CopyReplace) as usize,
        permissions_repaired: count(kinds, EntryKind::CopyRepair) as usize,
        failed: count(kinds, EntryKind::Failure) as usize,
        skipped: count(kinds, EntryKind::Skip) as usize,
        up_to_date: count(kinds, EntryKind::NoOp) as usize,
    };
    lemma_dry_run_classes(kinds, has_root, pubs);
    assert(executor_counts(et, kinds, true, has_root, pubs));
}

/// The executor kernel classifies a plan entry by entry.
proof fn lemma_executor_classes_drop_last(
    kinds: Seq<EntryKind>,
    dry_run: bool,
    has_root: bool,
    pubs: Seq<execute::Counter>,
)
    requires
        kinds.len() > 0,
        pubs.len() == kinds.len(),
    ensures
        executor_classes(kinds, dry_run, has_root, pubs).len() == kinds.len(),
        executor_classes(kinds, dry_run, has_root, pubs).drop_last() == executor_classes(
            kinds.drop_last(),
            dry_run,
            has_root,
            pubs.drop_last(),
        ),
        executor_classes(kinds, dry_run, has_root, pubs).last() == execute::entry_counter(
            executor_kind(kinds.last()),
            dry_run,
            has_root,
            pubs.last(),
        ),
{
    let classes = executor_classes(kinds, dry_run, has_root, pubs);
    let shorter = executor_classes(kinds.drop_last(), dry_run, has_root, pubs.drop_last());
    assert(classes.drop_last() =~= shorter);
}

/// The executor kernel's dry-run classes count each kind of entry in its own
/// counter: every copy by its destination expectation, every other entry by
/// its kind.
proof fn lemma_dry_run_classes(kinds: Seq<EntryKind>, has_root: bool, pubs: Seq<execute::Counter>)
    requires
        pubs.len() == kinds.len(),
    ensures
        ({
            let classes = executor_classes(kinds, true, has_root, pubs);
            &&& execute::count(classes, execute::Counter::Copied) == count(
                kinds,
                EntryKind::CopyMissing,
            )
            &&& execute::count(classes, execute::Counter::Replaced) == count(
                kinds,
                EntryKind::CopyReplace,
            )
            &&& execute::count(classes, execute::Counter::PermissionsRepaired) == count(
                kinds,
                EntryKind::CopyRepair,
            )
            &&& execute::count(classes, execute::Counter::Failed) == count(
                kinds,
                EntryKind::Failure,
            )
            &&& execute::count(classes, execute::Counter::Skipped) == count(kinds, EntryKind::Skip)
            &&& execute::count(classes, execute::Counter::UpToDate) == count(kinds, EntryKind::NoOp)
        }),
    decreases kinds.len(),
{
    if kinds.len() > 0 {
        lemma_dry_run_classes(kinds.drop_last(), has_root, pubs.drop_last());
        lemma_executor_classes_drop_last(kinds, true, has_root, pubs);
    }
}

/// In a real run, each copy lands in exactly one of the copied, replaced,
/// repaired and failed counters (whatever its publication returned, and a
/// failure without a destination root), and each planning failure is
/// failed.
proof fn lemma_real_run_classes(kinds: Seq<EntryKind>, has_root: bool, pubs: Seq<execute::Counter>)
    requires
        pubs.len() == kinds.len(),
        forall|i: int| 0 <= i < pubs.len() ==> is_publish_counter(#[trigger] pubs[i]),
    ensures
        ({
            let classes = executor_classes(kinds, false, has_root, pubs);
            &&& execute::count(classes, execute::Counter::Copied) + execute::count(
                classes,
                execute::Counter::Replaced,
            ) + execute::count(classes, execute::Counter::PermissionsRepaired) + execute::count(
                classes,
                execute::Counter::Failed,
            ) == copy_count(kinds) + count(kinds, EntryKind::Failure)
            &&& execute::count(classes, execute::Counter::Failed) >= count(
                kinds,
                EntryKind::Failure,
            )
            &&& execute::count(classes, execute::Counter::Skipped) == count(kinds, EntryKind::Skip)
            &&& execute::count(classes, execute::Counter::UpToDate) == count(kinds, EntryKind::NoOp)
        }),
    decreases kinds.len(),
{
    if kinds.len() > 0 {
        let shorter = pubs.drop_last();
        assert forall|i: int| 0 <= i < shorter.len() implies is_publish_counter(
            #[trigger] shorter[i],
        ) by {
            assert(shorter[i] == pubs[i]);
        }
        lemma_real_run_classes(kinds.drop_last(), has_root, shorter);
        lemma_executor_classes_drop_last(kinds, false, has_root, pubs);
        assert(is_publish_counter(pubs[pubs.len() - 1]));
    }
}

/// The executor kernel's dry run of a plan counts exactly what [`tally`]
/// counts, and its exit decision ([`execute::failure_summary`]'s
/// postcondition on those counters) is `planning_failures_spec`: `Some`
/// exactly when something failed, then `(failed, copied + replaced +
/// permissions_repaired + failed)`.
pub proof fn theorem_dry_run_agrees_with_executor(
    kinds: Seq<EntryKind>,
    has_root: bool,
    pubs: Seq<execute::Counter>,
    et: execute::Tally,
)
    requires
        executor_counts(et, kinds, true, has_root, pubs),
    ensures
        et.copied == count(kinds, EntryKind::CopyMissing),
        et.replaced == count(kinds, EntryKind::CopyReplace),
        et.permissions_repaired == count(kinds, EntryKind::CopyRepair),
        et.failed == count(kinds, EntryKind::Failure),
        et.skipped == count(kinds, EntryKind::Skip),
        et.up_to_date == count(kinds, EntryKind::NoOp),
        planning_failures_spec(kinds) is Some <==> et.failed > 0,
        planning_failures_spec(kinds) matches Some(p) ==> p.0 == et.failed && p.1 == et.copied
            + et.replaced + et.permissions_repaired + et.failed,
{
    lemma_dry_run_classes(kinds, has_root, pubs);
    assert(et.get(execute::Counter::Copied) == et.copied);
    assert(et.get(execute::Counter::Replaced) == et.replaced);
    assert(et.get(execute::Counter::PermissionsRepaired) == et.permissions_repaired);
    assert(et.get(execute::Counter::Failed) == et.failed);
    assert(et.get(execute::Counter::Skipped) == et.skipped);
    assert(et.get(execute::Counter::UpToDate) == et.up_to_date);
}

/// Every real run of the executor kernel over a plan counts the plan's
/// copies and planning failures, and nothing else, as succeeded or failed,
/// and fails at least once per planning failure.
pub proof fn lemma_real_run_accounting(
    kinds: Seq<EntryKind>,
    has_root: bool,
    pubs: Seq<execute::Counter>,
    et: execute::Tally,
)
    requires
        executor_counts(et, kinds, false, has_root, pubs),
    ensures
        et.copied + et.replaced + et.permissions_repaired + et.failed == copy_count(kinds) + count(
            kinds,
            EntryKind::Failure,
        ),
        et.failed >= count(kinds, EntryKind::Failure),
        et.skipped == count(kinds, EntryKind::Skip),
        et.up_to_date == count(kinds, EntryKind::NoOp),
{
    lemma_real_run_classes(kinds, has_root, pubs);
    assert(et.get(execute::Counter::Copied) == et.copied);
    assert(et.get(execute::Counter::Replaced) == et.replaced);
    assert(et.get(execute::Counter::PermissionsRepaired) == et.permissions_repaired);
    assert(et.get(execute::Counter::Failed) == et.failed);
    assert(et.get(execute::Counter::Skipped) == et.skipped);
    assert(et.get(execute::Counter::UpToDate) == et.up_to_date);
}

/// For a fixed plan, a failing dry run implies that every real run of the
/// executor kernel fails (`et.failed > 0`, so `execute::failure_summary`
/// returns `Some`), with the same total and at least as many failures,
/// whatever each publication returned and whether or not there is a
/// destination root.
pub proof fn theorem_real_run_fails_if_dry_run_fails(
    kinds: Seq<EntryKind>,
    has_root: bool,
    pubs: Seq<execute::Counter>,
    et: execute::Tally,
)
    requires
        executor_counts(et, kinds, false, has_root, pubs),
        planning_failures_spec(kinds) is Some,
    ensures
        et.failed > 0,
        et.failed >= planning_failures_spec(kinds)->Some_0.0,
        et.copied + et.replaced + et.permissions_repaired + et.failed == planning_failures_spec(
            kinds,
        )->Some_0.1,
{
    lemma_real_run_accounting(kinds, has_root, pubs, et);
}

} // verus!
