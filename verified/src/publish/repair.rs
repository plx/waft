//! In-place permission repair of a verified destination.
//!
//! A repair may only ever change the destination's mode. The machine starts
//! from the plan's premise — the planned destination snapshot and the planned
//! source snapshot describe the same bytes — and then walks production
//! through the publication checks one typestate at a time: the source handle
//! is unchanged, the caller's pre-publication callback (the index-lock
//! recheck) returned `Ok`, the destination parent is revalidated, the
//! destination name resolves to the pinned descriptor, and that descriptor
//! still reads back as planned. Only the state reached after all five hands
//! out the `fchmod` mode, so no other order compiles.
//!
//! Proven:
//!
//! * **R1** a repair asks for no namespace change at all (`no_namespace_change`
//!   holds for every state's trace): its only mutations are `fchmod`s on the
//!   pinned descriptor, so waft never alters the destination's content or
//!   which file the name holds;
//! * **R2** the source's mode is applied exactly once, and only after the five
//!   checks above succeeded in that order;
//! * **R3** [`repair_verdict`] classifies the read-back with content taking
//!   precedence: `Repaired` exactly when the content and the source's mode are
//!   both there, `ContentChanged` whenever the bytes cannot be proved intact
//!   (whatever the mode), `ModeChanged` otherwise;
//! * **R4** a second `fchmod` is handed out only after a read-back that found
//!   the content changed, and its mode is exactly the planned permission bits
//!   (`expected & 0o7777`, which never exceeds `0o7777`); a mode changed by
//!   another writer is never answered with another `fchmod`;
//! * **R5** [`Repaired`] — the only way production reports a repair — exists
//!   only after the read-back found both content and mode as intended, the
//!   destination name was proved again afterwards (name proved last), and the
//!   descriptor was synced; and the repaired file's planned content equals
//!   the planned source content.
//!
//! Trust: production runs each named check and feeds back its real result,
//! applies exactly the modes this machine hands out to the pinned
//! descriptor, and reads the descriptor back faithfully. `fchmod` applying
//! exactly the requested bits is a filesystem assumption (see the report on
//! setgid and FAT-like mounts).

#[allow(unused_imports)]
use vstd::prelude::*;

#[allow(unused_imports)]
use crate::publish::trace::Ev;
#[cfg(verus_keep_ghost)]
use crate::publish::trace::no_namespace_change;

verus! {

/// The parts of a file snapshot a repair compares.
#[derive(Clone, Copy)]
pub struct Snap {
    /// Length in bytes.
    pub len: u64,
    /// Content fingerprint.
    pub fingerprint: u64,
    /// Full `st_mode`.
    pub permissions: u32,
}

/// Two snapshots describe the same bytes.
pub open spec fn same_content(a: Snap, b: Snap) -> bool {
    a.len == b.len && a.fingerprint == b.fingerprint
}

/// What a read-back after a repair shows.
pub enum RepairVerdict {
    /// The planned bytes are still there and the mode is the source's.
    Repaired,
    /// The bytes are not the ones that were verified, or could not be read.
    ContentChanged,
    /// The bytes are intact, but the mode is not the one waft set.
    ModeChanged,
}

/// R3: classify a read-back (`None` when the descriptor could not be read
/// back as a regular file at all).
pub fn repair_verdict(observed: Option<Snap>, expected: Snap, source_permissions: u32) -> (v:
    RepairVerdict)
    ensures
        v is Repaired <==> (observed is Some && same_content(observed->Some_0, expected)
            && observed->Some_0.permissions == source_permissions),
        v is ContentChanged <==> (observed is None || !same_content(observed->Some_0, expected)),
        v is ModeChanged <==> (observed is Some && same_content(observed->Some_0, expected)
            && observed->Some_0.permissions != source_permissions),
{
    match observed {
        None => RepairVerdict::ContentChanged,
        Some(actual) => {
            if actual.len != expected.len || actual.fingerprint != expected.fingerprint {
                RepairVerdict::ContentChanged
            } else if actual.permissions != source_permissions {
                RepairVerdict::ModeChanged
            } else {
                RepairVerdict::Repaired
            }
        },
    }
}

/// R4: the mode a content-changed repair puts back — the permission bits of
/// the planned `st_mode`, the only bits `fchmod` gives meaning to.
pub fn revert_mode(expected_permissions: u32) -> (m: u32)
    ensures
        m == expected_permissions & 0o7777u32,
        m <= 0o7777u32,
{
    let m = expected_permissions & 0o7777u32;
    assert(expected_permissions & 0o7777u32 <= 0o7777u32) by (bit_vector);
    m
}

/// The checks a repair requires, in order, before its `fchmod`.
pub open spec fn repair_checks() -> Seq<Ev> {
    seq![
        Ev::SourceUnchanged,
        Ev::BeforePublish,
        Ev::ParentRevalidated,
        Ev::DestIsPinned { holds: true },
        Ev::PinnedMatches { holds: true },
    ]
}

/// The trace of a reported repair.
pub open spec fn repaired_trace(source_permissions: u32) -> Seq<Ev> {
    repair_checks() + seq![
        Ev::Fchmod { mode: source_permissions, ok: true },
        Ev::ReadBack { content_matches: true, mode_matches: true },
        Ev::DestIsPinned { holds: true },
        Ev::Sync { ok: true },
    ]
}

/// The repair premise does not hold: the planned destination content is not
/// the planned source content.
pub struct PremiseFailed;

/// A repair whose premise holds; production's next step is the
/// source-unchanged check.
pub struct Repair {
    expected: Snap,
    source_permissions: u32,
    #[allow(dead_code)]
    source: Ghost<Snap>,
    #[allow(dead_code)]
    trace: Ghost<Seq<Ev>>,
}

/// The shared invariant of the pre-`fchmod` states: the premise holds and the
/// trace is the first `n` checks.
pub open spec fn checks_done(expected: Snap, source: Snap, trace: Seq<Ev>, n: int) -> bool {
    &&& same_content(expected, source)
    &&& 0 <= n <= 5
    &&& trace == repair_checks().take(n)
}

impl Repair {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        checks_done(self.expected, self.source@, self.trace@, 0)
    }

    /// The planned destination snapshot.
    pub closed spec fn expected(&self) -> Snap {
        self.expected
    }

    /// The planned source snapshot.
    pub closed spec fn source(&self) -> Snap {
        self.source@
    }

    /// The mode the repair will apply: the source's full `st_mode`.
    pub closed spec fn source_permissions(&self) -> u32 {
        self.source_permissions
    }

    /// Start a repair of a destination planned as `expected` towards a source
    /// planned as `source`, whose current mode is `source_permissions`. A
    /// repair changes only the mode, so it is refused unless the two planned
    /// snapshots describe the same bytes.
    pub fn start(expected: Snap, source: Snap, source_permissions: u32) -> (r: Result<
        Repair,
        PremiseFailed,
    >)
        ensures
            r is Ok <==> same_content(expected, source),
            r matches Ok(repair) ==> {
                &&& repair.expected() == expected
                &&& repair.source() == source
                &&& repair.source_permissions() == source_permissions
            },
    {
        if expected.len == source.len && expected.fingerprint == source.fingerprint {
            let ghost trace = repair_checks().take(0);
            Ok(Repair { expected, source_permissions, source: Ghost(source), trace: Ghost(trace) })
        } else {
            Err(PremiseFailed)
        }
    }

    /// Feed the source-unchanged check.
    pub fn after_source_check<E>(self, checked: Result<(), E>) -> (r: Result<RepairCheck<2>, E>)
        ensures
            r is Ok <==> checked is Ok,
            checked matches Err(e) ==> r == Err::<RepairCheck<2>, E>(e),
            r matches Ok(next) ==> next.expected() == self.expected() && next.source()
                == self.source() && next.source_permissions() == self.source_permissions(),
    {
        proof {
            use_type_invariant(&self);
        }
        match checked {
            Ok(()) => {
                let ghost trace = repair_checks().take(1);
                Ok(
                    RepairCheck {
                        expected: self.expected,
                        source_permissions: self.source_permissions,
                        source: self.source,
                        trace: Ghost(trace),
                    },
                )
            },
            Err(error) => Err(error),
        }
    }
}

/// A repair that has passed the first `N - 1` checks; production's next step
/// is check number `N`: 2 = the pre-publication callback, 3 = the parent
/// revalidation, 4 = the destination name proof, 5 = the pinned read-back.
pub struct RepairCheck<const N: u64> {
    expected: Snap,
    source_permissions: u32,
    #[allow(dead_code)]
    source: Ghost<Snap>,
    #[allow(dead_code)]
    trace: Ghost<Seq<Ev>>,
}

impl<const N: u64> RepairCheck<N> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& 2 <= N <= 5
        &&& checks_done(self.expected, self.source@, self.trace@, N - 1)
    }

    /// The planned destination snapshot.
    pub closed spec fn expected(&self) -> Snap {
        self.expected
    }

    /// The planned source snapshot.
    pub closed spec fn source(&self) -> Snap {
        self.source@
    }

    /// The mode the repair will apply.
    pub closed spec fn source_permissions(&self) -> u32 {
        self.source_permissions
    }

    /// The checks that have passed.
    pub closed spec fn trace(&self) -> Seq<Ev> {
        self.trace@
    }

    fn advance<const M: u64>(self) -> (r: RepairCheck<M>)
        requires
            M == N + 1,
            M <= 5,
        ensures
            r.expected() == self.expected(),
            r.source() == self.source(),
            r.source_permissions() == self.source_permissions(),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost trace = repair_checks().take(N as int);
        RepairCheck {
            expected: self.expected,
            source_permissions: self.source_permissions,
            source: self.source,
            trace: Ghost(trace),
        }
    }

    fn finish(self) -> (r: RepairChmod)
        requires
            N == 5,
        ensures
            r.mode() == self.source_permissions(),
            r.expected() == self.expected(),
            r.source() == self.source(),
    {
        proof {
            use_type_invariant(&self);
            assert(repair_checks().take(5) =~= repair_checks());
        }
        RepairChmod {
            expected: self.expected,
            source_permissions: self.source_permissions,
            source: self.source,
            trace: Ghost(repair_checks()),
        }
    }
}

impl RepairCheck<2> {
    /// Feed the pre-publication callback's result.
    pub fn after_before_publish<E>(self, called: Result<(), E>) -> (r: Result<RepairCheck<3>, E>)
        ensures
            r is Ok <==> called is Ok,
            called matches Err(e) ==> r == Err::<RepairCheck<3>, E>(e),
            r matches Ok(next) ==> next.expected() == self.expected() && next.source()
                == self.source() && next.source_permissions() == self.source_permissions(),
    {
        match called {
            Ok(()) => Ok(self.advance()),
            Err(error) => Err(error),
        }
    }
}

impl RepairCheck<3> {
    /// Feed the destination parent revalidation.
    pub fn after_parent_check<E>(self, checked: Result<(), E>) -> (r: Result<RepairCheck<4>, E>)
        ensures
            r is Ok <==> checked is Ok,
            checked matches Err(e) ==> r == Err::<RepairCheck<4>, E>(e),
            r matches Ok(next) ==> next.expected() == self.expected() && next.source()
                == self.source() && next.source_permissions() == self.source_permissions(),
    {
        match checked {
            Ok(()) => Ok(self.advance()),
            Err(error) => Err(error),
        }
    }
}

impl RepairCheck<4> {
    /// Feed the proof that the destination name resolves to the pinned
    /// descriptor.
    pub fn after_name_check<E>(self, checked: Result<(), E>) -> (r: Result<RepairCheck<5>, E>)
        ensures
            r is Ok <==> checked is Ok,
            checked matches Err(e) ==> r == Err::<RepairCheck<5>, E>(e),
            r matches Ok(next) ==> next.expected() == self.expected() && next.source()
                == self.source() && next.source_permissions() == self.source_permissions(),
    {
        match checked {
            Ok(()) => Ok(self.advance()),
            Err(error) => Err(error),
        }
    }
}

impl RepairCheck<5> {
    /// Feed the pinned descriptor's read-back against the planned snapshot.
    /// Only now is the `fchmod` handed out.
    pub fn after_pinned_check<E>(self, checked: Result<(), E>) -> (r: Result<RepairChmod, E>)
        ensures
            r is Ok <==> checked is Ok,
            checked matches Err(e) ==> r == Err::<RepairChmod, E>(e),
            r matches Ok(next) ==> next.expected() == self.expected() && next.source()
                == self.source() && next.mode() == self.source_permissions(),
    {
        match checked {
            Ok(()) => Ok(self.finish()),
            Err(error) => Err(error),
        }
    }
}

/// Every check passed: production applies `RepairChmod::mode` to the
/// pinned descriptor.
pub struct RepairChmod {
    expected: Snap,
    source_permissions: u32,
    #[allow(dead_code)]
    source: Ghost<Snap>,
    #[allow(dead_code)]
    trace: Ghost<Seq<Ev>>,
}

impl RepairChmod {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& same_content(self.expected, self.source@)
        &&& self.trace@ == repair_checks()
    }

    /// The mode to apply: the source's full `st_mode`.
    pub closed spec fn mode(&self) -> u32 {
        self.source_permissions
    }

    /// The planned destination snapshot.
    pub closed spec fn expected(&self) -> Snap {
        self.expected
    }

    /// The planned source snapshot.
    pub closed spec fn source(&self) -> Snap {
        self.source@
    }

    /// The mode to apply.
    pub fn source_mode(&self) -> (m: u32)
        ensures
            m == self.mode(),
    {
        self.source_permissions
    }

    /// Feed the `fchmod`'s result.
    pub fn after_chmod<E>(self, applied: Result<(), E>) -> (r: Result<RepairReadBack, E>)
        ensures
            r is Ok <==> applied is Ok,
            applied matches Err(e) ==> r == Err::<RepairReadBack, E>(e),
            r matches Ok(next) ==> next.expected() == self.expected() && next.source_permissions()
                == self.mode(),
    {
        proof {
            use_type_invariant(&self);
        }
        match applied {
            Ok(()) => {
                let ghost trace = self.trace@.push(
                    Ev::Fchmod { mode: self.source_permissions, ok: true },
                );
                Ok(
                    RepairReadBack {
                        expected: self.expected,
                        source_permissions: self.source_permissions,
                        source: self.source,
                        trace: Ghost(trace),
                    },
                )
            },
            Err(error) => Err(error),
        }
    }
}

/// The source's mode is applied: production reads the pinned descriptor back.
pub struct RepairReadBack {
    expected: Snap,
    source_permissions: u32,
    #[allow(dead_code)]
    source: Ghost<Snap>,
    #[allow(dead_code)]
    trace: Ghost<Seq<Ev>>,
}

impl RepairReadBack {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& same_content(self.expected, self.source@)
        &&& self.trace@ == repair_checks().push(
            Ev::Fchmod { mode: self.source_permissions, ok: true },
        )
    }

    /// The planned destination snapshot.
    pub closed spec fn expected(&self) -> Snap {
        self.expected
    }

    /// The mode that was applied.
    pub closed spec fn source_permissions(&self) -> u32 {
        self.source_permissions
    }

    /// Feed the read-back (`None` if the descriptor could not be read back as
    /// a regular file).
    pub fn after_read_back(self, observed: Option<Snap>) -> (r: ReadBackStep)
        ensures
            r is Revert <==> repair_verdict_spec(observed, self.expected(), self.source_permissions())
                is ContentChanged,
            r is ModeChanged <==> repair_verdict_spec(observed, self.expected(), self.source_permissions())
                is ModeChanged,
            r is ProveName <==> repair_verdict_spec(observed, self.expected(), self.source_permissions())
                is Repaired,
            r matches ReadBackStep::Revert(revert) ==> revert.mode() == self.expected().permissions
                & 0o7777u32,
    {
        proof {
            use_type_invariant(&self);
        }
        let verdict = repair_verdict(observed, self.expected, self.source_permissions);
        let ghost read_back = Ev::ReadBack {
            content_matches: !(verdict is ContentChanged),
            mode_matches: verdict is Repaired,
        };
        let ghost trace = self.trace@.push(read_back);
        match verdict {
            RepairVerdict::ContentChanged => {
                let mode = revert_mode(self.expected.permissions);
                ReadBackStep::Revert(
                    RepairRevert {
                        mode,
                        expected: Ghost(self.expected),
                        source_permissions: Ghost(self.source_permissions),
                        trace: Ghost(trace),
                    },
                )
            },
            RepairVerdict::ModeChanged => ReadBackStep::ModeChanged,
            RepairVerdict::Repaired => ReadBackStep::ProveName(
                RepairProveName {
                    expected: self.expected,
                    source_permissions: self.source_permissions,
                    source: self.source,
                    trace: Ghost(trace),
                },
            ),
        }
    }
}

/// The verdict [`repair_verdict`] computes, as a specification.
pub open spec fn repair_verdict_spec(observed: Option<Snap>, expected: Snap, source_permissions: u32) -> RepairVerdict {
    if observed is None || !same_content(observed->Some_0, expected) {
        RepairVerdict::ContentChanged
    } else if observed->Some_0.permissions != source_permissions {
        RepairVerdict::ModeChanged
    } else {
        RepairVerdict::Repaired
    }
}

/// What the read-back decided.
pub enum ReadBackStep {
    /// The content changed: put the planned mode back with
    /// `RepairRevert::mode`, then report.
    Revert(RepairRevert),
    /// The content is intact but another writer changed the mode: report,
    /// and touch nothing.
    ModeChanged,
    /// Both halves hold: prove the destination name once more.
    ProveName(RepairProveName),
}

/// A repair whose premise turned out false after the `fchmod`.
pub struct RepairRevert {
    mode: u32,
    #[allow(dead_code)]
    expected: Ghost<Snap>,
    #[allow(dead_code)]
    source_permissions: Ghost<u32>,
    #[allow(dead_code)]
    trace: Ghost<Seq<Ev>>,
}

impl RepairRevert {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& self.mode == self.expected@.permissions & 0o7777u32
        &&& self.mode <= 0o7777u32
        &&& self.trace@ == repair_checks().push(
            Ev::Fchmod { mode: self.source_permissions@, ok: true },
        ).push(Ev::ReadBack { content_matches: false, mode_matches: false })
    }

    /// The mode to put back.
    pub closed spec fn mode(&self) -> u32 {
        self.mode
    }

    /// The trace so far: the checks, the `fchmod`, and a read-back that found
    /// the content changed.
    pub closed spec fn trace(&self) -> Seq<Ev> {
        self.trace@
    }

    /// The mode to put back with the second, final `fchmod`.
    pub fn revert_mode(&self) -> (m: u32)
        ensures
            m == self.mode(),
            m <= 0o7777u32,
    {
        proof {
            use_type_invariant(self);
        }
        self.mode
    }
}

/// A repair whose read-back found both content and mode as intended; the
/// destination name is proved last.
pub struct RepairProveName {
    expected: Snap,
    source_permissions: u32,
    #[allow(dead_code)]
    source: Ghost<Snap>,
    #[allow(dead_code)]
    trace: Ghost<Seq<Ev>>,
}

impl RepairProveName {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& same_content(self.expected, self.source@)
        &&& self.trace@ == repair_checks().push(
            Ev::Fchmod { mode: self.source_permissions, ok: true },
        ).push(Ev::ReadBack { content_matches: true, mode_matches: true })
    }

    /// Feed whether the destination name still resolves to the pinned
    /// descriptor. If another file took the name, nothing is chased or
    /// undone and no repair is reported.
    pub fn after_name_check(self, holds: bool) -> (r: Result<RepairSync, NameTaken>)
        ensures
            r is Ok <==> holds,
    {
        proof {
            use_type_invariant(&self);
        }
        if holds {
            let ghost trace = self.trace@.push(Ev::DestIsPinned { holds: true });
            Ok(
                RepairSync {
                    expected: self.expected,
                    source_permissions: self.source_permissions,
                    source: self.source,
                    trace: Ghost(trace),
                },
            )
        } else {
            Err(NameTaken)
        }
    }
}

/// Another file took the destination name after the repair.
pub struct NameTaken;

/// A repair proved at its name, waiting for the descriptor's `fsync`.
#[allow(dead_code)]
pub struct RepairSync {
    expected: Snap,
    source_permissions: u32,
    #[allow(dead_code)]
    source: Ghost<Snap>,
    #[allow(dead_code)]
    trace: Ghost<Seq<Ev>>,
}

impl RepairSync {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& same_content(self.expected, self.source@)
        &&& self.trace@ == repair_checks().push(
            Ev::Fchmod { mode: self.source_permissions, ok: true },
        ).push(Ev::ReadBack { content_matches: true, mode_matches: true }).push(
            Ev::DestIsPinned { holds: true },
        )
    }

    /// The mode that was applied.
    pub closed spec fn source_permissions(&self) -> u32 {
        self.source_permissions
    }

    /// The planned destination snapshot.
    pub closed spec fn expected(&self) -> Snap {
        self.expected
    }

    /// The planned source snapshot.
    pub closed spec fn source(&self) -> Snap {
        self.source@
    }

    /// Feed the `fsync`'s result. R5: a [`Repaired`] carries exactly the
    /// reported-repair trace, which changes no name.
    pub fn after_sync<E>(self, synced: Result<(), E>) -> (r: Result<Repaired, E>)
        ensures
            r is Ok <==> synced is Ok,
            synced matches Err(e) ==> r == Err::<Repaired, E>(e),
            r matches Ok(repaired) ==> {
                &&& repaired.trace() == repaired_trace(self.source_permissions())
                &&& no_namespace_change(repaired.trace())
                &&& same_content(self.expected(), self.source())
            },
    {
        proof {
            use_type_invariant(&self);
            lemma_repaired_trace_no_namespace_change(self.source_permissions);
        }
        match synced {
            Ok(()) => {
                let ghost trace = self.trace@.push(Ev::Sync { ok: true });
                proof {
                    assert(trace =~= repaired_trace(self.source_permissions));
                }
                Ok(
                    Repaired {
                        expected: Ghost(self.expected),
                        source: self.source,
                        source_permissions: Ghost(self.source_permissions),
                        trace: Ghost(trace),
                    },
                )
            },
            Err(error) => Err(error),
        }
    }
}

/// A repair that may be reported as one.
pub struct Repaired {
    #[allow(dead_code)]
    expected: Ghost<Snap>,
    #[allow(dead_code)]
    source: Ghost<Snap>,
    #[allow(dead_code)]
    source_permissions: Ghost<u32>,
    #[allow(dead_code)]
    trace: Ghost<Seq<Ev>>,
}

impl Repaired {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& same_content(self.expected@, self.source@)
        &&& self.trace@ == repaired_trace(self.source_permissions@)
    }

    /// What the repair asked for.
    pub closed spec fn trace(&self) -> Seq<Ev> {
        self.trace@
    }
}

/// R1 for the reported path: a reported repair changed no name.
pub proof fn lemma_repaired_trace_no_namespace_change(mode: u32)
    ensures
        no_namespace_change(repaired_trace(mode)),
        repaired_trace(mode)[5] == (Ev::Fchmod { mode, ok: true }),
{
    let t = repaired_trace(mode);
    assert(t =~= seq![
        Ev::SourceUnchanged,
        Ev::BeforePublish,
        Ev::ParentRevalidated,
        Ev::DestIsPinned { holds: true },
        Ev::PinnedMatches { holds: true },
        Ev::Fchmod { mode, ok: true },
        Ev::ReadBack { content_matches: true, mode_matches: true },
        Ev::DestIsPinned { holds: true },
        Ev::Sync { ok: true },
    ]);
}

/// R1 for the revert path: a repair that reverts changed no name either.
pub proof fn lemma_revert_trace_no_namespace_change(mode: u32)
    ensures
        no_namespace_change(
            repair_checks().push(Ev::Fchmod { mode, ok: true }).push(
                Ev::ReadBack { content_matches: false, mode_matches: false },
            ),
        ),
{
    let t = repair_checks().push(Ev::Fchmod { mode, ok: true }).push(
        Ev::ReadBack { content_matches: false, mode_matches: false },
    );
    assert(t =~= seq![
        Ev::SourceUnchanged,
        Ev::BeforePublish,
        Ev::ParentRevalidated,
        Ev::DestIsPinned { holds: true },
        Ev::PinnedMatches { holds: true },
        Ev::Fchmod { mode, ok: true },
        Ev::ReadBack { content_matches: false, mode_matches: false },
    ]);
}

} // verus!
