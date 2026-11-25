//! Verified kernel: how `executor::execute` walks a copy plan, what it
//! counts, what it reports, and what the exit status says about it.
//!
//! # What it decides
//!
//! [`run_plan`] is the per-entry loop of `executor::execute`. For every plan
//! entry it decides:
//!
//! * whether the entry is published at all (`spec_copy_route`: a dry run never
//!   publishes, a plan without a destination root never publishes, and only
//!   `Copy` entries are ever published);
//! * which of the six report counters it lands in ([`Counter`]);
//! * whether it produces a result row (every entry except an up-to-date one),
//!   and what that row says.
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
//! named lemma over the specification that `ensures` refers to). The
//! runner's ghost log records every call together with the plan-entry
//! payload it was given and the values it carried (the publication result,
//! the row), so these statements are about *which* entry each effect
//! concerns, not only about how many effects there are:
//!
//! * [`run_plan`] — there are publication results such that the runner's
//!   log grows by exactly `plan_log``(entries, dry_run, has_root, results)`:
//!   the entries' own events, in plan order. Every counter equals the number
//!   of entries whose class (`entry_counter`) is that counter, and the
//!   counters that make up the exit status's total fit in `usize`.
//! * `theorem_publishes_exactly_the_copy_entries` — the payloads published,
//!   in order, are exactly the `Copy` entries of the plan, each once, in plan
//!   order; a dry run, or a plan without a destination root, publishes
//!   nothing.
//! * `theorem_reports_every_entry_but_up_to_date` — the rows reported, in
//!   order, name exactly the entries that are not up to date, each once, in
//!   plan order.
//! * `theorem_publication_is_reported` — each publication is immediately
//!   followed by the row of the same copy entry, and that row is the
//!   publication's result: what it did, or the very error it failed with.
//! * `theorem_partition` — the six counters sum to the number of plan
//!   entries: each entry is counted exactly once. With
//!   `lemma_count_le_len`, no counter can overflow.
//! * `theorem_reports_match_counters` — for every counter except
//!   `UpToDate`, the number of rows reported with that class equals the
//!   counter, and no row is ever reported as up to date.
//! * `theorem_dry_run_agrees_with_publication` — the row a dry run predicts
//!   for an expectation is the row a successful publication of the matching
//!   kind reports.
//! * [`failure_summary`] — `Some` exactly when `failed > 0`; then the pair is
//!   `(failed, copied + replaced + permissions_repaired + failed)` and
//!   `1 <= failed <= total`. `theorem_failure_status_agrees_with_rows`:
//!   for a tally produced by [`run_plan`], `failed > 0` exactly when some
//!   reported row is a failure.
//! * [`prints_row`] — `--quiet` never hides a failure and hides every other
//!   row; [`summary_plan`] prints the summary exactly when not quiet, with the
//!   replaced/repaired clauses exactly when those counters are non-zero.
//!
//! What is *not* a theorem: that the number of `FAILED:` lines
//! `render_report` prints equals `CopyReport::failed`. That follows from
//! `theorem_reports_match_counters` and [`prints_row`] only through trusted
//! glue in `executor.rs` (the runner turning each reported row into one
//! `CopyResult`, `row_counter`, and `render_report`'s loop over the results).
//!
//! # Trust assumptions
//!
//! * The production [`EntryRunner`] does what its method names say: `publish`
//!   performs one publication of the given copy entry and returns its result
//!   without emitting a row; each `report_*` call pushes exactly one result
//!   row describing the given entry, saying what the given [`CopyRow`] says.
//!   The runner's `EntryRunner::log` is a ghost history of those calls; its
//!   `ensures` say only that a call happened, with which arguments, and what
//!   it returned.
//! * The glue converting `PlannedEntry`, `DestinationExpectation`, and
//!   `PublishOutcome` into [`PlanEntry`], [`DestExpectation`], and
//!   [`PublishKind`] is a faithful one-to-one mapping (exhaustive matches,
//!   pinned by unit tests in `executor.rs`).
//! * `failure_summary` is only given counters whose sum fits in `usize`. Every
//!   `CopyReport` waft builds carries the counters of a [`run_plan`] tally
//!   verbatim, and [`run_plan`] ensures that sum fits.

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

/// One effect the kernel asks the runner for, as recorded in its ghost log:
/// the plan-entry payload the effect concerns and the values it carried.
pub enum Event<C, S, F, E> {
    /// Copy entry `C` was published, and the publication returned this.
    Publish(C, Result<PublishKind, E>),
    /// The row of copy entry `C` was reported, saying this.
    ReportCopy(C, CopyRow<E>),
    /// The row of skipped entry `S` was reported.
    ReportSkip(S),
    /// The row of planning failure `F` was reported.
    ReportFailure(F),
}

/// The plan entry a result row is about.
pub enum Subject<C, S, F> {
    /// A copy entry.
    Copy(C),
    /// A skipped entry.
    Skip(S),
    /// A planning failure.
    Failure(F),
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

/// The row a dry run reports for a destination expectation.
pub open spec fn simulated_copy_row<E>(e: DestExpectation) -> CopyRow<E> {
    match e {
        DestExpectation::Missing => CopyRow::Copied,
        DestExpectation::ReplaceExisting => CopyRow::Replaced,
        DestExpectation::RepairPermissions => CopyRow::PermissionsRepaired,
    }
}

/// The row a publication result is reported as: what the publication did,
/// or the very error it failed with.
pub open spec fn result_row<E>(r: Result<PublishKind, E>) -> CopyRow<E> {
    match r {
        Ok(PublishKind::Created) => CopyRow::Copied,
        Ok(PublishKind::Replaced) => CopyRow::Replaced,
        Ok(PublishKind::PermissionsRepaired) => CopyRow::PermissionsRepaired,
        Err(e) => CopyRow::PublishFailed(e),
    }
}

/// The row of a copy entry handled by `route`, given its publication result
/// (consulted only when the entry is published).
pub open spec fn copy_row<E>(route: CopyRoute, e: DestExpectation, r: Result<PublishKind, E>) -> CopyRow<E> {
    match route {
        CopyRoute::Simulate => simulated_copy_row(e),
        CopyRoute::MissingDestinationRoot => CopyRow::MissingDestinationRoot,
        CopyRoute::Publish => result_row(r),
    }
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
pub open spec fn entry_publishes<C, S, F>(entry: PlanEntry<C, S, F>, dry_run: bool, has_root: bool) -> bool {
    entry is Copy && spec_copy_route(dry_run, has_root) == CopyRoute::Publish
}

/// The events of one entry, given its publication result `r` (consulted
/// only when the entry is published): a copy entry that is published is
/// published and then reported with the publication's result; any other copy
/// entry is reported with the row its route gives; a skip or a planning
/// failure is reported; an up-to-date entry produces nothing.
pub open spec fn entry_log<C, S, F, E>(
    entry: PlanEntry<C, S, F>,
    dry_run: bool,
    has_root: bool,
    r: Result<PublishKind, E>,
) -> Seq<Event<C, S, F, E>> {
    match entry {
        PlanEntry::Copy(op, e) => {
            let route = spec_copy_route(dry_run, has_root);
            let row = seq![Event::ReportCopy(op, copy_row(route, e, r))];
            if route == CopyRoute::Publish {
                seq![Event::Publish(op, r)] + row
            } else {
                row
            }
        },
        PlanEntry::NoOp => Seq::<Event<C, S, F, E>>::empty(),
        PlanEntry::Skip(s) => seq![Event::ReportSkip(s)],
        PlanEntry::Failure(f) => seq![Event::ReportFailure(f)],
    }
}

/// The events a whole plan produces: each entry's events, in plan order.
/// `results[i]` is entry `i`'s publication result.
pub open spec fn plan_log<C, S, F, E>(
    entries: Seq<PlanEntry<C, S, F>>,
    dry_run: bool,
    has_root: bool,
    results: Seq<Result<PublishKind, E>>,
) -> Seq<Event<C, S, F, E>>
    decreases entries.len(),
{
    if entries.len() == 0 {
        Seq::<Event<C, S, F, E>>::empty()
    } else {
        plan_log(entries.drop_last(), dry_run, has_root, results.drop_last()) + entry_log(
            entries.last(),
            dry_run,
            has_root,
            results.last(),
        )
    }
}

/// The class of every entry, in plan order.
pub open spec fn entry_classes<C, S, F, E>(
    entries: Seq<PlanEntry<C, S, F>>,
    dry_run: bool,
    has_root: bool,
    results: Seq<Result<PublishKind, E>>,
) -> Seq<Counter> {
    Seq::new(
        entries.len(),
        |i: int| entry_counter(entries[i].kind(), dry_run, has_root, publish_result_counter(results[i])),
    )
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

/// The class of a row event; `None` for a publication.
pub open spec fn report_class<C, S, F, E>(e: Event<C, S, F, E>) -> Option<Counter> {
    match e {
        Event::Publish(_, _) => None,
        Event::ReportCopy(_, row) => Some(copy_row_counter(row)),
        Event::ReportSkip(_) => Some(Counter::Skipped),
        Event::ReportFailure(_) => Some(Counter::Failed),
    }
}

/// The classes of a log's rows, in order.
pub open spec fn report_classes<C, S, F, E>(log: Seq<Event<C, S, F, E>>) -> Seq<Counter>
    decreases log.len(),
{
    if log.len() == 0 {
        Seq::<Counter>::empty()
    } else {
        report_classes(log.drop_last()) + match report_class(log.last()) {
            Some(c) => seq![c],
            None => Seq::<Counter>::empty(),
        }
    }
}

/// Number of rows of class `c` in an event log.
pub open spec fn count_reports<C, S, F, E>(log: Seq<Event<C, S, F, E>>, c: Counter) -> nat {
    count(report_classes(log), c)
}

/// The copy entries a log publishes, in order.
pub open spec fn published_ops<C, S, F, E>(log: Seq<Event<C, S, F, E>>) -> Seq<C>
    decreases log.len(),
{
    if log.len() == 0 {
        Seq::<C>::empty()
    } else {
        published_ops(log.drop_last()) + match log.last() {
            Event::Publish(op, _) => seq![op],
            _ => Seq::<C>::empty(),
        }
    }
}

/// The copy entries of a plan, in plan order.
pub open spec fn copy_ops<C, S, F>(entries: Seq<PlanEntry<C, S, F>>) -> Seq<C>
    decreases entries.len(),
{
    if entries.len() == 0 {
        Seq::<C>::empty()
    } else {
        copy_ops(entries.drop_last()) + match entries.last() {
            PlanEntry::Copy(op, _) => seq![op],
            _ => Seq::<C>::empty(),
        }
    }
}

/// The entry a row event is about; `None` for a publication.
pub open spec fn row_subject<C, S, F, E>(e: Event<C, S, F, E>) -> Option<Subject<C, S, F>> {
    match e {
        Event::Publish(_, _) => None,
        Event::ReportCopy(op, _) => Some(Subject::Copy(op)),
        Event::ReportSkip(s) => Some(Subject::Skip(s)),
        Event::ReportFailure(f) => Some(Subject::Failure(f)),
    }
}

/// The entries a log's rows are about, in order.
pub open spec fn row_subjects<C, S, F, E>(log: Seq<Event<C, S, F, E>>) -> Seq<Subject<C, S, F>>
    decreases log.len(),
{
    if log.len() == 0 {
        Seq::<Subject<C, S, F>>::empty()
    } else {
        row_subjects(log.drop_last()) + match row_subject(log.last()) {
            Some(s) => seq![s],
            None => Seq::<Subject<C, S, F>>::empty(),
        }
    }
}

/// An entry as the subject of a row; `None` for an up-to-date entry.
pub open spec fn entry_subject<C, S, F>(entry: PlanEntry<C, S, F>) -> Option<Subject<C, S, F>> {
    match entry {
        PlanEntry::Copy(op, _) => Some(Subject::Copy(op)),
        PlanEntry::NoOp => None,
        PlanEntry::Skip(s) => Some(Subject::Skip(s)),
        PlanEntry::Failure(f) => Some(Subject::Failure(f)),
    }
}

/// The entries of a plan that are not up to date, in plan order.
pub open spec fn reported_entries<C, S, F>(entries: Seq<PlanEntry<C, S, F>>) -> Seq<Subject<C, S, F>>
    decreases entries.len(),
{
    if entries.len() == 0 {
        Seq::<Subject<C, S, F>>::empty()
    } else {
        reported_entries(entries.drop_last()) + match entry_subject(entries.last()) {
            Some(s) => seq![s],
            None => Seq::<Subject<C, S, F>>::empty(),
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
    spec fn log(&self) -> Seq<Event<C, S, F, E>>;

    /// Publish one copy entry under `destination_root` and return the result.
    fn publish(&mut self, op: &C, destination_root: &R) -> (r: Result<PublishKind, E>)
        ensures
            final(self).log() == old(self).log().push(Event::Publish(*op, r)),
    ;

    /// Report the result row of a copy entry.
    fn report_copy(&mut self, op: &C, row: CopyRow<E>)
        ensures
            final(self).log() == old(self).log().push(Event::ReportCopy(*op, row)),
    ;

    /// Report the result row of a skipped entry.
    fn report_skip(&mut self, skip: &S)
        ensures
            final(self).log() == old(self).log().push(Event::ReportSkip(*skip)),
    ;

    /// Report the result row of a planning failure.
    fn report_failure(&mut self, failure: &F)
        ensures
            final(self).log() == old(self).log().push(Event::ReportFailure(*failure)),
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
        row == simulated_copy_row::<E>(e),
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
        out.1 == result_row(r),
{
    match r {
        Ok(PublishKind::Created) => (Counter::Copied, CopyRow::Copied),
        Ok(PublishKind::Replaced) => (Counter::Replaced, CopyRow::Replaced),
        Ok(PublishKind::PermissionsRepaired) => (Counter::PermissionsRepaired, CopyRow::PermissionsRepaired),
        Err(e) => (Counter::Failed, CopyRow::PublishFailed(e)),
    }
}

/// Execute a plan: for each entry in plan order, publish it if
/// `spec_copy_route` says to, report its row unless it is up to date, and
/// count it in its class.
pub fn run_plan<C, S, F, R, E, Run: EntryRunner<C, S, F, R, E>>(
    runner: &mut Run,
    entries: &[PlanEntry<C, S, F>],
    dry_run: bool,
    destination_root: Option<&R>,
) -> (t: Tally)
    ensures
        exists|results: Seq<Result<PublishKind, E>>|
            {
                &&& results.len() == entries@.len()
                &&& final(runner).log() == old(runner).log() + #[trigger] plan_log(
                    entries@,
                    dry_run,
                    destination_root is Some,
                    results,
                )
                &&& forall|c: Counter|
                    #[trigger] t.get(c) == count(
                        entry_classes(entries@, dry_run, destination_root is Some, results),
                        c,
                    )
            },
        t.total() == entries@.len(),
        t.copied + t.replaced + t.permissions_repaired + t.failed <= usize::MAX,
{
    let ghost has_root = destination_root is Some;
    let ghost log0 = runner.log();
    let ghost mut results: Seq<Result<PublishKind, E>> = Seq::empty();
    let mut tally = Tally::new();
    let mut i: usize = 0;
    proof {
        assert(entries@.take(0) =~= Seq::<PlanEntry<C, S, F>>::empty());
        assert(entry_classes(entries@.take(0), dry_run, has_root, results) =~= Seq::<Counter>::empty());
        assert(runner.log() =~= log0 + Seq::<Event<C, S, F, E>>::empty());
    }
    while i < entries.len()
        invariant
            i <= entries@.len(),
            has_root == (destination_root is Some),
            results.len() == i,
            runner.log() == log0 + plan_log(entries@.take(i as int), dry_run, has_root, results),
            forall|c: Counter|
                #[trigger] tally.get(c) == count(
                    entry_classes(entries@.take(i as int), dry_run, has_root, results),
                    c,
                ),
        decreases entries.len() - i,
    {
        let ghost prev_log = runner.log();
        let ghost prev_classes = entry_classes(entries@.take(i as int), dry_run, has_root, results);
        let ghost entry = entries@[i as int];
        let class: Counter;
        let ghost mut published: Result<PublishKind, E> = Ok(PublishKind::Created);
        match &entries[i] {
            PlanEntry::Copy(op, expectation) => {
                if dry_run {
                    runner.report_copy(op, simulated_row(*expectation));
                    class = simulated_class(*expectation);
                } else {
                    match destination_root {
                        None => {
                            runner.report_copy(op, CopyRow::MissingDestinationRoot);
                            class = Counter::Failed;
                        },
                        Some(root) => {
                            let result = runner.publish(op, root);
                            proof {
                                published = result;
                            }
                            let (c, row) = published_row(result);
                            runner.report_copy(op, row);
                            class = c;
                        },
                    }
                }
            },
            PlanEntry::NoOp => {
                class = Counter::UpToDate;
            },
            PlanEntry::Skip(skip) => {
                runner.report_skip(skip);
                class = Counter::Skipped;
            },
            PlanEntry::Failure(failure) => {
                runner.report_failure(failure);
                class = Counter::Failed;
            },
        }
        proof {
            let next = results.push(published);
            assert(class == entry_counter(entry.kind(), dry_run, has_root, publish_result_counter(published)));
            assert(runner.log() =~= prev_log + entry_log(entry, dry_run, has_root, published));
            assert(entries@.take(i + 1).drop_last() =~= entries@.take(i as int));
            assert(entries@.take(i + 1).last() == entry);
            assert(next.drop_last() =~= results);
            assert(next.last() == published);
            assert(runner.log() =~= log0 + plan_log(entries@.take(i + 1), dry_run, has_root, next));
            assert(entry_classes(entries@.take(i + 1), dry_run, has_root, next) =~= prev_classes.push(class));
            lemma_count_le_len(prev_classes, class);
            results = next;
        }
        tally.record(class);
        proof {
            assert forall|c: Counter|
                #[trigger] tally.get(c) == count(
                    entry_classes(entries@.take(i + 1), dry_run, has_root, results),
                    c,
                ) by {
                assert(entry_classes(entries@.take(i + 1), dry_run, has_root, results) =~= prev_classes.push(class));
                assert(prev_classes.push(class).drop_last() =~= prev_classes);
            }
        }
        i += 1;
    }
    proof {
        assert(entries@.take(i as int) =~= entries@);
        let classes = entry_classes(entries@, dry_run, has_root, results);
        assert(entry_classes(entries@.take(i as int), dry_run, has_root, results) == classes);
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
///
/// The requirement holds for every tally [`run_plan`] returns (it is one of
/// its postconditions), and so for every `CopyReport` waft builds, which
/// carries those counters verbatim.
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

proof fn lemma_count_concat(a: Seq<Counter>, b: Seq<Counter>, c: Counter)
    ensures
        count(a + b, c) == count(a, c) + count(b, c),
    decreases b.len(),
{
    if b.len() == 0 {
        assert(a + b =~= a);
    } else {
        assert((a + b).drop_last() =~= a + b.drop_last());
        lemma_count_concat(a, b.drop_last(), c);
    }
}

proof fn lemma_report_classes_concat<C, S, F, E>(a: Seq<Event<C, S, F, E>>, b: Seq<Event<C, S, F, E>>)
    ensures
        report_classes(a + b) == report_classes(a) + report_classes(b),
    decreases b.len(),
{
    if b.len() == 0 {
        assert(a + b =~= a);
        assert(report_classes(a) + report_classes(b) =~= report_classes(a));
    } else {
        assert((a + b).drop_last() =~= a + b.drop_last());
        assert((a + b).last() == b.last());
        lemma_report_classes_concat(a, b.drop_last());
        let tail = match report_class(b.last()) {
            Some(c) => seq![c],
            None => Seq::<Counter>::empty(),
        };
        assert(report_classes(a + b) == report_classes(a + b.drop_last()) + tail);
        assert(report_classes(b) == report_classes(b.drop_last()) + tail);
        assert(report_classes(a + b) =~= report_classes(a) + report_classes(b));
    }
}

proof fn lemma_published_ops_concat<C, S, F, E>(a: Seq<Event<C, S, F, E>>, b: Seq<Event<C, S, F, E>>)
    ensures
        published_ops(a + b) == published_ops(a) + published_ops(b),
    decreases b.len(),
{
    if b.len() == 0 {
        assert(a + b =~= a);
        assert(published_ops(a) + published_ops(b) =~= published_ops(a));
    } else {
        assert((a + b).drop_last() =~= a + b.drop_last());
        assert((a + b).last() == b.last());
        lemma_published_ops_concat(a, b.drop_last());
        let tail = match b.last() {
            Event::Publish(op, _) => seq![op],
            _ => Seq::<C>::empty(),
        };
        assert(published_ops(a + b) == published_ops(a + b.drop_last()) + tail);
        assert(published_ops(b) == published_ops(b.drop_last()) + tail);
        assert(published_ops(a + b) =~= published_ops(a) + published_ops(b));
    }
}

proof fn lemma_row_subjects_concat<C, S, F, E>(a: Seq<Event<C, S, F, E>>, b: Seq<Event<C, S, F, E>>)
    ensures
        row_subjects(a + b) == row_subjects(a) + row_subjects(b),
    decreases b.len(),
{
    if b.len() == 0 {
        assert(a + b =~= a);
        assert(row_subjects(a) + row_subjects(b) =~= row_subjects(a));
    } else {
        assert((a + b).drop_last() =~= a + b.drop_last());
        assert((a + b).last() == b.last());
        lemma_row_subjects_concat(a, b.drop_last());
        let tail = match row_subject(b.last()) {
            Some(s) => seq![s],
            None => Seq::<Subject<C, S, F>>::empty(),
        };
        assert(row_subjects(a + b) == row_subjects(a + b.drop_last()) + tail);
        assert(row_subjects(b) == row_subjects(b.drop_last()) + tail);
        assert(row_subjects(a + b) =~= row_subjects(a) + row_subjects(b));
    }
}

/// What one entry's events contribute: one row of the entry's class unless
/// it is up to date, a publication of its own payload exactly when it is
/// published, and one row about the entry itself unless it is up to date.
proof fn lemma_entry_log<C, S, F, E>(entry: PlanEntry<C, S, F>, dry_run: bool, has_root: bool, r: Result<PublishKind, E>)
    ensures
        ({
            let class = entry_counter(entry.kind(), dry_run, has_root, publish_result_counter(r));
            report_classes(entry_log(entry, dry_run, has_root, r)) == if class != Counter::UpToDate {
                seq![class]
            } else {
                Seq::<Counter>::empty()
            }
        }),
        published_ops(entry_log(entry, dry_run, has_root, r)) == if entry_publishes(entry, dry_run, has_root) {
            seq![entry->Copy_0]
        } else {
            Seq::<C>::empty()
        },
        row_subjects(entry_log(entry, dry_run, has_root, r)) == match entry_subject(entry) {
            Some(s) => seq![s],
            None => Seq::<Subject<C, S, F>>::empty(),
        },
{
    reveal_with_fuel(report_classes, 3);
    reveal_with_fuel(published_ops, 3);
    reveal_with_fuel(row_subjects, 3);
    let el = entry_log(entry, dry_run, has_root, r);
    let empty = Seq::<Event<C, S, F, E>>::empty();
    match entry {
        PlanEntry::Copy(op, e) => {
            let route = spec_copy_route(dry_run, has_root);
            let row = Event::ReportCopy(op, copy_row(route, e, r));
            if route == CopyRoute::Publish {
                assert(el =~= seq![Event::Publish(op, r), row]);
                assert(el.drop_last() =~= seq![Event::Publish(op, r)]);
                assert(el.drop_last().drop_last() =~= empty);
                assert(copy_row_counter(result_row(r)) == publish_result_counter(r));
                assert(report_classes(el) =~= seq![publish_result_counter(r)]);
                assert(published_ops(el) =~= seq![op]);
                assert(row_subjects(el) =~= seq![Subject::<C, S, F>::Copy(op)]);
            } else {
                assert(el =~= seq![row]);
                assert(el.drop_last() =~= empty);
                assert(copy_row_counter(simulated_copy_row::<E>(e)) == simulated_counter(e));
                assert(report_classes(el) =~= seq![copy_row_counter(copy_row(route, e, r))]);
                assert(published_ops(el) =~= Seq::<C>::empty());
                assert(row_subjects(el) =~= seq![Subject::<C, S, F>::Copy(op)]);
            }
        },
        PlanEntry::NoOp => {},
        PlanEntry::Skip(s) => {
            assert(el.drop_last() =~= empty);
            assert(report_classes(el) =~= seq![Counter::Skipped]);
            assert(published_ops(el) =~= Seq::<C>::empty());
            assert(row_subjects(el) =~= seq![Subject::<C, S, F>::Skip(s)]);
        },
        PlanEntry::Failure(f) => {
            assert(el.drop_last() =~= empty);
            assert(report_classes(el) =~= seq![Counter::Failed]);
            assert(published_ops(el) =~= Seq::<C>::empty());
            assert(row_subjects(el) =~= seq![Subject::<C, S, F>::Failure(f)]);
        },
    }
}

proof fn lemma_entry_classes_drop_last<C, S, F, E>(
    entries: Seq<PlanEntry<C, S, F>>,
    dry_run: bool,
    has_root: bool,
    results: Seq<Result<PublishKind, E>>,
)
    requires
        entries.len() > 0,
        results.len() == entries.len(),
    ensures
        entry_classes(entries, dry_run, has_root, results) =~= entry_classes(
            entries.drop_last(),
            dry_run,
            has_root,
            results.drop_last(),
        ).push(entry_counter(entries.last().kind(), dry_run, has_root, publish_result_counter(results.last()))),
{
}

/// Rows agree with counters: for every class except `UpToDate`, the number
/// of rows reported with that class equals the number of entries in it, and
/// no row is ever reported as up to date.
pub proof fn theorem_reports_match_counters<C, S, F, E>(
    entries: Seq<PlanEntry<C, S, F>>,
    dry_run: bool,
    has_root: bool,
    results: Seq<Result<PublishKind, E>>,
    c: Counter,
)
    requires
        results.len() == entries.len(),
    ensures
        c != Counter::UpToDate ==> count_reports(plan_log(entries, dry_run, has_root, results), c)
            == count(entry_classes(entries, dry_run, has_root, results), c),
        count_reports(plan_log(entries, dry_run, has_root, results), Counter::UpToDate) == 0,
    decreases entries.len(),
{
    if entries.len() > 0 {
        let last = entries.last();
        let r = results.last();
        let class = entry_counter(last.kind(), dry_run, has_root, publish_result_counter(r));
        theorem_reports_match_counters(entries.drop_last(), dry_run, has_root, results.drop_last(), c);
        lemma_entry_classes_drop_last(entries, dry_run, has_root, results);
        let prefix = plan_log(entries.drop_last(), dry_run, has_root, results.drop_last());
        let el = entry_log(last, dry_run, has_root, r);
        lemma_report_classes_concat(prefix, el);
        lemma_entry_log(last, dry_run, has_root, r);
        let tail = if class != Counter::UpToDate {
            seq![class]
        } else {
            Seq::<Counter>::empty()
        };
        assert(report_classes(prefix + el) == report_classes(prefix) + tail);
        lemma_count_concat(report_classes(prefix), tail, c);
        lemma_count_concat(report_classes(prefix), tail, Counter::UpToDate);
        let prev = entry_classes(entries.drop_last(), dry_run, has_root, results.drop_last());
        assert(prev.push(class).drop_last() =~= prev);
        assert(prev.push(class).last() == class);
        assert(entry_classes(entries, dry_run, has_root, results) == prev.push(class));
        assert(seq![class].drop_last() =~= Seq::<Counter>::empty());
        assert(seq![class].last() == class);
        assert(count(Seq::<Counter>::empty(), c) == 0);
        assert(count(Seq::<Counter>::empty(), Counter::UpToDate) == 0);
        assert(count(seq![class], c) == count(Seq::<Counter>::empty(), c) + if class == c {
            1nat
        } else {
            0nat
        });
        assert(count(seq![class], Counter::UpToDate) == count(Seq::<Counter>::empty(), Counter::UpToDate)
            + if class == Counter::UpToDate {
            1nat
        } else {
            0nat
        });
        assert(count(prev.push(class), c) == count(prev, c) + if class == c {
            1nat
        } else {
            0nat
        });
    } else {
        assert(plan_log(entries, dry_run, has_root, results) =~= Seq::<Event<C, S, F, E>>::empty());
        assert(report_classes(Seq::<Event<C, S, F, E>>::empty()) =~= Seq::<Counter>::empty());
        assert(entry_classes(entries, dry_run, has_root, results) =~= Seq::<Counter>::empty());
        assert(count(Seq::<Counter>::empty(), c) == 0);
        assert(count(Seq::<Counter>::empty(), Counter::UpToDate) == 0);
    }
}

/// Publication happens exactly where the route says: a dry run, or a real
/// run without a destination root, publishes nothing; otherwise the
/// payloads published are exactly the plan's `Copy` entries, each once, in
/// plan order.
pub proof fn theorem_publishes_exactly_the_copy_entries<C, S, F, E>(
    entries: Seq<PlanEntry<C, S, F>>,
    dry_run: bool,
    has_root: bool,
    results: Seq<Result<PublishKind, E>>,
)
    requires
        results.len() == entries.len(),
    ensures
        published_ops(plan_log(entries, dry_run, has_root, results)) == if !dry_run && has_root {
            copy_ops(entries)
        } else {
            Seq::<C>::empty()
        },
    decreases entries.len(),
{
    if entries.len() > 0 {
        let last = entries.last();
        let r = results.last();
        theorem_publishes_exactly_the_copy_entries(entries.drop_last(), dry_run, has_root, results.drop_last());
        let prefix = plan_log(entries.drop_last(), dry_run, has_root, results.drop_last());
        let el = entry_log(last, dry_run, has_root, r);
        lemma_published_ops_concat(prefix, el);
        lemma_entry_log(last, dry_run, has_root, r);
        if !dry_run && has_root {
            assert(published_ops(prefix + el) =~= copy_ops(entries));
        } else {
            assert(published_ops(prefix + el) =~= Seq::<C>::empty());
        }
    } else {
        assert(published_ops(Seq::<Event<C, S, F, E>>::empty()) =~= Seq::<C>::empty());
    }
}

/// Every entry that is not up to date is reported exactly once, in plan
/// order: the rows reported name exactly those entries, in that order.
pub proof fn theorem_reports_every_entry_but_up_to_date<C, S, F, E>(
    entries: Seq<PlanEntry<C, S, F>>,
    dry_run: bool,
    has_root: bool,
    results: Seq<Result<PublishKind, E>>,
)
    requires
        results.len() == entries.len(),
    ensures
        row_subjects(plan_log(entries, dry_run, has_root, results)) == reported_entries(entries),
    decreases entries.len(),
{
    if entries.len() > 0 {
        let last = entries.last();
        let r = results.last();
        theorem_reports_every_entry_but_up_to_date(entries.drop_last(), dry_run, has_root, results.drop_last());
        let prefix = plan_log(entries.drop_last(), dry_run, has_root, results.drop_last());
        let el = entry_log(last, dry_run, has_root, r);
        lemma_row_subjects_concat(prefix, el);
        lemma_entry_log(last, dry_run, has_root, r);
        assert(row_subjects(prefix + el) =~= reported_entries(entries));
    } else {
        assert(row_subjects(Seq::<Event<C, S, F, E>>::empty()) =~= Seq::<Subject<C, S, F>>::empty());
    }
}

/// Every publication is immediately followed by the row of the same copy
/// entry, and that row is the publication's result: what it did, or the
/// error it failed with.
pub proof fn theorem_publication_is_reported<C, S, F, E>(
    entries: Seq<PlanEntry<C, S, F>>,
    dry_run: bool,
    has_root: bool,
    results: Seq<Result<PublishKind, E>>,
    j: int,
)
    requires
        results.len() == entries.len(),
        0 <= j < plan_log(entries, dry_run, has_root, results).len(),
        plan_log(entries, dry_run, has_root, results)[j] is Publish,
    ensures
        j + 1 < plan_log(entries, dry_run, has_root, results).len(),
        plan_log(entries, dry_run, has_root, results)[j + 1] == Event::<C, S, F, E>::ReportCopy(
            plan_log(entries, dry_run, has_root, results)[j]->Publish_0,
            result_row(plan_log(entries, dry_run, has_root, results)[j]->Publish_1),
        ),
    decreases entries.len(),
{
    if entries.len() > 0 {
        let last = entries.last();
        let r = results.last();
        let prefix = plan_log(entries.drop_last(), dry_run, has_root, results.drop_last());
        let el = entry_log(last, dry_run, has_root, r);
        let log = plan_log(entries, dry_run, has_root, results);
        assert(log == prefix + el);
        if j < prefix.len() {
            assert(log[j] == prefix[j]);
            theorem_publication_is_reported(entries.drop_last(), dry_run, has_root, results.drop_last(), j);
            if j + 1 >= prefix.len() {
                // The prefix ends with a row, never with a publication.
                lemma_plan_log_ends_with_row(entries.drop_last(), dry_run, has_root, results.drop_last());
                assert(false);
            }
            assert(log[j + 1] == prefix[j + 1]);
        } else {
            assert(log[j] == el[j - prefix.len()]);
            match last {
                PlanEntry::Copy(op, e) => {
                    let route = spec_copy_route(dry_run, has_root);
                    if route == CopyRoute::Publish {
                        assert(el =~= seq![
                            Event::Publish(op, r),
                            Event::ReportCopy(op, result_row(r)),
                        ]);
                        assert(j - prefix.len() == 0);
                        assert(log[j + 1] == el[1]);
                    } else {
                        assert(el =~= seq![Event::<C, S, F, E>::ReportCopy(op, copy_row(route, e, r))]);
                    }
                },
                PlanEntry::NoOp => {},
                PlanEntry::Skip(s) => {
                    assert(el =~= seq![Event::<C, S, F, E>::ReportSkip(s)]);
                },
                PlanEntry::Failure(f) => {
                    assert(el =~= seq![Event::<C, S, F, E>::ReportFailure(f)]);
                },
            }
        }
    }
}

proof fn lemma_plan_log_ends_with_row<C, S, F, E>(
    entries: Seq<PlanEntry<C, S, F>>,
    dry_run: bool,
    has_root: bool,
    results: Seq<Result<PublishKind, E>>,
)
    requires
        results.len() == entries.len(),
    ensures
        plan_log(entries, dry_run, has_root, results).len() > 0 ==> !(plan_log(
            entries,
            dry_run,
            has_root,
            results,
        ).last() is Publish),
    decreases entries.len(),
{
    if entries.len() > 0 {
        let last = entries.last();
        let r = results.last();
        let prefix = plan_log(entries.drop_last(), dry_run, has_root, results.drop_last());
        let el = entry_log(last, dry_run, has_root, r);
        lemma_plan_log_ends_with_row(entries.drop_last(), dry_run, has_root, results.drop_last());
        if el.len() > 0 {
            assert((prefix + el).last() == el.last());
            match last {
                PlanEntry::Copy(op, e) => {
                    let route = spec_copy_route(dry_run, has_root);
                    if route == CopyRoute::Publish {
                        assert(el =~= seq![
                            Event::Publish(op, r),
                            Event::ReportCopy(op, result_row(r)),
                        ]);
                    } else {
                        assert(el =~= seq![Event::<C, S, F, E>::ReportCopy(op, copy_row(route, e, r))]);
                    }
                },
                _ => {},
            }
        } else {
            assert(prefix + el =~= prefix);
        }
    }
}

/// A dry run predicts what a successful publication of the matching kind
/// reports: `Missing` and `Created` are both the `Copied` row,
/// `ReplaceExisting` and `Replaced` are both the `Replaced` row, and
/// `RepairPermissions` and `PermissionsRepaired` are both the
/// `PermissionsRepaired` row (and so the same class).
pub proof fn theorem_dry_run_agrees_with_publication<E>()
    ensures
        simulated_copy_row::<E>(DestExpectation::Missing) == result_row::<E>(Ok(PublishKind::Created)),
        simulated_copy_row::<E>(DestExpectation::ReplaceExisting) == result_row::<E>(Ok(PublishKind::Replaced)),
        simulated_copy_row::<E>(DestExpectation::RepairPermissions) == result_row::<E>(
            Ok(PublishKind::PermissionsRepaired),
        ),
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
pub proof fn theorem_failure_status_agrees_with_rows<C, S, F, E>(
    t: Tally,
    entries: Seq<PlanEntry<C, S, F>>,
    dry_run: bool,
    has_root: bool,
    results: Seq<Result<PublishKind, E>>,
)
    requires
        results.len() == entries.len(),
        forall|c: Counter| #[trigger] t.get(c) == count(entry_classes(entries, dry_run, has_root, results), c),
    ensures
        t.failed > 0 <==> exists|i: int|
            0 <= i < entries.len() && #[trigger] entry_counter(
                entries[i].kind(),
                dry_run,
                has_root,
                publish_result_counter(results[i]),
            ) == Counter::Failed,
        t.failed > 0 <==> count_reports(plan_log(entries, dry_run, has_root, results), Counter::Failed) > 0,
{
    let classes = entry_classes(entries, dry_run, has_root, results);
    assert(t.get(Counter::Failed) == count(classes, Counter::Failed));
    lemma_count_pos_has_witness(classes, Counter::Failed);
    theorem_reports_match_counters(entries, dry_run, has_root, results, Counter::Failed);
    if t.failed > 0 {
        let i = choose|i: int| 0 <= i < classes.len() && classes[i] == Counter::Failed;
        assert(entry_counter(entries[i].kind(), dry_run, has_root, publish_result_counter(results[i]))
            == Counter::Failed);
    }
    if exists|i: int|
        0 <= i < entries.len() && #[trigger] entry_counter(
            entries[i].kind(),
            dry_run,
            has_root,
            publish_result_counter(results[i]),
        ) == Counter::Failed {
        let i = choose|i: int|
            0 <= i < entries.len() && #[trigger] entry_counter(
                entries[i].kind(),
                dry_run,
                has_root,
                publish_result_counter(results[i]),
            ) == Counter::Failed;
        assert(classes[i] == Counter::Failed);
    }
}

} // verus!
