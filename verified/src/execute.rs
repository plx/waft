//! Verified kernel: how `executor::execute` walks a copy plan, what it
//! counts, what it reports, and what the exit status says about it.
//!
//! # What it decides
//!
//! [`run_plan`] is the per-entry loop of `executor::execute`. It visits the
//! plan entries in order, exactly once each, and for every entry decides:
//!
//! * whether the entry is published at all (`spec_copy_route`: a dry run never
//!   publishes, a plan without a destination root never publishes, and only
//!   `Copy` entries are ever published);
//! * which of the six report counters it lands in ([`Counter`]);
//! * whether it produces a result row (every entry except an up-to-date one).
//!
//! The effects — taking the index lock, rechecking trackedness, copying, and
//! building the `CopyResult` rows — stay in production behind the
//! [`EntryRunner`] trait, so the order and number of filesystem and Git
//! queries are exactly the ones the kernel asks for.
//!
//! [`failure_summary`] decides the `(failed, total)` pair behind waft's exit
//! status, and [`prints_row`] / [`summary_plan`] decide which report lines
//! `render_report` prints under `--quiet`.
//!
//! # Theorems
//!
//! About the shipped exec code (each is an `ensures` of the function or a
//! named lemma over the specification that `ensures` refers to):
//!
//! * [`run_plan`] — the runner's event log grows by exactly
//!   `plan_log``(kinds, dry_run, has_root, pubs)` for the publish results
//!   `pubs` the runner returned, and every counter equals the number of
//!   entries whose class (`entry_counter`) is that counter.
//! * `theorem_partition` — the six counters sum to the number of plan
//!   entries: each entry is counted exactly once. With
//!   `lemma_count_le_len`, no counter can overflow.
//! * `theorem_reports_match_counters` — for every counter except
//!   `UpToDate`, the number of rows reported with that class equals the
//!   counter; up-to-date entries produce no row; so the number of `FAILED`
//!   rows equals `failed`, and the rows and the summary cannot disagree.
//! * `theorem_no_publication_without_route` — a dry run, or a plan without
//!   a destination root, issues no publication at all; otherwise exactly the
//!   `Copy` entries are published.
//! * `theorem_publication_is_reported` — each publication is immediately
//!   followed by the row for the same entry, with the class the publication
//!   result maps to (a publish failure is always reported as `Failed`).
//! * `theorem_dry_run_agrees_with_publication` — the class a dry run
//!   predicts for an expectation is the class a successful publication of the
//!   matching kind reports.
//! * [`failure_summary`] — `Some` exactly when `failed > 0`; then the pair is
//!   `(failed, copied + replaced + permissions_repaired + failed)` and
//!   `1 <= failed <= total`. `theorem_failure_status_agrees_with_rows`:
//!   for a tally produced by [`run_plan`], `failed > 0` exactly when some
//!   reported row is a failure.
//! * [`prints_row`] — `--quiet` never hides a failure and hides every other
//!   row; [`summary_plan`] prints the summary exactly when not quiet, with the
//!   replaced/repaired clauses exactly when those counters are non-zero.
//!
//! # Trust assumptions
//!
//! * The production [`EntryRunner`] does what its method names say: `publish`
//!   performs one publication of the given copy entry and returns its result
//!   without emitting a row; each `report_*` call pushes exactly one result
//!   row describing the given entry with the given class. The runner's
//!   `EntryRunner::log` is a ghost history of those calls; its `ensures` say
//!   only that a call happened and returned what it returned.
//! * The glue converting `PlannedEntry`, `DestinationExpectation`, and
//!   `PublishOutcome` into [`PlanEntry`], [`DestExpectation`], and
//!   [`PublishKind`] is a faithful one-to-one mapping (exhaustive matches,
//!   pinned by unit tests in `executor.rs`).
//! * `failure_summary` is only given counters whose sum fits in `usize`. Every
//!   `CopyReport` waft builds comes from `execute`, whose counters come from
//!   [`run_plan`] and sum to the plan length (see `theorem_partition`).

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

// ---------------------------------------------------------------------------
// Plain data mirrored from `src/model.rs`.
// ---------------------------------------------------------------------------

/// What planning recorded at the destination of a copy entry
/// (mirrors `model::DestinationExpectation` without its snapshot).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DestExpectation {
    /// The destination did not exist.
    Missing,
    /// An existing untracked destination is replaced.
    ReplaceExisting,
    /// An existing untracked destination has its permissions repaired.
    RepairPermissions,
}

/// What a successful publication did (mirrors `model::PublishOutcome`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishKind {
    /// A missing destination was created.
    Created,
    /// An existing destination was replaced.
    Replaced,
    /// An existing destination's permissions were repaired.
    PermissionsRepaired,
}

/// The report counter an entry lands in (one per `CopyReport` counter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Counter {
    /// `CopyReport::copied`.
    Copied,
    /// `CopyReport::replaced`.
    Replaced,
    /// `CopyReport::permissions_repaired`.
    PermissionsRepaired,
    /// `CopyReport::failed`.
    Failed,
    /// `CopyReport::skipped`.
    Skipped,
    /// `CopyReport::up_to_date`.
    UpToDate,
}

/// One plan entry, carrying production payloads the kernel never inspects
/// (mirrors `model::PlannedEntry`).
pub enum PlanEntry<C, S, F> {
    /// A file to copy, with what planning expected at its destination.
    Copy(C, DestExpectation),
    /// A file that is already up to date.
    NoOp,
    /// A file left alone, with the planner's reason.
    Skip(S),
    /// A file planning could not describe.
    Failure(F),
}

/// The payload-free shape of a [`PlanEntry`], used by the specifications.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// A copy entry with its destination expectation.
    Copy(DestExpectation),
    /// An up-to-date entry.
    NoOp,
    /// A skipped entry.
    Skip,
    /// A planning failure.
    Failure,
}

impl<C, S, F> PlanEntry<C, S, F> {
    /// The entry's shape.
    pub open spec fn kind(&self) -> EntryKind {
        match self {
            PlanEntry::Copy(_, e) => EntryKind::Copy(*e),
            PlanEntry::NoOp => EntryKind::NoOp,
            PlanEntry::Skip(_) => EntryKind::Skip,
            PlanEntry::Failure(_) => EntryKind::Failure,
        }
    }
}

/// The shapes of a sequence of plan entries.
pub open spec fn kinds<C, S, F>(entries: Seq<PlanEntry<C, S, F>>) -> Seq<EntryKind> {
    entries.map_values(|e: PlanEntry<C, S, F>| e.kind())
}

/// The result row of a copy entry.
pub enum CopyRow<E> {
    /// Published (or, in a dry run, would publish) a missing destination.
    Copied,
    /// Replaced (or would replace) an existing destination.
    Replaced,
    /// Repaired (or would repair) an existing destination's permissions.
    PermissionsRepaired,
    /// The plan has no destination root, so nothing could be published.
    MissingDestinationRoot,
    /// Publication failed with this error.
    PublishFailed(E),
}

/// How a copy entry is handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyRoute {
    /// Dry run: report what would happen; publish nothing.
    Simulate,
    /// Real run without a destination root: fail the entry; publish nothing.
    MissingDestinationRoot,
    /// Real run: publish.
    Publish,
}

/// One effect the kernel asks the runner for, as recorded in its ghost log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A publication was attempted; its result maps to this counter.
    Publish(Counter),
    /// A result row with this class was reported.
    Report(Counter),
}

// ---------------------------------------------------------------------------
// Specification.
// ---------------------------------------------------------------------------

/// Copy-entry precedence: a dry run simulates before anything else is
/// considered; a real run without a destination root fails; otherwise the
/// entry is published.
pub open spec fn spec_copy_route(dry_run: bool, has_root: bool) -> CopyRoute {
    if dry_run {
        CopyRoute::Simulate
    } else if !has_root {
        CopyRoute::MissingDestinationRoot
    } else {
        CopyRoute::Publish
    }
}

/// The class a dry run reports for a destination expectation.
pub open spec fn simulated_counter(e: DestExpectation) -> Counter {
    match e {
        DestExpectation::Missing => Counter::Copied,
        DestExpectation::ReplaceExisting => Counter::Replaced,
        DestExpectation::RepairPermissions => Counter::PermissionsRepaired,
    }
}

/// The class a successful publication reports
/// (mirrors `From<PublishOutcome> for CopyOutcome`).
pub open spec fn published_counter(p: PublishKind) -> Counter {
    match p {
        PublishKind::Created => Counter::Copied,
        PublishKind::Replaced => Counter::Replaced,
        PublishKind::PermissionsRepaired => Counter::PermissionsRepaired,
    }
}

/// The class a publication result reports: any error is a failure.
pub open spec fn publish_result_counter<E>(r: Result<PublishKind, E>) -> Counter {
    match r {
        Ok(p) => published_counter(p),
        Err(_) => Counter::Failed,
    }
}

/// The class a copy row reports.
pub open spec fn copy_row_counter<E>(row: CopyRow<E>) -> Counter {
    match row {
        CopyRow::Copied => Counter::Copied,
        CopyRow::Replaced => Counter::Replaced,
        CopyRow::PermissionsRepaired => Counter::PermissionsRepaired,
        CopyRow::MissingDestinationRoot => Counter::Failed,
        CopyRow::PublishFailed(_) => Counter::Failed,
    }
}

/// The classes a publication can end in.
pub open spec fn is_publish_counter(c: Counter) -> bool {
    c == Counter::Copied || c == Counter::Replaced || c == Counter::PermissionsRepaired || c
        == Counter::Failed
}

/// The class of one entry. `published` is the class of the entry's
/// publication result; it is consulted only for an entry that is published.
pub open spec fn entry_counter(kind: EntryKind, dry_run: bool, has_root: bool, published: Counter) -> Counter {
    match kind {
        EntryKind::NoOp => Counter::UpToDate,
        EntryKind::Skip => Counter::Skipped,
        EntryKind::Failure => Counter::Failed,
        EntryKind::Copy(e) => match spec_copy_route(dry_run, has_root) {
            CopyRoute::Simulate => simulated_counter(e),
            CopyRoute::MissingDestinationRoot => Counter::Failed,
            CopyRoute::Publish => published,
        },
    }
}

/// Whether an entry is published.
pub open spec fn entry_publishes(kind: EntryKind, dry_run: bool, has_root: bool) -> bool {
    kind is Copy && spec_copy_route(dry_run, has_root) == CopyRoute::Publish
}

/// The events one entry produces, in order: its publication (if it is
/// published), then its row (unless it is up to date).
pub open spec fn entry_log(kind: EntryKind, dry_run: bool, has_root: bool, published: Counter) -> Seq<Event> {
    let c = entry_counter(kind, dry_run, has_root, published);
    let publication = if entry_publishes(kind, dry_run, has_root) {
        seq![Event::Publish(published)]
    } else {
        Seq::<Event>::empty()
    };
    let row = if c != Counter::UpToDate {
        seq![Event::Report(c)]
    } else {
        Seq::<Event>::empty()
    };
    publication + row
}

/// The events a whole plan produces: each entry's events, in plan order.
/// `pubs[i]` is the class of entry `i`'s publication result.
pub open spec fn plan_log(kinds: Seq<EntryKind>, dry_run: bool, has_root: bool, pubs: Seq<Counter>) -> Seq<Event>
    decreases kinds.len(),
{
    if kinds.len() == 0 {
        Seq::<Event>::empty()
    } else {
        plan_log(kinds.drop_last(), dry_run, has_root, pubs.drop_last()) + entry_log(
            kinds.last(),
            dry_run,
            has_root,
            pubs.last(),
        )
    }
}

/// The class of every entry, in plan order.
pub open spec fn entry_classes(kinds: Seq<EntryKind>, dry_run: bool, has_root: bool, pubs: Seq<Counter>) -> Seq<Counter> {
    Seq::new(kinds.len(), |i: int| entry_counter(kinds[i], dry_run, has_root, pubs[i]))
}

/// Number of occurrences of `c` in `s`.
pub open spec fn count(s: Seq<Counter>, c: Counter) -> nat
    decreases s.len(),
{
    if s.len() == 0 {
        0
    } else {
        count(s.drop_last(), c) + if s.last() == c {
            1nat
        } else {
            0nat
        }
    }
}

/// Number of rows of class `c` in an event log.
pub open spec fn count_reports(log: Seq<Event>, c: Counter) -> nat
    decreases log.len(),
{
    if log.len() == 0 {
        0
    } else {
        count_reports(log.drop_last(), c) + if log.last() == Event::Report(c) {
            1nat
        } else {
            0nat
        }
    }
}

/// Number of publications in an event log.
pub open spec fn count_publications(log: Seq<Event>) -> nat
    decreases log.len(),
{
    if log.len() == 0 {
        0
    } else {
        count_publications(log.drop_last()) + if log.last() is Publish {
            1nat
        } else {
            0nat
        }
    }
}

/// Number of `Copy` entries.
pub open spec fn count_copies(kinds: Seq<EntryKind>) -> nat
    decreases kinds.len(),
{
    if kinds.len() == 0 {
        0
    } else {
        count_copies(kinds.drop_last()) + if kinds.last() is Copy {
            1nat
        } else {
            0nat
        }
    }
}

// ---------------------------------------------------------------------------
// The runner oracle.
// ---------------------------------------------------------------------------

/// The effects of executing a plan, implemented by `executor.rs`.
///
/// `C`, `S`, and `F` are the production payloads of copy, skip, and failure
/// entries; `R` is the destination root; `E` is a publication error.
pub trait EntryRunner<C, S, F, R, E> {
    /// Ghost history of the calls made on this runner.
    spec fn log(&self) -> Seq<Event>;

    /// Publish one copy entry under `destination_root` and return the result.
    fn publish(&mut self, op: &C, destination_root: &R) -> (r: Result<PublishKind, E>)
        ensures
            final(self).log() == old(self).log().push(Event::Publish(publish_result_counter(r))),
    ;

    /// Report the result row of a copy entry.
    fn report_copy(&mut self, op: &C, row: CopyRow<E>)
        ensures
            final(self).log() == old(self).log().push(Event::Report(copy_row_counter(row))),
    ;

    /// Report the result row of a skipped entry.
    fn report_skip(&mut self, skip: &S)
        ensures
            final(self).log() == old(self).log().push(Event::Report(Counter::Skipped)),
    ;

    /// Report the result row of a planning failure.
    fn report_failure(&mut self, failure: &F)
        ensures
            final(self).log() == old(self).log().push(Event::Report(Counter::Failed)),
    ;
}

// ---------------------------------------------------------------------------
// Counters.
// ---------------------------------------------------------------------------

/// The six report counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tally {
    /// Entries created at a missing destination.
    pub copied: usize,
    /// Existing destinations replaced.
    pub replaced: usize,
    /// Existing destinations whose permissions were repaired.
    pub permissions_repaired: usize,
    /// Entries that failed.
    pub failed: usize,
    /// Entries skipped.
    pub skipped: usize,
    /// Entries already up to date.
    pub up_to_date: usize,
}

impl Tally {
    /// The value of counter `c`.
    pub open spec fn get(&self, c: Counter) -> nat {
        match c {
            Counter::Copied => self.copied as nat,
            Counter::Replaced => self.replaced as nat,
            Counter::PermissionsRepaired => self.permissions_repaired as nat,
            Counter::Failed => self.failed as nat,
            Counter::Skipped => self.skipped as nat,
            Counter::UpToDate => self.up_to_date as nat,
        }
    }

    /// The sum of all six counters.
    pub open spec fn total(&self) -> nat {
        self.copied as nat + self.replaced as nat + self.permissions_repaired as nat
            + self.failed as nat + self.skipped as nat + self.up_to_date as nat
    }

    /// All counters zero.
    pub fn new() -> (t: Tally)
        ensures
            forall|c: Counter| #[trigger] t.get(c) == 0,
    {
        Tally { copied: 0, replaced: 0, permissions_repaired: 0, failed: 0, skipped: 0, up_to_date: 0 }
    }

    /// Count one more entry of class `c`.
    fn record(&mut self, c: Counter)
        requires
            old(self).get(c) < usize::MAX,
        ensures
            final(self).get(c) == old(self).get(c) + 1,
            forall|d: Counter| d != c ==> final(self).get(d) == old(self).get(d),
    {
        match c {
            Counter::Copied => self.copied += 1,
            Counter::Replaced => self.replaced += 1,
            Counter::PermissionsRepaired => self.permissions_repaired += 1,
            Counter::Failed => self.failed += 1,
            Counter::Skipped => self.skipped += 1,
            Counter::UpToDate => self.up_to_date += 1,
        }
    }
}

impl Default for Tally {
    fn default() -> (t: Tally)
        ensures
            forall|c: Counter| #[trigger] t.get(c) == 0,
    {
        Tally::new()
    }
}

// ---------------------------------------------------------------------------
// Executable decisions.
// ---------------------------------------------------------------------------

fn simulated_row<E>(e: DestExpectation) -> (row: CopyRow<E>)
    ensures
        copy_row_counter(row) == simulated_counter(e),
{
    match e {
        DestExpectation::Missing => CopyRow::Copied,
        DestExpectation::ReplaceExisting => CopyRow::Replaced,
        DestExpectation::RepairPermissions => CopyRow::PermissionsRepaired,
    }
}

fn simulated_class(e: DestExpectation) -> (c: Counter)
    ensures
        c == simulated_counter(e),
{
    match e {
        DestExpectation::Missing => Counter::Copied,
        DestExpectation::ReplaceExisting => Counter::Replaced,
        DestExpectation::RepairPermissions => Counter::PermissionsRepaired,
    }
}

/// Split a publication result into its class and its row.
fn published_row<E>(r: Result<PublishKind, E>) -> (out: (Counter, CopyRow<E>))
    ensures
        out.0 == publish_result_counter(r),
        copy_row_counter(out.1) == out.0,
{
    match r {
        Ok(PublishKind::Created) => (Counter::Copied, CopyRow::Copied),
        Ok(PublishKind::Replaced) => (Counter::Replaced, CopyRow::Replaced),
        Ok(PublishKind::PermissionsRepaired) => (Counter::PermissionsRepaired, CopyRow::PermissionsRepaired),
        Err(e) => (Counter::Failed, CopyRow::PublishFailed(e)),
    }
}

/// Execute a plan: visit every entry once, in order, publishing exactly the
/// entries `spec_copy_route` says to publish, reporting a row for every entry
/// that is not up to date, and counting every entry in its class.
pub fn run_plan<C, S, F, R, E, Run: EntryRunner<C, S, F, R, E>>(
    runner: &mut Run,
    entries: &[PlanEntry<C, S, F>],
    dry_run: bool,
    destination_root: Option<&R>,
) -> (t: Tally)
    ensures
        exists|pubs: Seq<Counter>|
            {
                &&& pubs.len() == entries@.len()
                &&& forall|i: int| 0 <= i < pubs.len() ==> is_publish_counter(#[trigger] pubs[i])
                &&& final(runner).log() == old(runner).log() + #[trigger] plan_log(
                    kinds(entries@),
                    dry_run,
                    destination_root is Some,
                    pubs,
                )
                &&& forall|c: Counter|
                    #[trigger] t.get(c) == count(
                        entry_classes(kinds(entries@), dry_run, destination_root is Some, pubs),
                        c,
                    )
            },
        t.total() == entries@.len(),
{
    let ghost ks = kinds(entries@);
    let ghost has_root = destination_root is Some;
    let ghost log0 = runner.log();
    let ghost mut pubs: Seq<Counter> = Seq::empty();
    let mut tally = Tally::new();
    let mut i: usize = 0;
    proof {
        assert(ks.take(0) =~= Seq::<EntryKind>::empty());
        assert(entry_classes(ks.take(0), dry_run, has_root, pubs) =~= Seq::<Counter>::empty());
        assert(runner.log() =~= log0 + Seq::<Event>::empty());
    }
    while i < entries.len()
        invariant
            i <= entries@.len(),
            ks == kinds(entries@),
            has_root == (destination_root is Some),
            pubs.len() == i,
            forall|j: int| 0 <= j < pubs.len() ==> is_publish_counter(#[trigger] pubs[j]),
            runner.log() == log0 + plan_log(ks.take(i as int), dry_run, has_root, pubs),
            forall|c: Counter|
                #[trigger] tally.get(c) == count(
                    entry_classes(ks.take(i as int), dry_run, has_root, pubs),
                    c,
                ),
        decreases entries.len() - i,
    {
        let ghost prev_log = runner.log();
        let ghost prev_classes = entry_classes(ks.take(i as int), dry_run, has_root, pubs);
        let ghost kind = ks[i as int];
        assert(kind == entries@[i as int].kind());
        let class: Counter;
        let ghost published: Counter;
        match &entries[i] {
            PlanEntry::Copy(op, expectation) => {
                if dry_run {
                    runner.report_copy(op, simulated_row(*expectation));
                    class = simulated_class(*expectation);
                    proof { published = Counter::Copied; }
                } else {
                    match destination_root {
                        None => {
                            runner.report_copy(op, CopyRow::MissingDestinationRoot);
                            class = Counter::Failed;
                            proof { published = Counter::Copied; }
                        },
                        Some(root) => {
                            let result = runner.publish(op, root);
                            proof { published = publish_result_counter(result); }
                            let (c, row) = published_row(result);
                            runner.report_copy(op, row);
                            class = c;
                        },
                    }
                }
            },
            PlanEntry::NoOp => {
                class = Counter::UpToDate;
                proof { published = Counter::Copied; }
            },
            PlanEntry::Skip(skip) => {
                runner.report_skip(skip);
                class = Counter::Skipped;
                proof { published = Counter::Copied; }
            },
            PlanEntry::Failure(failure) => {
                runner.report_failure(failure);
                class = Counter::Failed;
                proof { published = Counter::Copied; }
            },
        }
        proof {
            let next_pubs = pubs.push(published);
            assert(class == entry_counter(kind, dry_run, has_root, published));
            assert(runner.log() == prev_log + entry_log(kind, dry_run, has_root, published)) by {
                let el = entry_log(kind, dry_run, has_root, published);
                assert(runner.log() =~= prev_log + el);
            }
            assert(ks.take(i + 1).drop_last() =~= ks.take(i as int));
            assert(ks.take(i + 1).last() == kind);
            assert(next_pubs.drop_last() =~= pubs);
            assert(entry_classes(ks.take(i + 1), dry_run, has_root, next_pubs) =~= prev_classes.push(class));
            lemma_count_le_len(prev_classes, class);
            pubs = next_pubs;
        }
        tally.record(class);
        proof {
            assert forall|c: Counter|
                #[trigger] tally.get(c) == count(
                    entry_classes(ks.take(i + 1), dry_run, has_root, pubs),
                    c,
                ) by {
                assert(entry_classes(ks.take(i + 1), dry_run, has_root, pubs) =~= prev_classes.push(class));
                assert(prev_classes.push(class).drop_last() =~= prev_classes);
            }
        }
        i += 1;
    }
    proof {
        assert(ks.take(i as int) =~= ks);
        let classes = entry_classes(ks, dry_run, has_root, pubs);
        assert(entry_classes(ks.take(i as int), dry_run, has_root, pubs) == classes);
        theorem_partition(classes);
        assert(classes.len() == entries@.len());
        assert(tally.get(Counter::Copied) == count(classes, Counter::Copied));
        assert(tally.get(Counter::Replaced) == count(classes, Counter::Replaced));
        assert(tally.get(Counter::PermissionsRepaired) == count(classes, Counter::PermissionsRepaired));
        assert(tally.get(Counter::Failed) == count(classes, Counter::Failed));
        assert(tally.get(Counter::Skipped) == count(classes, Counter::Skipped));
        assert(tally.get(Counter::UpToDate) == count(classes, Counter::UpToDate));
    }
    tally
}

/// The `(failed, total)` pair behind waft's exit status, or `None` when
/// nothing failed. `total` counts every destination the run published or
/// tried to: successes plus failures, excluding skips and up-to-date files.
pub fn failure_summary(t: &Tally) -> (r: Option<(usize, usize)>)
    requires
        t.copied + t.replaced + t.permissions_repaired + t.failed <= usize::MAX,
    ensures
        r is Some <==> t.failed > 0,
        r matches Some(p) ==> {
            &&& p.0 == t.failed
            &&& p.1 == t.copied + t.replaced + t.permissions_repaired + t.failed
            &&& 1 <= p.0 <= p.1
        },
{
    if t.failed > 0 {
        Some((t.failed, t.copied + t.replaced + t.permissions_repaired + t.failed))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Report rendering policy.
// ---------------------------------------------------------------------------

/// Whether `render_report` prints the row of an entry of class `c`.
pub open spec fn spec_prints_row(c: Counter, quiet: bool) -> bool {
    c == Counter::Failed || !quiet
}

/// Decide whether a result row is printed. Quiet suppresses routine
/// progress, never a failure.
pub fn prints_row(c: Counter, quiet: bool) -> (b: bool)
    ensures
        b == spec_prints_row(c, quiet),
        c == Counter::Failed ==> b,
        quiet && c != Counter::Failed ==> !b,
        !quiet ==> b,
{
    match c {
        Counter::Failed => true,
        Counter::Copied | Counter::Replaced | Counter::PermissionsRepaired | Counter::Skipped
        | Counter::UpToDate => !quiet,
    }
}

/// Which parts of the one-line summary `render_report` prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SummaryPlan {
    /// Print the summary line at all.
    pub print: bool,
    /// Include the `, N replaced` clause.
    pub replaced_clause: bool,
    /// Include the `, N permissions repaired` clause.
    pub repaired_clause: bool,
}

/// Decide the shape of the summary line: printed exactly when not quiet;
/// the replace and repair clauses appear exactly when they happened. The
/// copied, failed, skipped, and up-to-date clauses are always present.
pub fn summary_plan(quiet: bool, replaced: usize, permissions_repaired: usize) -> (p: SummaryPlan)
    ensures
        p.print == !quiet,
        p.replaced_clause == (replaced > 0),
        p.repaired_clause == (permissions_repaired > 0),
{
    SummaryPlan { print: !quiet, replaced_clause: replaced > 0, repaired_clause: permissions_repaired > 0 }
}

// ---------------------------------------------------------------------------
// Theorems.
// ---------------------------------------------------------------------------

/// A count never exceeds the length of the sequence.
pub proof fn lemma_count_le_len(s: Seq<Counter>, c: Counter)
    ensures
        count(s, c) <= s.len(),
    decreases s.len(),
{
    if s.len() > 0 {
        lemma_count_le_len(s.drop_last(), c);
    }
}

/// The sum of the six counts over a sequence of classes.
pub open spec fn sum_counts(s: Seq<Counter>) -> nat {
    count(s, Counter::Copied) + count(s, Counter::Replaced) + count(s, Counter::PermissionsRepaired)
        + count(s, Counter::Failed) + count(s, Counter::Skipped) + count(s, Counter::UpToDate)
}

/// Partition: every class is exactly one of the six counters, so the counts
/// sum to the number of entries.
pub proof fn theorem_partition(s: Seq<Counter>)
    ensures
        sum_counts(s) == s.len(),
    decreases s.len(),
{
    if s.len() > 0 {
        theorem_partition(s.drop_last());
    }
}

proof fn lemma_count_reports_concat(a: Seq<Event>, b: Seq<Event>, c: Counter)
    ensures
        count_reports(a + b, c) == count_reports(a, c) + count_reports(b, c),
    decreases b.len(),
{
    if b.len() == 0 {
        assert(a + b =~= a);
    } else {
        assert((a + b).drop_last() =~= a + b.drop_last());
        lemma_count_reports_concat(a, b.drop_last(), c);
    }
}

proof fn lemma_count_publications_concat(a: Seq<Event>, b: Seq<Event>)
    ensures
        count_publications(a + b) == count_publications(a) + count_publications(b),
    decreases b.len(),
{
    if b.len() == 0 {
        assert(a + b =~= a);
    } else {
        assert((a + b).drop_last() =~= a + b.drop_last());
        lemma_count_publications_concat(a, b.drop_last());
    }
}

proof fn lemma_singleton_counts(e: Event, c: Counter)
    ensures
        count_reports(seq![e], c) == if e == Event::Report(c) {
            1nat
        } else {
            0nat
        },
        count_publications(seq![e]) == if e is Publish {
            1nat
        } else {
            0nat
        },
        count_reports(Seq::<Event>::empty(), c) == 0,
        count_publications(Seq::<Event>::empty()) == 0,
{
    let s = seq![e];
    let empty = Seq::<Event>::empty();
    assert(s.drop_last() =~= empty);
    assert(s.last() == e);
    assert(count_reports(empty, c) == 0);
    assert(count_publications(empty) == 0);
    assert(count_reports(s, c) == count_reports(s.drop_last(), c) + if s.last() == Event::Report(c) {
        1nat
    } else {
        0nat
    });
    assert(count_publications(s) == count_publications(s.drop_last()) + if s.last() is Publish {
        1nat
    } else {
        0nat
    });
}

proof fn lemma_entry_classes_drop_last(kinds: Seq<EntryKind>, dry_run: bool, has_root: bool, pubs: Seq<Counter>)
    requires
        kinds.len() > 0,
        pubs.len() == kinds.len(),
    ensures
        entry_classes(kinds, dry_run, has_root, pubs) =~= entry_classes(
            kinds.drop_last(),
            dry_run,
            has_root,
            pubs.drop_last(),
        ).push(entry_counter(kinds.last(), dry_run, has_root, pubs.last())),
{
}

/// Rows agree with counters: for every class except `UpToDate`, the number
/// of rows reported with that class equals the number of entries in it, and
/// no row is ever reported as up to date. In particular the number of
/// `FAILED` lines equals `failed`.
pub proof fn theorem_reports_match_counters(
    kinds: Seq<EntryKind>,
    dry_run: bool,
    has_root: bool,
    pubs: Seq<Counter>,
    c: Counter,
)
    requires
        pubs.len() == kinds.len(),
    ensures
        c != Counter::UpToDate ==> count_reports(plan_log(kinds, dry_run, has_root, pubs), c)
            == count(entry_classes(kinds, dry_run, has_root, pubs), c),
        count_reports(plan_log(kinds, dry_run, has_root, pubs), Counter::UpToDate) == 0,
    decreases kinds.len(),
{
    if kinds.len() > 0 {
        let k = kinds.last();
        let p = pubs.last();
        theorem_reports_match_counters(kinds.drop_last(), dry_run, has_root, pubs.drop_last(), c);
        lemma_entry_classes_drop_last(kinds, dry_run, has_root, pubs);
        let prefix = plan_log(kinds.drop_last(), dry_run, has_root, pubs.drop_last());
        let el = entry_log(k, dry_run, has_root, p);
        lemma_count_reports_concat(prefix, el, c);
        lemma_count_reports_concat(prefix, el, Counter::UpToDate);
        let ec = entry_counter(k, dry_run, has_root, p);
        let publication = if entry_publishes(k, dry_run, has_root) {
            seq![Event::Publish(p)]
        } else {
            Seq::<Event>::empty()
        };
        let row = if ec != Counter::UpToDate {
            seq![Event::Report(ec)]
        } else {
            Seq::<Event>::empty()
        };
        assert(el == publication + row);
        lemma_count_reports_concat(publication, row, c);
        lemma_count_reports_concat(publication, row, Counter::UpToDate);
        lemma_singleton_counts(Event::Publish(p), c);
        lemma_singleton_counts(Event::Publish(p), Counter::UpToDate);
        lemma_singleton_counts(Event::Report(ec), c);
        lemma_singleton_counts(Event::Report(ec), Counter::UpToDate);
        let classes = entry_classes(kinds, dry_run, has_root, pubs);
        assert(classes.drop_last() =~= entry_classes(kinds.drop_last(), dry_run, has_root, pubs.drop_last()));
    }
}

/// Publication happens exactly where the route says: a dry run, or a real
/// run without a destination root, publishes nothing; otherwise exactly the
/// `Copy` entries are published, one publication each.
pub proof fn theorem_no_publication_without_route(
    kinds: Seq<EntryKind>,
    dry_run: bool,
    has_root: bool,
    pubs: Seq<Counter>,
)
    requires
        pubs.len() == kinds.len(),
    ensures
        count_publications(plan_log(kinds, dry_run, has_root, pubs)) == if !dry_run && has_root {
            count_copies(kinds)
        } else {
            0
        },
    decreases kinds.len(),
{
    if kinds.len() > 0 {
        let k = kinds.last();
        let p = pubs.last();
        theorem_no_publication_without_route(kinds.drop_last(), dry_run, has_root, pubs.drop_last());
        let prefix = plan_log(kinds.drop_last(), dry_run, has_root, pubs.drop_last());
        let el = entry_log(k, dry_run, has_root, p);
        lemma_count_publications_concat(prefix, el);
        let ec = entry_counter(k, dry_run, has_root, p);
        let publication = if entry_publishes(k, dry_run, has_root) {
            seq![Event::Publish(p)]
        } else {
            Seq::<Event>::empty()
        };
        let row = if ec != Counter::UpToDate {
            seq![Event::Report(ec)]
        } else {
            Seq::<Event>::empty()
        };
        lemma_count_publications_concat(publication, row);
        lemma_singleton_counts(Event::Publish(p), ec);
        lemma_singleton_counts(Event::Report(ec), ec);
    }
}

/// Every publication is immediately followed by the row of the same entry,
/// and that row's class is the class of the publication's result.
pub proof fn theorem_publication_is_reported(
    kinds: Seq<EntryKind>,
    dry_run: bool,
    has_root: bool,
    pubs: Seq<Counter>,
    j: int,
)
    requires
        pubs.len() == kinds.len(),
        forall|i: int| 0 <= i < pubs.len() ==> is_publish_counter(#[trigger] pubs[i]),
        0 <= j < plan_log(kinds, dry_run, has_root, pubs).len(),
        plan_log(kinds, dry_run, has_root, pubs)[j] is Publish,
    ensures
        j + 1 < plan_log(kinds, dry_run, has_root, pubs).len(),
        plan_log(kinds, dry_run, has_root, pubs)[j + 1] == Event::Report(
            plan_log(kinds, dry_run, has_root, pubs)[j]->Publish_0,
        ),
    decreases kinds.len(),
{
    if kinds.len() > 0 {
        let k = kinds.last();
        let p = pubs.last();
        let prefix = plan_log(kinds.drop_last(), dry_run, has_root, pubs.drop_last());
        let el = entry_log(k, dry_run, has_root, p);
        let log = plan_log(kinds, dry_run, has_root, pubs);
        assert(log == prefix + el);
        assert(is_publish_counter(pubs[pubs.len() - 1]));
        if j < prefix.len() {
            assert(log[j] == prefix[j]);
            theorem_publication_is_reported(kinds.drop_last(), dry_run, has_root, pubs.drop_last(), j);
            if j + 1 >= prefix.len() {
                // The prefix ends with a row, never with a publication.
                lemma_plan_log_ends_with_row(kinds.drop_last(), dry_run, has_root, pubs.drop_last());
                assert(false);
            }
            assert(log[j + 1] == prefix[j + 1]);
        } else {
            let ec = entry_counter(k, dry_run, has_root, p);
            assert(el[j - prefix.len()] is Publish);
            assert(entry_publishes(k, dry_run, has_root));
            assert(ec == p);
            assert(p != Counter::UpToDate);
            assert(el =~= seq![Event::Publish(p), Event::Report(p)]);
            assert(j - prefix.len() == 0);
        }
    }
}

proof fn lemma_plan_log_ends_with_row(kinds: Seq<EntryKind>, dry_run: bool, has_root: bool, pubs: Seq<Counter>)
    requires
        pubs.len() == kinds.len(),
        forall|i: int| 0 <= i < pubs.len() ==> is_publish_counter(#[trigger] pubs[i]),
    ensures
        plan_log(kinds, dry_run, has_root, pubs).len() > 0 ==> plan_log(
            kinds,
            dry_run,
            has_root,
            pubs,
        ).last() is Report,
    decreases kinds.len(),
{
    if kinds.len() > 0 {
        let k = kinds.last();
        let p = pubs.last();
        assert(is_publish_counter(pubs[pubs.len() - 1]));
        let prefix = plan_log(kinds.drop_last(), dry_run, has_root, pubs.drop_last());
        let el = entry_log(k, dry_run, has_root, p);
        assert forall|i: int| 0 <= i < pubs.drop_last().len() implies is_publish_counter(
            #[trigger] pubs.drop_last()[i],
        ) by {
            assert(pubs.drop_last()[i] == pubs[i]);
        }
        lemma_plan_log_ends_with_row(kinds.drop_last(), dry_run, has_root, pubs.drop_last());
        let ec = entry_counter(k, dry_run, has_root, p);
        if el.len() > 0 {
            assert((prefix + el).last() == el.last());
            if entry_publishes(k, dry_run, has_root) {
                assert(ec == p);
            }
        } else {
            assert(prefix + el =~= prefix);
        }
    }
}

/// A dry run predicts the class a successful publication of the matching
/// kind reports: `Missing` and `Created` are both `Copied`,
/// `ReplaceExisting` and `Replaced` are both `Replaced`, and
/// `RepairPermissions` and `PermissionsRepaired` are both
/// `PermissionsRepaired`.
pub proof fn theorem_dry_run_agrees_with_publication()
    ensures
        simulated_counter(DestExpectation::Missing) == published_counter(PublishKind::Created),
        simulated_counter(DestExpectation::ReplaceExisting) == published_counter(PublishKind::Replaced),
        simulated_counter(DestExpectation::RepairPermissions) == published_counter(
            PublishKind::PermissionsRepaired,
        ),
{
}

proof fn lemma_count_pos_has_witness(s: Seq<Counter>, c: Counter)
    ensures
        count(s, c) > 0 <==> exists|i: int| 0 <= i < s.len() && s[i] == c,
    decreases s.len(),
{
    if s.len() > 0 {
        lemma_count_pos_has_witness(s.drop_last(), c);
        if count(s.drop_last(), c) > 0 {
            let i = choose|i: int| 0 <= i < s.drop_last().len() && s.drop_last()[i] == c;
            assert(s[i] == c);
        }
        if exists|i: int| 0 <= i < s.len() && s[i] == c {
            let i = choose|i: int| 0 <= i < s.len() && s[i] == c;
            if i < s.len() - 1 {
                assert(s.drop_last()[i] == c);
            }
        }
    }
}

/// For a tally produced by [`run_plan`], the exit status reports failure
/// exactly when some entry's class is `Failed` — equivalently, when some
/// reported row is a failure.
pub proof fn theorem_failure_status_agrees_with_rows(
    t: Tally,
    kinds: Seq<EntryKind>,
    dry_run: bool,
    has_root: bool,
    pubs: Seq<Counter>,
)
    requires
        pubs.len() == kinds.len(),
        forall|c: Counter| #[trigger] t.get(c) == count(entry_classes(kinds, dry_run, has_root, pubs), c),
    ensures
        t.failed > 0 <==> exists|i: int|
            0 <= i < kinds.len() && #[trigger] entry_counter(kinds[i], dry_run, has_root, pubs[i]) == Counter::Failed,
        t.failed > 0 <==> count_reports(plan_log(kinds, dry_run, has_root, pubs), Counter::Failed) > 0,
{
    let classes = entry_classes(kinds, dry_run, has_root, pubs);
    assert(t.get(Counter::Failed) == count(classes, Counter::Failed));
    lemma_count_pos_has_witness(classes, Counter::Failed);
    theorem_reports_match_counters(kinds, dry_run, has_root, pubs, Counter::Failed);
    if t.failed > 0 {
        let i = choose|i: int| 0 <= i < classes.len() && classes[i] == Counter::Failed;
        assert(entry_counter(kinds[i], dry_run, has_root, pubs[i]) == Counter::Failed);
    }
    if exists|i: int|
        0 <= i < kinds.len() && #[trigger] entry_counter(kinds[i], dry_run, has_root, pubs[i]) == Counter::Failed {
        let i = choose|i: int|
            0 <= i < kinds.len() && #[trigger] entry_counter(kinds[i], dry_run, has_root, pubs[i]) == Counter::Failed;
        assert(classes[i] == Counter::Failed);
    }
}

} // verus!
