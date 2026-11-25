//! Replacement of a verified destination: an atomic exchange with a proven
//! undo, or — where the filesystem cannot exchange two names — displacement
//! followed by a no-clobber publication.
//!
//! The machine starts once production has proved that the destination name
//! resolves to the pinned destination descriptor (identity `v`) and that the
//! descriptor still reads back as the planned snapshot
//! ([`Exchange::after_destination_verified`]). From there every step is a
//! typestate: production performs exactly the probe or syscall the state
//! names, feeds back the answer, and receives the next state, so no other
//! order compiles. The staging guard (and, on the displacement path, the
//! displaced guard) is passed in by `&mut` and only this machine re-aims or
//! disarms it at the decision points.
//!
//! Every state carries the ghost trace of what was asked so far, with
//! `safe` as part of its type invariant, and every finished record
//! ([`ExchangeDone`], [`UndoDone`], [`DisplaceDone`]) carries `safe` plus a
//! contract on its verdict. Proven, for every path:
//!
//! * **X1** the exchange is issued only right after proving the destination
//!   holds `v` with its planned bytes (and an exchange back only right after
//!   proving the destination holds this run's prepared file `p`);
//! * **X2** `Replaced` on the exchange path means: the exchange succeeded, the
//!   name it swapped out was then proved to hold `v`, the pinned descriptor
//!   still read back as planned, and only then was `v` removed by a proven
//!   unlink — the guard is re-aimed at `v` in exactly that branch;
//! * **X3** the undo touches the destination only after proving it still holds
//!   `p`, and unlinks the staging name only after proving it holds `p` again
//!   (through the guard's own identity); every failed proof disarms the guard;
//! * **X4** the outgoing inode of a lost race is never the target of an unlink
//!   (a corollary of `proven_unlinks`: it is neither the run's own file nor
//!   a verified `v`);
//! * **X5** an undo reports the clean "left untouched" verdict only if the
//!   staging name was then removed by a proven unlink;
//! * **X6** an exchange error is either "unsupported", which leads to a fresh
//!   re-proof and the displacement path, or ends the replacement with no
//!   further action;
//! * **D1** the destination is moved aside only after an exchange was reported
//!   unsupported and the destination was re-proved;
//! * **D2** the displaced name is unlinked only for `v`, only after the moved
//!   entry was identified as `v` and the pinned descriptor read back as
//!   planned, and only after the replacement was published;
//! * **D3/D4** anything else that was displaced is put back with a no-clobber
//!   restore, never published over and never unlinked except by that
//!   restore's own link cleanup; a failed publication is followed by exactly
//!   one restore;
//! * **D5** `RestoredUntouched` (and a restore after a failed publication that
//!   left nothing behind) means the displaced name was consumed;
//! * **D7** `Replaced` on the displacement path means the displaced entry was
//!   verified as `v`, the replacement was published, and `v` was removed by a
//!   proven unlink.
//!
//! Trust: production proves the two facts [`Exchange::after_destination_verified`]
//! and [`Reprove::reproved`] stand for before calling them, runs every probe
//! against the identity the state hands out, and feeds back true answers. The
//! displaced name is a fresh 128-bit nonce (the A-fresh axiom of
//! [`crate::publish::model`]).

#[allow(unused_imports)]
use vstd::prelude::*;

use crate::publish::noreplace::{Attempt, NoReplace, NoReplaceDone};
#[cfg(verus_keep_ghost)]
use crate::publish::noreplace::{consumed, published, res_of};
use crate::publish::ordering::Ready;
use crate::publish::removal::{GuardState, ProvenRemoval, RemovalDone};
#[allow(unused_imports)]
use crate::publish::trace::{Ev, Identity, Ino, Leftover, Name, Published, Removal, Res, Run};
#[cfg(verus_keep_ghost)]
use crate::publish::trace::{
    entitled, ino_of, lemma_safe_append_noreplace, lemma_safe_append_removal, lemma_safe_push,
    lemma_safe_push_observation, lemma_verified_at_append, moved_then_verified, safe, verified_at,
};

verus! {

/// The two observations every replacement starts from: the destination
/// resolves to `v`, and the pinned descriptor reads back as planned.
pub open spec fn verified_prefix(v: Ino) -> Seq<Ev> {
    seq![Ev::Probe { name: Name::Dest, ino: v, holds: true }, Ev::PinnedMatches { holds: true }]
}

/// The run constants of a replacement.
pub open spec fn run_of(own: Option<Ino>, v: Ino, p: Ino) -> Run {
    Run { own, verified: Some(v), prepared: Some(p) }
}

/// The bookkeeping every replacement state carries: the run, and a safe
/// trace.
pub open spec fn state_ok<I: Identity>(v: I, p: I, trace: Seq<Ev>, run: Run) -> bool {
    &&& run == run_of(run.own, v.ino(), p.ino())
    &&& safe(trace, run)
}

/// `t` begins with the verified prefix for `v`.
pub open spec fn starts_verified(t: Seq<Ev>, v: Ino) -> bool {
    t.len() >= 2 && t.take(2) == verified_prefix(v)
}

proof fn lemma_safe_empty(run: Run)
    ensures
        safe(Seq::<Ev>::empty(), run),
{
}

proof fn lemma_take_push(t: Seq<Ev>, e: Ev, k: int)
    requires
        0 <= k <= t.len(),
    ensures
        t.push(e).take(k) == t.take(k),
{
    assert(t.push(e).take(k) =~= t.take(k));
}

proof fn lemma_take_append(a: Seq<Ev>, b: Seq<Ev>, k: int)
    requires
        0 <= k <= a.len(),
    ensures
        (a + b).take(k) == a.take(k),
{
    assert((a + b).take(k) =~= a.take(k));
}

proof fn lemma_contains_append(a: Seq<Ev>, b: Seq<Ev>, e: Ev)
    ensures
        a.contains(e) ==> (a + b).contains(e),
        b.contains(e) ==> (a + b).contains(e),
{
    if a.contains(e) {
        let i = choose|i: int| 0 <= i < a.len() && a[i] == e;
        assert((a + b)[i] == e);
    }
    if b.contains(e) {
        let i = choose|i: int| 0 <= i < b.len() && b[i] == e;
        assert((a + b)[a.len() + i] == e);
    }
}

proof fn lemma_published_append(a: Seq<Ev>, b: Seq<Ev>, n: Name)
    ensures
        published(b, n) ==> published(a + b, n),
        consumed(b, n) ==> consumed(a + b, n),
        published(a, n) ==> published(a + b, n),
{
    lemma_contains_append(a, b, Ev::RenameNoReplace { from: n, res: Res::Ok });
    lemma_contains_append(a, b, Ev::Link { from: n, ok: true });
    lemma_contains_append(a, b, Ev::Unlink { name: n, ok: true });
}

// ---------------------------------------------------------------------------
// The exchange.
// ---------------------------------------------------------------------------

/// A replacement whose destination has just been proved: production's next
/// step is `renameat2(Staging, Dest, RENAME_EXCHANGE)`.
#[allow(dead_code)]
pub struct Exchange<I: Identity> {
    v: I,
    p: I,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> Exchange<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.trace@, self.run@)
        &&& self.trace@ == verified_prefix(self.v.ino())
    }
}

impl<I: Identity> Exchange<I> {
    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// Start the replacement of a run whose publication checks have all
    /// passed (`_ready`).
    ///
    /// Production calls this only after proving, in this order, that the
    /// destination name resolves to the pinned destination descriptor whose
    /// identity is `verified`, and that the descriptor still reads back as
    /// exactly the planned snapshot. `prepared` is the identity of this run's
    /// staging file and `guard` is its staging guard.
    pub fn after_destination_verified(
        _ready: Ready,
        _guard: &GuardState<I>,
        verified: I,
        prepared: I,
    ) -> (r: Self)
        ensures
            r.run() == run_of(_guard.ident(), verified.ino(), prepared.ino()),
    {
        let ghost run = run_of(_guard.ident(), verified.ino(), prepared.ino());
        let ghost trace = verified_prefix(verified.ino());
        proof {
            lemma_safe_empty(run);
            let e0 = Ev::Probe { name: Name::Dest, ino: verified.ino(), holds: true };
            let e1 = Ev::PinnedMatches { holds: true };
            lemma_safe_push_observation(Seq::empty(), e0, run);
            lemma_safe_push_observation(seq![e0], e1, run);
            assert(Seq::<Ev>::empty().push(e0) =~= seq![e0]);
            assert(seq![e0].push(e1) =~= trace);
        }
        Exchange { v: verified, p: prepared, trace: Ghost(trace), run: Ghost(run) }
    }

    /// Feed the exchange's answer.
    pub fn after_exchange<E>(self, exchanged: Attempt<E>) -> (r: Exchanged<I, E>)
        ensures
            r is Check <==> exchanged is Done,
            r is Displace <==> exchanged is Unsupported,
            r matches Exchanged::Failed(e) ==> exchanged == Attempt::<E>::Failed(e),
            r matches Exchanged::Check(c) ==> c.run() == self.run(),
            r matches Exchanged::Displace(d) ==> d.run() == self.run(),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost t = self.trace@;
        let ghost e = Ev::Exchange { res: res_of(exchanged) };
        let ghost u = t.push(e);
        proof {
            lemma_safe_push(t, e, self.run@);
        }
        match exchanged {
            Attempt::Done => Exchanged::Check(
                SwapProbe { v: self.v, p: self.p, trace: Ghost(u), run: self.run },
            ),
            Attempt::Unsupported => Exchanged::Displace(
                Reprove { v: self.v, p: self.p, trace: Ghost(u), run: self.run },
            ),
            Attempt::Failed(error) => Exchanged::Failed(error),
        }
    }
}

/// After the exchange.
pub enum Exchanged<I: Identity, E> {
    /// It succeeded: prove what the staging name now holds.
    Check(SwapProbe<I>),
    /// Exchange is unsupported here: re-prove the destination, then
    /// displace.
    Displace(Reprove<I>),
    /// It failed: nothing was exchanged and nothing more may be done.
    Failed(E),
}

/// An exchange that succeeded, waiting for the proof that the staging name
/// now holds the planned inode.
#[allow(dead_code)]
pub struct SwapProbe<I: Identity> {
    v: I,
    p: I,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> SwapProbe<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.trace@, self.run@)
        &&& self.trace@ == verified_prefix(self.v.ino()).push(Ev::Exchange { res: Res::Ok })
    }
}

impl<I: Identity> SwapProbe<I> {
    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// The identity to probe the staging name against: the planned inode.
    pub fn expected(&self) -> (r: I)
        ensures
            Some(r.ino()) == self.run().verified,
    {
        proof {
            use_type_invariant(self);
        }
        self.v
    }

    /// Feed whether the staging name holds the planned inode.
    pub fn after_probe(self, holds: bool) -> (r: SwapProbed<I>)
        ensures
            r is Pinned <==> holds,
            r matches SwapProbed::Pinned(s) ==> s.run() == self.run(),
            r matches SwapProbed::Undo(u) ==> u.run() == self.run(),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost t = self.trace@;
        let ghost u = t.push(Ev::Probe { name: Name::Staging, ino: self.v.ino(), holds });
        proof {
            lemma_safe_push_observation(t, u.last(), self.run@);
            assert(u == t.push(u.last()));
        }
        if holds {
            SwapProbed::Pinned(SwapPinned { v: self.v, p: self.p, trace: Ghost(u), run: self.run })
        } else {
            SwapProbed::Undo(Undo { v: self.v, p: self.p, trace: Ghost(u), run: self.run })
        }
    }
}

/// After the staging-name proof.
pub enum SwapProbed<I: Identity> {
    /// The staging name holds the planned inode: check its bytes.
    Pinned(SwapPinned<I>),
    /// Another writer's file was swapped out: undo.
    Undo(Undo<I>),
}

/// The swapped-out name holds the planned inode; waiting for the read-back
/// of the pinned descriptor.
#[allow(dead_code)]
pub struct SwapPinned<I: Identity> {
    v: I,
    p: I,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> SwapPinned<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.trace@, self.run@)
        &&& self.trace@ == verified_prefix(self.v.ino()).push(Ev::Exchange { res: Res::Ok }).push(
            Ev::Probe { name: Name::Staging, ino: self.v.ino(), holds: true },
        )
    }
}

impl<I: Identity> SwapPinned<I> {
    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// Feed whether the pinned descriptor still reads back as planned.
    ///
    /// Only if it does is the swapped-out file the one that was planned
    /// against: the staging guard is then re-aimed at it and its proven
    /// removal handed out. Otherwise the exchange is undone and the guard is
    /// left exactly as it was.
    pub fn after_pinned(self, guard: &mut GuardState<I>, matches: bool) -> (r: SwapChecked<I>)
        requires
            old(guard).name() == Name::Staging,
            old(guard).ident() == self.run().own,
        ensures
            r is Remove <==> matches,
            r matches SwapChecked::Remove(x, removal) ==> {
                &&& x.run() == self.run()
                &&& removal.name() == Name::Staging
                &&& removal.ident() == self.run().verified
                &&& final(guard).ident() == self.run().verified
                &&& !final(guard).is_armed()
                &&& final(guard).name() == Name::Staging
            },
            r matches SwapChecked::Undo(u) ==> u.run() == self.run() && *final(guard) == *old(guard),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost t = self.trace@;
        let ghost u = t.push(Ev::PinnedMatches { holds: matches });
        proof {
            lemma_safe_push_observation(t, u.last(), self.run@);
            assert(u == t.push(u.last()));
        }
        if matches {
            proof {
                assert(moved_then_verified(u, 2, Name::Staging, self.v.ino()));
            }
            guard.now_holds(self.v);
            let removal = guard.take_removal();
            SwapChecked::Remove(
                ExchangeRemove { v: self.v, p: self.p, trace: Ghost(u), run: self.run },
                removal,
            )
        } else {
            SwapChecked::Undo(Undo { v: self.v, p: self.p, trace: Ghost(u), run: self.run })
        }
    }
}

/// After the read-back.
pub enum SwapChecked<I: Identity> {
    /// The planned file was swapped out intact: remove it, then report.
    Remove(ExchangeRemove<I>, ProvenRemoval<I>),
    /// It was not: undo.
    Undo(Undo<I>),
}

/// The replacement stands; waiting for the proven removal of the planned
/// file from the staging name.
#[allow(dead_code)]
pub struct ExchangeRemove<I: Identity> {
    v: I,
    p: I,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> ExchangeRemove<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.trace@, self.run@)
        &&& verified_at(self.trace@, Name::Staging, self.v.ino())
    }
}

impl<I: Identity> ExchangeRemove<I> {
    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// Feed the removal handed out with this state.
    pub fn after_removal<E>(self, removal: RemovalDone<E>) -> (d: ExchangeDone<E>)
        requires
            removal.name() == Name::Staging,
            removal.ident() == self.run().verified,
        ensures
            d.run() == self.run(),
            d.result() is Ok <==> removal.outcome() is Removed,
            removal.outcome() matches Removal::Left(l) ==> d.result() == Err::<(), Leftover<E>>(l),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost t = self.trace@;
        let ghost b = removal.trace();
        let outcome = removal.into_outcome();
        let ghost u = t + b;
        proof {
            lemma_safe_append_removal(t, b, Name::Staging, self.run@.verified, outcome, self.run@);
            lemma_verified_at_append(t, b, Name::Staging, self.v.ino());
        }
        let result = match outcome {
            Removal::Removed => Ok(()),
            Removal::Left(left) => Err(left),
        };
        ExchangeDone { result, trace: Ghost(u), run: self.run }
    }
}

/// A finished exchange replacement: `Ok` when the planned file was removed
/// from the staging name, otherwise why it is still there.
#[allow(dead_code)]
pub struct ExchangeDone<E> {
    result: Result<(), Leftover<E>>,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

/// X2: the exchange happened, the swapped-out name was verified to hold the
/// planned inode, and the trace ends with that inode's successful removal.
pub open spec fn exchanged_and_removed(t: Seq<Ev>, v: Option<Ino>) -> bool {
    &&& v is Some
    &&& verified_at(t, Name::Staging, v->Some_0)
    &&& t.len() > 0
    &&& t.last() == (Ev::Unlink { name: Name::Staging, ok: true })
}

impl<E> ExchangeDone<E> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& safe(self.trace@, self.run@)
        &&& self.run@.verified is Some
        &&& verified_at(self.trace@, Name::Staging, self.run@.verified->Some_0)
        &&& (self.result is Ok ==> exchanged_and_removed(self.trace@, self.run@.verified))
    }

    /// The run this replacement acted for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// What the replacement asked for.
    pub closed spec fn trace(&self) -> Seq<Ev> {
        self.trace@
    }

    /// The verdict.
    pub closed spec fn result(&self) -> Result<(), Leftover<E>> {
        self.result
    }

    /// The verdict, with its contract.
    pub fn into_result(self) -> (r: Result<(), Leftover<E>>)
        ensures
            r == self.result(),
            safe(self.trace(), self.run()),
            r is Ok ==> exchanged_and_removed(self.trace(), self.run().verified),
    {
        proof {
            use_type_invariant(&self);
        }
        self.result
    }
}

// ---------------------------------------------------------------------------
// The undo.
// ---------------------------------------------------------------------------

/// An exchange that published over a file this run never verified; production
/// runs its pre-undo hook, then probes whether the destination still holds
/// this run's prepared file.
#[allow(dead_code)]
pub struct Undo<I: Identity> {
    v: I,
    p: I,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> Undo<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        state_ok(self.v, self.p, self.trace@, self.run@)
    }
}

impl<I: Identity> Undo<I> {
    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// The identity to probe the destination against: this run's prepared
    /// file.
    pub fn prepared(&self) -> (r: I)
        ensures
            Some(r.ino()) == self.run().prepared,
    {
        proof {
            use_type_invariant(self);
        }
        self.p
    }

    /// Feed whether the destination still holds this run's prepared file.
    /// If not, a third writer has taken it: nothing is swapped back and the
    /// guard is disarmed, so the swapped-out file is kept.
    pub fn after_destination_probe<E>(self, guard: &mut GuardState<I>, holds: bool) -> (r:
        UndoProbed<I, E>)
        requires
            old(guard).name() == Name::Staging,
        ensures
            r is ExchangeBack <==> holds,
            r matches UndoProbed::Done(d) ==> d.run() == self.run() && d.verdict()
                == UndoVerdict::<E>::ChangedTwice && !final(guard).is_armed(),
            r matches UndoProbed::ExchangeBack(x) ==> x.run() == self.run() && *final(guard)
                == *old(guard),
            final(guard).name() == old(guard).name(),
            final(guard).ident() == old(guard).ident(),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost t = self.trace@;
        let ghost u = t.push(Ev::Probe { name: Name::Dest, ino: self.p.ino(), holds });
        proof {
            lemma_safe_push_observation(t, u.last(), self.run@);
            assert(u == t.push(u.last()));
        }
        if holds {
            UndoProbed::ExchangeBack(
                UndoExchangeBack { v: self.v, p: self.p, trace: Ghost(u), run: self.run },
            )
        } else {
            guard.disarm();
            UndoProbed::Done(
                UndoDone { verdict: UndoVerdict::ChangedTwice, trace: Ghost(u), run: self.run },
            )
        }
    }
}

/// After the destination probe of an undo.
pub enum UndoProbed<I: Identity, E> {
    /// The undo was refused.
    Done(UndoDone<E>),
    /// The destination holds this run's file: exchange the names back.
    ExchangeBack(UndoExchangeBack<I>),
}

/// An undo whose destination was just proved to hold this run's file.
#[allow(dead_code)]
pub struct UndoExchangeBack<I: Identity> {
    v: I,
    p: I,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> UndoExchangeBack<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.trace@, self.run@)
        &&& self.trace@.len() > 0
        &&& self.trace@.last() == (Ev::Probe { name: Name::Dest, ino: self.p.ino(), holds: true })
    }
}

impl<I: Identity> UndoExchangeBack<I> {
    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// Feed the answer to exchanging the names back. A failure leaves the
    /// swapped-out file under the staging name; the guard is disarmed so it
    /// is never removed.
    pub fn after_exchange_back<E>(self, guard: &mut GuardState<I>, exchanged: Result<(), E>) -> (r:
        UndoExchangedBack<I, E>)
        requires
            old(guard).name() == Name::Staging,
        ensures
            r is ProbeStaging <==> exchanged is Ok,
            r matches UndoExchangedBack::Done(d) ==> d.run() == self.run() && (exchanged matches Err(
                e,
            ) && d.verdict() == UndoVerdict::<E>::UndoFailed(e)) && !final(guard).is_armed(),
            r matches UndoExchangedBack::ProbeStaging(x) ==> x.run() == self.run() && *final(guard)
                == *old(guard),
            final(guard).name() == old(guard).name(),
            final(guard).ident() == old(guard).ident(),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost t = self.trace@;
        let ghost e = Ev::Exchange { res: if exchanged is Ok { Res::Ok } else { Res::Failed } };
        let ghost u = t.push(e);
        proof {
            lemma_safe_push(t, e, self.run@);
        }
        match exchanged {
            Ok(()) => UndoExchangedBack::ProbeStaging(
                UndoStagingProbe { v: self.v, p: self.p, trace: Ghost(u), run: self.run },
            ),
            Err(error) => {
                guard.disarm();
                UndoExchangedBack::Done(
                    UndoDone {
                        verdict: UndoVerdict::UndoFailed(error),
                        trace: Ghost(u),
                        run: self.run,
                    },
                )
            },
        }
    }
}

/// After the exchange back.
pub enum UndoExchangedBack<I: Identity, E> {
    /// The undo failed.
    Done(UndoDone<E>),
    /// The names are swapped back: prove the staging name holds this run's
    /// file again.
    ProbeStaging(UndoStagingProbe<I>),
}

/// An undo that swapped back, waiting for the staging-name proof.
#[allow(dead_code)]
pub struct UndoStagingProbe<I: Identity> {
    v: I,
    p: I,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> UndoStagingProbe<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        state_ok(self.v, self.p, self.trace@, self.run@)
    }
}

impl<I: Identity> UndoStagingProbe<I> {
    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// The identity to probe the staging name against: this run's prepared
    /// file.
    pub fn prepared(&self) -> (r: I)
        ensures
            Some(r.ino()) == self.run().prepared,
    {
        proof {
            use_type_invariant(self);
        }
        self.p
    }

    /// Feed whether the staging name holds this run's file again. Only then
    /// is the guard's proven removal handed out; otherwise the guard is
    /// disarmed and the name left alone.
    pub fn after_staging_probe<E>(self, guard: &mut GuardState<I>, holds: bool) -> (r:
        UndoStagingProbed<I, E>)
        requires
            old(guard).name() == Name::Staging,
            old(guard).ident() == self.run().own,
        ensures
            r is Remove <==> holds,
            r matches UndoStagingProbed::Done(d) ==> d.run() == self.run() && d.verdict()
                == UndoVerdict::<E>::ReplacementNotHeld && !final(guard).is_armed(),
            r matches UndoStagingProbed::Remove(x, removal) ==> {
                &&& x.run() == self.run()
                &&& removal.name() == Name::Staging
                &&& removal.ident() == self.run().own
                &&& !final(guard).is_armed()
            },
            final(guard).name() == old(guard).name(),
            final(guard).ident() == old(guard).ident(),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost t = self.trace@;
        let ghost u = t.push(Ev::Probe { name: Name::Staging, ino: self.p.ino(), holds });
        proof {
            lemma_safe_push_observation(t, u.last(), self.run@);
            assert(u == t.push(u.last()));
        }
        if holds {
            let removal = guard.take_removal();
            UndoStagingProbed::Remove(
                UndoRemove { v: self.v, p: self.p, trace: Ghost(u), run: self.run },
                removal,
            )
        } else {
            guard.disarm();
            UndoStagingProbed::Done(
                UndoDone {
                    verdict: UndoVerdict::ReplacementNotHeld,
                    trace: Ghost(u),
                    run: self.run,
                },
            )
        }
    }
}

/// After the staging-name proof of an undo.
pub enum UndoStagingProbed<I: Identity, E> {
    /// The staging name was taken; it is left alone.
    Done(UndoDone<E>),
    /// Remove this run's prepared file through the staging guard.
    Remove(UndoRemove<I>, ProvenRemoval<I>),
}

/// An undo waiting for its final proven removal.
#[allow(dead_code)]
pub struct UndoRemove<I: Identity> {
    v: I,
    p: I,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> UndoRemove<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        state_ok(self.v, self.p, self.trace@, self.run@)
    }
}

impl<I: Identity> UndoRemove<I> {
    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// Feed the removal handed out with this state.
    pub fn after_removal<E>(self, removal: RemovalDone<E>) -> (d: UndoDone<E>)
        requires
            removal.name() == Name::Staging,
            removal.ident() == self.run().own,
        ensures
            d.run() == self.run(),
            d.verdict() is Undone <==> removal.outcome() is Removed,
            removal.outcome() matches Removal::Left(l) ==> d.verdict()
                == UndoVerdict::ReplacementNotRemoved(l),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost t = self.trace@;
        let ghost b = removal.trace();
        let outcome = removal.into_outcome();
        proof {
            lemma_safe_append_removal(t, b, Name::Staging, self.run@.own, outcome, self.run@);
        }
        let verdict = match outcome {
            Removal::Removed => UndoVerdict::Undone,
            Removal::Left(left) => UndoVerdict::ReplacementNotRemoved(left),
        };
        UndoDone { verdict, trace: Ghost(t + b), run: self.run }
    }
}

/// How an undo ended. Every verdict is a per-file failure.
pub enum UndoVerdict<E> {
    /// The destination no longer held this run's file: nothing was swapped
    /// back and the swapped-out file is kept under the staging name.
    ChangedTwice,
    /// Exchanging back failed: the swapped-out file is kept under the staging
    /// name.
    UndoFailed(E),
    /// Swapped back, but the staging name no longer holds this run's file; it
    /// is left alone.
    ReplacementNotHeld,
    /// Swapped back and this run's file removed: the destination was left
    /// untouched.
    Undone,
    /// Swapped back, but this run's file could not be removed.
    ReplacementNotRemoved(Leftover<E>),
}

/// A finished undo.
#[allow(dead_code)]
pub struct UndoDone<E> {
    verdict: UndoVerdict<E>,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<E> UndoDone<E> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& safe(self.trace@, self.run@)
        &&& (self.verdict is Undone ==> self.trace@.len() > 0 && self.trace@.last() == (Ev::Unlink {
            name: Name::Staging,
            ok: true,
        }))
    }

    /// The run this replacement acted for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// What the replacement asked for.
    pub closed spec fn trace(&self) -> Seq<Ev> {
        self.trace@
    }

    /// The verdict.
    pub closed spec fn verdict(&self) -> UndoVerdict<E> {
        self.verdict
    }

    /// The verdict, with its contract.
    pub fn into_verdict(self) -> (r: UndoVerdict<E>)
        ensures
            r == self.verdict(),
            safe(self.trace(), self.run()),
            r is Undone ==> self.trace().len() > 0 && self.trace().last() == (Ev::Unlink {
                name: Name::Staging,
                ok: true,
            }),
    {
        proof {
            use_type_invariant(&self);
        }
        self.verdict
    }
}

// ---------------------------------------------------------------------------
// Displacement.
// ---------------------------------------------------------------------------

/// An exchange reported unsupported: production re-proves the destination
/// name and the pinned descriptor, then moves the destination aside.
#[allow(dead_code)]
pub struct Reprove<I: Identity> {
    v: I,
    p: I,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> Reprove<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.trace@, self.run@)
        &&& self.trace@ == verified_prefix(self.v.ino()).push(
            Ev::Exchange { res: Res::Unsupported },
        )
    }
}

impl<I: Identity> Reprove<I> {
    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// Production calls this only after proving, again and in this order,
    /// that the destination name resolves to the pinned descriptor and that
    /// the descriptor reads back as planned.
    pub fn reproved(self) -> (r: Aside<I>)
        ensures
            r.run() == self.run(),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost t = self.trace@;
        let ghost e0 = Ev::Probe { name: Name::Dest, ino: self.v.ino(), holds: true };
        let ghost e1 = Ev::PinnedMatches { holds: true };
        proof {
            lemma_safe_push_observation(t, e0, self.run@);
            lemma_safe_push_observation(t.push(e0), e1, self.run@);
        }
        Aside { v: self.v, p: self.p, trace: Ghost(t.push(e0).push(e1)), run: self.run }
    }
}

/// A displacement whose destination was re-proved: production generates a
/// fresh displaced name and moves the destination onto it.
#[allow(dead_code)]
pub struct Aside<I: Identity> {
    v: I,
    p: I,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

/// The trace of a displacement right before the destination is moved aside.
pub open spec fn aside_prefix(v: Ino) -> Seq<Ev> {
    verified_prefix(v).push(Ev::Exchange { res: Res::Unsupported }).push(
        Ev::Probe { name: Name::Dest, ino: v, holds: true },
    ).push(Ev::PinnedMatches { holds: true })
}

impl<I: Identity> Aside<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.trace@, self.run@)
        &&& self.trace@ == aside_prefix(self.v.ino())
    }
}

impl<I: Identity> Aside<I> {
    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// Feed the answer to `renameat(Dest, Displaced)`. On success the
    /// displaced name gets a guard entitled to the planned inode.
    pub fn after_rename_aside<E>(self, moved: Result<(), E>) -> (r: Result<
        (Lstat<I>, GuardState<I>),
        E,
    >)
        ensures
            r is Ok <==> moved is Ok,
            moved matches Err(e) ==> r == Err::<(Lstat<I>, GuardState<I>), E>(e),
            r matches Ok((l, g)) ==> {
                &&& l.run() == self.run()
                &&& g.name() == Name::Displaced
                &&& g.ident() == self.run().verified
                &&& g.is_armed()
            },
    {
        proof {
            use_type_invariant(&self);
        }
        match moved {
            Ok(()) => {
                let ghost t = self.trace@;
                let ghost e = Ev::RenameAside { ok: true };
                proof {
                    lemma_safe_push(t, e, self.run@);
                }
                let guard = GuardState::displaced(self.v);
                Ok((Lstat { v: self.v, p: self.p, trace: Ghost(t.push(e)), run: self.run }, guard))
            },
            Err(error) => Err(error),
        }
    }
}

/// The destination was moved aside: production identifies what moved, by
/// reading the displaced name's identity without following a symlink.
#[allow(dead_code)]
pub struct Lstat<I: Identity> {
    v: I,
    p: I,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> Lstat<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.trace@, self.run@)
        &&& self.trace@ == aside_prefix(self.v.ino()).push(Ev::RenameAside { ok: true })
    }
}

impl<I: Identity> Lstat<I> {
    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// Feed the displaced name's identity. Anything but the planned inode is
    /// put back; an unreadable name is kept and reported.
    pub fn after_lstat<E>(self, displaced: &mut GuardState<I>, found: Result<I, E>) -> (r: Lstated<
        I,
        E,
    >)
        requires
            old(displaced).name() == Name::Displaced,
        ensures
            r matches Lstated::Done(d) ==> {
                &&& d.run() == self.run()
                &&& (found matches Err(e) && d.verdict() == DisplaceVerdict::<E>::ReadBackFailed(e))
                &&& !final(displaced).is_armed()
            },
            r matches Lstated::Restore(x, restore) ==> {
                &&& found matches Ok(d)
                &&& d.ino() != self.run().verified->Some_0
                &&& x.run() == self.run()
                &&& restore.from() == Name::Displaced
                &&& restore.ident() == Some(d.ino())
                &&& x.found() == d.ino()
                &&& !final(displaced).is_armed()
            },
            r matches Lstated::Pinned(x) ==> {
                &&& found matches Ok(d)
                &&& Some(d.ino()) == self.run().verified
                &&& x.run() == self.run()
                &&& *final(displaced) == *old(displaced)
            },
            final(displaced).name() == old(displaced).name(),
            final(displaced).ident() == old(displaced).ident(),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost t = self.trace@;
        match found {
            Err(error) => {
                let ghost u = t.push(Ev::Lstat { name: Name::Displaced, found: None });
                proof {
                    lemma_safe_push_observation(t, u.last(), self.run@);
                    assert(u == t.push(u.last()));
                }
                displaced.disarm();
                Lstated::Done(
                    DisplaceDone {
                        verdict: DisplaceVerdict::ReadBackFailed(error),
                        trace: Ghost(u),
                        run: self.run,
                    },
                )
            },
            Ok(d) => {
                let ghost u = t.push(Ev::Lstat { name: Name::Displaced, found: Some(d.ino()) });
                proof {
                    lemma_safe_push_observation(t, u.last(), self.run@);
                    assert(u == t.push(u.last()));
                }
                if d.same_inode(&self.v) {
                    Lstated::Pinned(
                        DisplacedPinned { v: self.v, p: self.p, d, trace: Ghost(u), run: self.run },
                    )
                } else {
                    displaced.disarm();
                    Lstated::Restore(
                        Restore { v: self.v, p: self.p, d, trace: Ghost(u), run: self.run },
                        NoReplace::new(Some(d), Ghost(Name::Displaced)),
                    )
                }
            },
        }
    }
}

/// After identifying the displaced entry.
pub enum Lstated<I: Identity, E> {
    /// The displaced name could not be read back.
    Done(DisplaceDone<E>),
    /// Something other than the planned inode was displaced: put it back
    /// with a no-clobber publication, then report.
    Restore(Restore<I>, NoReplace<I>),
    /// The planned inode was displaced: check its bytes.
    Pinned(DisplacedPinned<I>),
}

/// The planned inode was displaced; waiting for the read-back of the pinned
/// descriptor.
#[allow(dead_code)]
pub struct DisplacedPinned<I: Identity> {
    v: I,
    p: I,
    d: I,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> DisplacedPinned<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.trace@, self.run@)
        &&& self.d.ino() == self.v.ino()
        &&& self.trace@ == aside_prefix(self.v.ino()).push(Ev::RenameAside { ok: true }).push(
            Ev::Lstat { name: Name::Displaced, found: Some(self.v.ino()) },
        )
    }
}

impl<I: Identity> DisplacedPinned<I> {
    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// Feed whether the pinned descriptor still reads back as planned. If it
    /// does, the replacement is published next (entitled to clean up the
    /// staging guard's identity); otherwise the displaced file is put back.
    pub fn after_pinned(self, guard: &GuardState<I>, displaced: &mut GuardState<I>, matches: bool) -> (r:
        DisplacedChecked<I>)
        requires
            guard.name() == Name::Staging,
            guard.ident() == self.run().own,
            old(displaced).name() == Name::Displaced,
        ensures
            r is Publish <==> matches,
            r matches DisplacedChecked::Publish(x, publish) ==> {
                &&& x.run() == self.run()
                &&& publish.from() == Name::Staging
                &&& publish.ident() == self.run().own
                &&& *final(displaced) == *old(displaced)
            },
            r matches DisplacedChecked::Restore(x, restore) ==> {
                &&& x.run() == self.run()
                &&& restore.from() == Name::Displaced
                &&& restore.ident() == self.run().verified
                &&& x.found() == self.run().verified->Some_0
                &&& !final(displaced).is_armed()
            },
            final(displaced).name() == old(displaced).name(),
            final(displaced).ident() == old(displaced).ident(),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost t = self.trace@;
        let ghost u = t.push(Ev::PinnedMatches { holds: matches });
        proof {
            lemma_safe_push_observation(t, u.last(), self.run@);
            assert(u == t.push(u.last()));
        }
        if matches {
            proof {
                assert(moved_then_verified(u, 5, Name::Displaced, self.v.ino()));
            }
            DisplacedChecked::Publish(
                Publish { v: self.v, p: self.p, trace: Ghost(u), run: self.run },
                NoReplace::new(guard.identity(), Ghost(Name::Staging)),
            )
        } else {
            displaced.disarm();
            DisplacedChecked::Restore(
                Restore { v: self.v, p: self.p, d: self.d, trace: Ghost(u), run: self.run },
                NoReplace::new(Some(self.d), Ghost(Name::Displaced)),
            )
        }
    }
}

/// After checking the displaced planned inode's bytes.
pub enum DisplacedChecked<I: Identity> {
    /// It is intact: publish the replacement onto the vacated name.
    Publish(Publish<I>, NoReplace<I>),
    /// It was rewritten in place: put it back.
    Restore(Restore<I>, NoReplace<I>),
}

/// A displaced file that is not the planned one (or not with the planned
/// bytes) is being put back; waiting for that publication.
#[allow(dead_code)]
pub struct Restore<I: Identity> {
    v: I,
    p: I,
    d: I,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> Restore<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        state_ok(self.v, self.p, self.trace@, self.run@)
    }
}

impl<I: Identity> Restore<I> {
    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// The identity of the displaced entry being put back.
    pub closed spec fn found(&self) -> Ino {
        self.d.ino()
    }

    /// Feed the restore handed out with this state.
    pub fn after_restore<E>(self, restore: NoReplaceDone<E>) -> (d: DisplaceDone<E>)
        requires
            restore.from() == Name::Displaced,
            restore.ident() == Some(self.found()),
        ensures
            d.run() == self.run(),
            restore.result() == Ok::<Published<E>, E>(Published::Consumed) ==> d.verdict()
                == DisplaceVerdict::<E>::RestoredUntouched,
            restore.result() matches Ok(Published::LeftBehind(l)) ==> d.verdict()
                == DisplaceVerdict::RestoredLeftBehind(l),
            restore.result() matches Err(e) ==> d.verdict() == DisplaceVerdict::RestoreFailed(e),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost t = self.trace@;
        let ghost b = restore.trace();
        let ghost ident = restore.ident();
        let result = restore.into_result();
        proof {
            lemma_safe_append_noreplace(t, b, Name::Displaced, ident, result, self.run@);
            crate::publish::noreplace::theorem_noreplace(b, Name::Displaced, ident, result);
            lemma_published_append(t, b, Name::Displaced);
        }
        let verdict = match result {
            Ok(Published::Consumed) => DisplaceVerdict::RestoredUntouched,
            Ok(Published::LeftBehind(left)) => DisplaceVerdict::RestoredLeftBehind(left),
            Err(error) => DisplaceVerdict::RestoreFailed(error),
        };
        DisplaceDone { verdict, trace: Ghost(t + b), run: self.run }
    }
}

/// The displaced file is the planned one, intact; waiting for the
/// publication of the replacement.
#[allow(dead_code)]
pub struct Publish<I: Identity> {
    v: I,
    p: I,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> Publish<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.trace@, self.run@)
        &&& verified_at(self.trace@, Name::Displaced, self.v.ino())
    }
}

impl<I: Identity> Publish<I> {
    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// Feed the publication handed out with this state. Published: the
    /// staging guard is disarmed (its name is consumed or reported by the
    /// publication) and the displaced planned file's proven removal is handed
    /// out. Not published: the displaced file is put back.
    pub fn after_publish<E>(
        self,
        guard: &mut GuardState<I>,
        displaced: &mut GuardState<I>,
        publication: NoReplaceDone<E>,
    ) -> (r: PublishedStep<I, E>)
        requires
            publication.from() == Name::Staging,
            publication.ident() == self.run().own,
            old(displaced).name() == Name::Displaced,
            old(displaced).ident() == self.run().verified,
        ensures
            r is Cleanup <==> publication.result() is Ok,
            r matches PublishedStep::Cleanup(x, removal) ==> {
                &&& x.run() == self.run()
                &&& removal.name() == Name::Displaced
                &&& removal.ident() == self.run().verified
                &&& !final(guard).is_armed()
                &&& !final(displaced).is_armed()
            },
            r matches PublishedStep::Restore(x, restore) ==> {
                &&& x.run() == self.run()
                &&& restore.from() == Name::Displaced
                &&& restore.ident() == self.run().verified
                &&& *final(guard) == *old(guard)
                &&& *final(displaced) == *old(displaced)
            },
            final(guard).name() == old(guard).name(),
            final(guard).ident() == old(guard).ident(),
            final(displaced).name() == old(displaced).name(),
            final(displaced).ident() == old(displaced).ident(),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost t = self.trace@;
        let ghost b = publication.trace();
        let ghost ident = publication.ident();
        let result = publication.into_result();
        proof {
            lemma_safe_append_noreplace(t, b, Name::Staging, ident, result, self.run@);
            crate::publish::noreplace::theorem_noreplace(b, Name::Staging, ident, result);
            lemma_published_append(t, b, Name::Staging);
            lemma_verified_at_append(t, b, Name::Displaced, self.v.ino());
        }
        match result {
            Ok(staging) => {
                guard.disarm();
                let removal = displaced.take_removal();
                let ghost v = self.v.ino();
                PublishedStep::Cleanup(
                    Cleanup { v: Ghost(v), staging, trace: Ghost(t + b), run: self.run },
                    removal,
                )
            },
            Err(error) => PublishedStep::Restore(
                RestoreAfterFailure {
                    v: self.v,
                    publish_error: error,
                    trace: Ghost(t + b),
                    run: self.run,
                },
                NoReplace::new(Some(self.v), Ghost(Name::Displaced)),
            ),
        }
    }
}

/// After the publication of the replacement.
pub enum PublishedStep<I: Identity, E> {
    /// Published: remove the displaced planned file, then report.
    Cleanup(Cleanup<E>, ProvenRemoval<I>),
    /// Not published: put the displaced file back, then report.
    Restore(RestoreAfterFailure<I, E>, NoReplace<I>),
}

/// The replacement is published; waiting for the proven removal of the
/// displaced planned file.
#[allow(dead_code)]
pub struct Cleanup<E> {
    v: Ghost<Ino>,
    staging: Published<E>,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<E> Cleanup<E> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& safe(self.trace@, self.run@)
        &&& self.run@.verified == Some(self.v@)
        &&& verified_at(self.trace@, Name::Displaced, self.v@)
        &&& published(self.trace@, Name::Staging)
    }

    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// Feed the removal handed out with this state. Both cleanups matter: the
    /// displaced original may hold secrets, and a surviving staging name may
    /// belong to another writer.
    pub fn after_removal(self, removal: RemovalDone<E>) -> (d: DisplaceDone<E>)
        requires
            removal.name() == Name::Displaced,
            removal.ident() == self.run().verified,
        ensures
            d.run() == self.run(),
            d.verdict() is Replaced <==> (removal.outcome() is Removed && self.staging() is Consumed),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost t = self.trace@;
        let ghost b = removal.trace();
        let outcome = removal.into_outcome();
        proof {
            lemma_safe_append_removal(t, b, Name::Displaced, self.run@.verified, outcome, self.run@);
            lemma_verified_at_append(t, b, Name::Displaced, self.v@);
            lemma_published_append(t, b, Name::Staging);
        }
        let verdict = match (outcome, self.staging) {
            (Removal::Removed, Published::Consumed) => DisplaceVerdict::Replaced,
            (Removal::Left(displaced), Published::Consumed) => DisplaceVerdict::DisplacedLeft(
                displaced,
            ),
            (Removal::Removed, Published::LeftBehind(staging)) => DisplaceVerdict::StagingLeft(
                staging,
            ),
            (Removal::Left(displaced_left), Published::LeftBehind(staging_left)) => {
                DisplaceVerdict::BothLeft { displaced: displaced_left, staging: staging_left }
            },
        };
        DisplaceDone { verdict, trace: Ghost(t + b), run: self.run }
    }

    /// What the publication left under the staging name.
    pub closed spec fn staging(&self) -> Published<E> {
        self.staging
    }
}

/// The replacement could not be published; waiting for the restore of the
/// displaced planned file.
#[allow(dead_code)]
pub struct RestoreAfterFailure<I: Identity, E> {
    v: I,
    publish_error: E,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity, E> RestoreAfterFailure<I, E> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& safe(self.trace@, self.run@)
        &&& self.run@.verified == Some(self.v.ino())
    }
}

impl<I: Identity, E> RestoreAfterFailure<I, E> {
    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// Feed the restore handed out with this state. Restored: the displaced
    /// guard is disarmed (its name was consumed or is reported) and the
    /// staging guard stays armed, since the replacement was never published.
    /// Not restored: both are disarmed and both files are kept.
    pub fn after_restore(
        self,
        guard: &mut GuardState<I>,
        displaced: &mut GuardState<I>,
        restore: NoReplaceDone<E>,
    ) -> (d: DisplaceDone<E>)
        requires
            restore.from() == Name::Displaced,
            restore.ident() == self.run().verified,
        ensures
            d.run() == self.run(),
            restore.result() is Ok ==> {
                &&& d.verdict() is PublishFailedRestored
                &&& !final(displaced).is_armed()
                &&& *final(guard) == *old(guard)
            },
            restore.result() is Err ==> {
                &&& d.verdict() is PublishFailedStranded
                &&& !final(displaced).is_armed()
                &&& !final(guard).is_armed()
            },
            final(guard).name() == old(guard).name(),
            final(guard).ident() == old(guard).ident(),
            final(displaced).name() == old(displaced).name(),
            final(displaced).ident() == old(displaced).ident(),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost t = self.trace@;
        let ghost b = restore.trace();
        let ghost ident = restore.ident();
        let result = restore.into_result();
        proof {
            lemma_safe_append_noreplace(t, b, Name::Displaced, ident, result, self.run@);
            crate::publish::noreplace::theorem_noreplace(b, Name::Displaced, ident, result);
            lemma_published_append(t, b, Name::Displaced);
        }
        let verdict = match result {
            Ok(restored) => {
                displaced.disarm();
                let left = match restored {
                    Published::Consumed => None,
                    Published::LeftBehind(left) => Some(left),
                };
                DisplaceVerdict::PublishFailedRestored { publish: self.publish_error, left }
            },
            Err(restore_error) => {
                guard.disarm();
                displaced.disarm();
                DisplaceVerdict::PublishFailedStranded {
                    publish: self.publish_error,
                    restore: restore_error,
                }
            },
        };
        DisplaceDone { verdict, trace: Ghost(t + b), run: self.run }
    }
}

/// How a displacement ended.
pub enum DisplaceVerdict<E> {
    /// Published, and the displaced planned file removed.
    Replaced,
    /// The displaced name could not be read back: nothing was published or
    /// deleted, and the destination name is vacant.
    ReadBackFailed(E),
    /// The displaced file was not the planned one and was put back.
    RestoredUntouched,
    /// ... put back, but the displaced name was left behind.
    RestoredLeftBehind(Leftover<E>),
    /// ... and could not be put back: it is kept under the displaced name.
    RestoreFailed(E),
    /// Published, but the displaced planned file could not be removed.
    DisplacedLeft(Leftover<E>),
    /// Published and the displaced file removed, but the staging name was
    /// left behind.
    StagingLeft(Leftover<E>),
    /// Published, but both names were left behind.
    BothLeft { displaced: Leftover<E>, staging: Leftover<E> },
    /// Not published; the displaced file was put back (`left`: its name was
    /// left behind as well).
    PublishFailedRestored { publish: E, left: Option<Leftover<E>> },
    /// Not published and not put back: both files are kept.
    PublishFailedStranded { publish: E, restore: E },
}

/// A finished displacement.
#[allow(dead_code)]
pub struct DisplaceDone<E> {
    verdict: DisplaceVerdict<E>,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

/// D7: the displaced entry was verified as the planned inode, the
/// replacement was published, and the trace ends with the planned inode's
/// successful removal from the displaced name.
pub open spec fn displaced_and_replaced(t: Seq<Ev>, v: Option<Ino>) -> bool {
    &&& v is Some
    &&& verified_at(t, Name::Displaced, v->Some_0)
    &&& published(t, Name::Staging)
    &&& t.len() > 0
    &&& t.last() == (Ev::Unlink { name: Name::Displaced, ok: true })
}

impl<E> DisplaceDone<E> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& safe(self.trace@, self.run@)
        &&& (self.verdict is Replaced ==> displaced_and_replaced(self.trace@, self.run@.verified))
        &&& (self.verdict is RestoredUntouched ==> consumed(self.trace@, Name::Displaced))
        &&& (self.verdict matches DisplaceVerdict::PublishFailedRestored { left, .. } && left is None
            ==> consumed(self.trace@, Name::Displaced))
    }

    /// The run this replacement acted for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// What the replacement asked for.
    pub closed spec fn trace(&self) -> Seq<Ev> {
        self.trace@
    }

    /// The verdict.
    pub closed spec fn verdict(&self) -> DisplaceVerdict<E> {
        self.verdict
    }

    /// The verdict, with its contract.
    pub fn into_verdict(self) -> (r: DisplaceVerdict<E>)
        ensures
            r == self.verdict(),
            safe(self.trace(), self.run()),
            r is Replaced ==> displaced_and_replaced(self.trace(), self.run().verified),
            r is RestoredUntouched ==> consumed(self.trace(), Name::Displaced),
            (r matches DisplaceVerdict::PublishFailedRestored { left, .. } && left is None)
                ==> consumed(self.trace(), Name::Displaced),
    {
        proof {
            use_type_invariant(&self);
        }
        self.verdict
    }
}

} // verus!
