//! Replacement of a verified destination: an atomic exchange with a proven
//! undo, or — where the filesystem cannot exchange two names — displacement
//! followed by a no-clobber publication.
//!
//! The machine starts once production has proved that the destination name
//! resolves to the pinned destination descriptor (identity `v`) and that the
//! descriptor still reads back as the planned snapshot
//! ([`Exchange::after_destination_verified`]). From there every step is a
//! typestate: production feeds back the answer to the probe or syscall the
//! state names and receives the next state, so the answers can only be fed in
//! the machine's order. The staging guard (and, on the displacement path, the
//! displaced guard) is passed in by `&mut`, and this machine only ever
//! disarms it; a guard's identity never changes. Every proven removal and
//! no-clobber publication the machine hands out carries the state waiting for
//! it, so its result can only be fed back to that state.
//!
//! Every state carries the ghost trace of what was asked so far, with
//! `safe` as part of its type invariant, and every finished record
//! ([`ExchangeDone`], [`UndoDone`], [`DisplaceDone`]) carries `safe` plus a
//! contract on its verdict. Proven, for every path:
//!
//! * **X1** the exchange is issued only right after proving the destination
//!   holds `v` with its planned bytes (and an exchange back only right after
//!   proving the destination holds this run's prepared file `p`);
//! * **X2** `Ok` on the exchange path means: the exchange succeeded, the
//!   name it swapped out was then proved to hold `v`, the pinned descriptor
//!   still read back as planned, and the trace ends with a probe proving the
//!   staging name still holds `v` immediately followed by its successful
//!   unlink (`exchanged_and_removed`); the staging guard is disarmed in
//!   exactly that branch and the removal of `v` is the kernel's own;
//! * **X3** the undo touches the destination only after proving it still holds
//!   `p`, and unlinks the staging name only after proving it holds `p` again
//!   and then proving it holds the staging guard's identity; every failed
//!   proof disarms the guard;
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
//! * **D2** the planned inode `v` is unlinked from the displaced name only by
//!   the kernel's proven removal for `v`, only after the moved entry was
//!   identified as `v` and the pinned descriptor read back as planned, and only
//!   after the replacement was published; the only other unlink of the
//!   displaced name is a restore's own link-fallback cleanup (D3/D4);
//! * **D3/D4** anything else that was displaced is put back with a no-clobber
//!   restore, never published over and never unlinked except by that
//!   restore's own link cleanup; a failed publication is followed by exactly
//!   one restore;
//! * **D5** every displacement verdict says exactly which of the two names the
//!   run consumed (`names_left`): `Replaced` means both the staging name and
//!   the displaced name are gone, `DisplacedLeft`/`StagingLeft`/`BothLeft`
//!   name exactly the ones that are not, the restore verdicts say whether the
//!   displaced name was consumed, and no verdict before publication claims a
//!   publication of the staging name;
//! * **D6** the verdict after a publication carries exactly the leftover
//!   reasons the publication and the removal produced, and the verdict after
//!   a failed publication carries exactly the publication's and the restore's
//!   errors, each in its own field;
//! * **D7** `Replaced` on the displacement path means the displaced entry was
//!   verified as `v`, the replacement was published, and the trace ends with a
//!   probe proving the displaced name holds `v` immediately followed by its
//!   successful unlink.
//!
//! Trust: production proves the two facts [`Exchange::after_destination_verified`]
//! and [`Reprove::reproved`] stand for before calling them, runs every probe
//! against the identity the state hands out, and feeds back true answers. The
//! displaced name is a fresh 128-bit nonce (the A-fresh axiom of
//! [`crate::publish::model`]).

#[allow(unused_imports)]
use vstd::prelude::*;

use crate::publish::noreplace::{Attempt, AwaitsPublication, NoReplace, NoReplaceDone};
#[cfg(verus_keep_ghost)]
use crate::publish::noreplace::{consumed, published, res_of};
use crate::publish::ordering::Ready;
use crate::publish::removal::{AwaitsRemoval, GuardState, ProvenRemoval, RemovalDone};
#[allow(unused_imports)]
use crate::publish::trace::{Ev, Identity, Ino, Leftover, Name, Published, Removal, Res, Run};
#[cfg(verus_keep_ghost)]
use crate::publish::trace::{
    ino_of, lemma_safe_append_noreplace, lemma_safe_append_removal, lemma_safe_push,
    lemma_safe_push_observation, lemma_verified_at_append, moved_then_verified, noreplace_trace,
    removal_trace, safe, verified_at,
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
pub open spec fn state_ok<I: Identity>(
    v: I,
    p: I,
    own: Option<I>,
    trace: Seq<Ev>,
    run: Run,
) -> bool {
    &&& run == run_of(ino_of(own), v.ino(), p.ino())
    &&& safe(trace, run)
}

/// `t` publishes nothing and unlinks nothing: no `RENAME_NOREPLACE`, link or
/// unlink at all.
pub open spec fn quiet(t: Seq<Ev>) -> bool {
    forall|i: int|
        #![trigger t[i]]
        0 <= i < t.len() ==> !(t[i] is RenameNoReplace || t[i] is Link || t[i] is Unlink)
}

proof fn lemma_safe_empty(run: Run)
    ensures
        safe(Seq::<Ev>::empty(), run),
{
}

proof fn lemma_contains_append(a: Seq<Ev>, b: Seq<Ev>, e: Ev)
    ensures
        (a + b).contains(e) <==> (a.contains(e) || b.contains(e)),
{
    if a.contains(e) {
        let i = choose|i: int| 0 <= i < a.len() && a[i] == e;
        assert((a + b)[i] == e);
    }
    if b.contains(e) {
        let i = choose|i: int| 0 <= i < b.len() && b[i] == e;
        assert((a + b)[a.len() + i] == e);
    }
    if (a + b).contains(e) {
        let i = choose|i: int| 0 <= i < (a + b).len() && (a + b)[i] == e;
        if i < a.len() {
            assert(a[i] == e);
        } else {
            assert(b[i - a.len()] == e);
        }
    }
}

/// Publication and consumption of a name in a concatenation.
proof fn lemma_names_append(a: Seq<Ev>, b: Seq<Ev>, n: Name)
    ensures
        published(a + b, n) <==> (published(a, n) || published(b, n)),
        consumed(a + b, n) <==> (consumed(a, n) || consumed(b, n)),
{
    lemma_contains_append(a, b, Ev::RenameNoReplace { from: n, res: Res::Ok });
    lemma_contains_append(a, b, Ev::Link { from: n, ok: true });
    lemma_contains_append(a, b, Ev::Unlink { name: n, ok: true });
}

/// A quiet trace neither publishes nor consumes any name.
proof fn lemma_quiet_names(t: Seq<Ev>, n: Name)
    requires
        quiet(t),
    ensures
        !published(t, n),
        !consumed(t, n),
{
    if t.contains(Ev::RenameNoReplace { from: n, res: Res::Ok }) {
        let i = choose|i: int|
            0 <= i < t.len() && t[i] == Ev::RenameNoReplace { from: n, res: Res::Ok };
        assert(!(t[i] is RenameNoReplace));
    }
    if t.contains(Ev::Link { from: n, ok: true }) {
        let i = choose|i: int| 0 <= i < t.len() && t[i] == Ev::Link { from: n, ok: true };
        assert(!(t[i] is Link));
    }
    if t.contains(Ev::Unlink { name: n, ok: true }) {
        let i = choose|i: int| 0 <= i < t.len() && t[i] == Ev::Unlink { name: n, ok: true };
        assert(!(t[i] is Unlink));
    }
}

/// Appending an event that neither publishes nor unlinks keeps a trace quiet.
proof fn lemma_quiet_push(t: Seq<Ev>, e: Ev)
    requires
        quiet(t),
        !(e is RenameNoReplace || e is Link || e is Unlink),
    ensures
        quiet(t.push(e)),
{
    assert forall|i: int| 0 <= i < t.push(e).len() implies !(t.push(e)[i] is RenameNoReplace
        || t.push(e)[i] is Link || t.push(e)[i] is Unlink) by {
        if i < t.len() {
            assert(t.push(e)[i] == t[i]);
        }
    }
}

proof fn lemma_quiet_verified_prefix(v: Ino)
    ensures
        quiet(verified_prefix(v)),
{
    let t = verified_prefix(v);
    assert forall|i: int| 0 <= i < t.len() implies !(t[i] is RenameNoReplace || t[i] is Link
        || t[i] is Unlink) by {
        if i == 0 {
        } else {
            assert(i == 1);
        }
    }
}

/// A proven removal of `n` touches no other name and publishes nothing; it
/// consumes `n` exactly when it removed it.
proof fn lemma_removal_names<E>(b: Seq<Ev>, n: Name, x: Option<Ino>, out: Removal<E>, m: Name)
    requires
        removal_trace(b, n, x, out),
    ensures
        !published(b, m),
        m != n ==> !consumed(b, m),
        consumed(b, n) <==> out is Removed,
{
    if b.contains(Ev::RenameNoReplace { from: m, res: Res::Ok }) {
        let i = choose|i: int|
            0 <= i < b.len() && b[i] == Ev::RenameNoReplace { from: m, res: Res::Ok };
    }
    if b.contains(Ev::Link { from: m, ok: true }) {
        let i = choose|i: int| 0 <= i < b.len() && b[i] == Ev::Link { from: m, ok: true };
    }
    if b.contains(Ev::Unlink { name: m, ok: true }) {
        let i = choose|i: int| 0 <= i < b.len() && b[i] == Ev::Unlink { name: m, ok: true };
    }
    if b.contains(Ev::RenameNoReplace { from: n, res: Res::Ok }) {
        let i = choose|i: int|
            0 <= i < b.len() && b[i] == Ev::RenameNoReplace { from: n, res: Res::Ok };
    }
    if b.contains(Ev::Unlink { name: n, ok: true }) {
        let i = choose|i: int| 0 <= i < b.len() && b[i] == Ev::Unlink { name: n, ok: true };
    }
    if out is Removed {
        assert(b[1] == Ev::Unlink { name: n, ok: true });
    }
}

/// A no-clobber publication of `n` neither publishes nor consumes any other
/// name.
proof fn lemma_noreplace_names<E>(
    b: Seq<Ev>,
    n: Name,
    x: Option<Ino>,
    result: Result<Published<E>, E>,
    m: Name,
)
    requires
        noreplace_trace(b, n, x, result),
        m != n,
    ensures
        !published(b, m),
        !consumed(b, m),
{
    let rnr = Ev::RenameNoReplace { from: m, res: Res::Ok };
    let link = Ev::Link { from: m, ok: true };
    let unlink = Ev::Unlink { name: m, ok: true };
    if b.len() >= 2 && b[0] == (Ev::RenameNoReplace { from: n, res: Res::Unsupported }) && b[1]
        == (Ev::Link { from: n, ok: true }) && result is Ok {
        let c = b.skip(2);
        lemma_removal_names(c, n, x, crate::publish::trace::removal_of(result->Ok_0), m);
        if b.contains(rnr) {
            let i = choose|i: int| 0 <= i < b.len() && b[i] == rnr;
            assert(c[i - 2] == rnr);
            assert(c.contains(rnr));
        }
        if b.contains(link) {
            let i = choose|i: int| 0 <= i < b.len() && b[i] == link;
            assert(c[i - 2] == link);
            assert(c.contains(link));
        }
        if b.contains(unlink) {
            let i = choose|i: int| 0 <= i < b.len() && b[i] == unlink;
            assert(c[i - 2] == unlink);
            assert(c.contains(unlink));
        }
    } else {
        if b.contains(rnr) {
            let i = choose|i: int| 0 <= i < b.len() && b[i] == rnr;
        }
        if b.contains(link) {
            let i = choose|i: int| 0 <= i < b.len() && b[i] == link;
        }
        if b.contains(unlink) {
            let i = choose|i: int| 0 <= i < b.len() && b[i] == unlink;
        }
    }
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
    own: Option<I>,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> Exchange<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.own, self.trace@, self.run@)
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
    /// staging file and `guard` is its staging guard, whose identity is the
    /// run's own file for the rest of the replacement.
    pub fn after_destination_verified(
        _ready: Ready,
        guard: &GuardState<I>,
        verified: I,
        prepared: I,
    ) -> (r: Self)
        ensures
            r.run() == run_of(guard.ident(), verified.ino(), prepared.ino()),
    {
        let own = guard.identity();
        let ghost run = run_of(ino_of(own), verified.ino(), prepared.ino());
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
        Exchange { v: verified, p: prepared, own, trace: Ghost(trace), run: Ghost(run) }
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
                SwapProbe { v: self.v, p: self.p, own: self.own, trace: Ghost(u), run: self.run },
            ),
            Attempt::Unsupported => Exchanged::Displace(
                Reprove { v: self.v, p: self.p, own: self.own, trace: Ghost(u), run: self.run },
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
    own: Option<I>,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> SwapProbe<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.own, self.trace@, self.run@)
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
            SwapProbed::Pinned(
                SwapPinned { v: self.v, p: self.p, own: self.own, trace: Ghost(u), run: self.run },
            )
        } else {
            SwapProbed::Undo(
                Undo { v: self.v, p: self.p, own: self.own, trace: Ghost(u), run: self.run },
            )
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
    own: Option<I>,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> SwapPinned<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.own, self.trace@, self.run@)
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
    /// against: the staging guard is then disarmed (its name no longer holds
    /// this run's file) and the kernel's own proven removal of the planned
    /// inode from the staging name is handed out. Otherwise the exchange is
    /// undone and the guard is left exactly as it was.
    pub fn after_pinned(self, guard: &mut GuardState<I>, matches: bool) -> (r: SwapChecked<I>)
        ensures
            r is Remove <==> matches,
            r matches SwapChecked::Remove(removal) ==> {
                &&& removal.then().run() == self.run()
                &&& removal.name() == Name::Staging
                &&& removal.ident() == self.run().verified
                &&& !final(guard).is_armed()
            },
            r matches SwapChecked::Undo(u) ==> u.run() == self.run() && *final(guard)
                == *old(guard),
            final(guard).name() == old(guard).name(),
            final(guard).ident() == old(guard).ident(),
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
            guard.disarm();
            let remove = ExchangeRemove {
                v: self.v,
                p: self.p,
                own: self.own,
                trace: Ghost(u),
                run: self.run,
            };
            SwapChecked::Remove(ProvenRemoval::new(Some(self.v), remove))
        } else {
            SwapChecked::Undo(
                Undo { v: self.v, p: self.p, own: self.own, trace: Ghost(u), run: self.run },
            )
        }
    }
}

/// After the read-back.
pub enum SwapChecked<I: Identity> {
    /// The planned file was swapped out intact: remove it from the staging
    /// name, then feed the result back with `after_removal`.
    Remove(ProvenRemoval<I, ExchangeRemove<I>>),
    /// It was not: undo.
    Undo(Undo<I>),
}

/// The replacement stands; waiting for the proven removal of the planned
/// file from the staging name.
#[allow(dead_code)]
pub struct ExchangeRemove<I: Identity> {
    v: I,
    p: I,
    own: Option<I>,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> ExchangeRemove<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.own, self.trace@, self.run@)
        &&& verified_at(self.trace@, Name::Staging, self.v.ino())
    }

    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }
}

impl<I: Identity> AwaitsRemoval for ExchangeRemove<I> {
    closed spec fn removal_name(&self) -> Name {
        Name::Staging
    }

    closed spec fn removal_ident(&self) -> Option<Ino> {
        self.run@.verified
    }
}

impl<I: Identity, E> RemovalDone<E, ExchangeRemove<I>> {
    /// Feed back the removal of the planned file from the staging name.
    pub fn after_removal(self) -> (d: ExchangeDone<E>)
        ensures
            d.run() == self.then().run(),
            d.result() is Ok <==> self.outcome() is Removed,
            self.outcome() matches Removal::Left(l) ==> d.result() == Err::<(), Leftover<E>>(l),
    {
        let ghost b = self.trace();
        let (state, outcome) = self.into_parts();
        proof {
            use_type_invariant(&state);
        }
        let ghost t = state.trace@;
        let ghost u = t + b;
        proof {
            lemma_safe_append_removal(t, b, Name::Staging, state.run@.verified, outcome, state.run@);
            lemma_verified_at_append(t, b, Name::Staging, state.v.ino());
            if outcome is Removed {
                assert(u[u.len() - 2] == b[0]);
                assert(u[u.len() - 1] == b[1]);
            }
        }
        let result = match outcome {
            Removal::Removed => Ok(()),
            Removal::Left(left) => Err(left),
        };
        ExchangeDone { result, trace: Ghost(u), run: state.run }
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
/// planned inode, and the trace ends with a probe proving the staging name
/// holds that inode immediately followed by its successful unlink.
pub open spec fn exchanged_and_removed(t: Seq<Ev>, v: Option<Ino>) -> bool {
    &&& v is Some
    &&& verified_at(t, Name::Staging, v->Some_0)
    &&& t.len() >= 2
    &&& t[t.len() - 2] == (Ev::Probe { name: Name::Staging, ino: v->Some_0, holds: true })
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
    own: Option<I>,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> Undo<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        state_ok(self.v, self.p, self.own, self.trace@, self.run@)
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
                UndoExchangeBack {
                    v: self.v,
                    p: self.p,
                    own: self.own,
                    trace: Ghost(u),
                    run: self.run,
                },
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
    own: Option<I>,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> UndoExchangeBack<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.own, self.trace@, self.run@)
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
                UndoStagingProbe {
                    v: self.v,
                    p: self.p,
                    own: self.own,
                    trace: Ghost(u),
                    run: self.run,
                },
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
    own: Option<I>,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> UndoStagingProbe<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        state_ok(self.v, self.p, self.own, self.trace@, self.run@)
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
    /// is the staging guard's removal handed out — entitled to the run's own
    /// file, the identity the guard was entitled to when the replacement
    /// started; the guard is disarmed either way, since that removal replaces
    /// it or the name is left alone.
    pub fn after_staging_probe<E>(self, guard: &mut GuardState<I>, holds: bool) -> (r:
        UndoStagingProbed<I, E>)
        ensures
            r is Remove <==> holds,
            r matches UndoStagingProbed::Done(d) ==> d.run() == self.run() && d.verdict()
                == UndoVerdict::<E>::ReplacementNotHeld,
            r matches UndoStagingProbed::Remove(removal) ==> {
                &&& removal.then().run() == self.run()
                &&& removal.name() == Name::Staging
                &&& removal.ident() == self.run().own
            },
            !final(guard).is_armed(),
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
        guard.disarm();
        if holds {
            let remove = UndoRemove {
                v: self.v,
                p: self.p,
                own: self.own,
                trace: Ghost(u),
                run: self.run,
            };
            UndoStagingProbed::Remove(ProvenRemoval::new(self.own, remove))
        } else {
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
    /// Remove this run's prepared file from the staging name, then feed the
    /// result back with `after_removal`.
    Remove(ProvenRemoval<I, UndoRemove<I>>),
}

/// An undo waiting for its final proven removal.
#[allow(dead_code)]
pub struct UndoRemove<I: Identity> {
    v: I,
    p: I,
    own: Option<I>,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> UndoRemove<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        state_ok(self.v, self.p, self.own, self.trace@, self.run@)
    }

    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }
}

impl<I: Identity> AwaitsRemoval for UndoRemove<I> {
    closed spec fn removal_name(&self) -> Name {
        Name::Staging
    }

    closed spec fn removal_ident(&self) -> Option<Ino> {
        self.run@.own
    }
}

impl<I: Identity, E> RemovalDone<E, UndoRemove<I>> {
    /// Feed back the removal of this run's file from the staging name.
    pub fn after_removal(self) -> (d: UndoDone<E>)
        ensures
            d.run() == self.then().run(),
            d.verdict() is Undone <==> self.outcome() is Removed,
            self.outcome() matches Removal::Left(l) ==> d.verdict()
                == UndoVerdict::ReplacementNotRemoved(l),
    {
        let ghost b = self.trace();
        let (state, outcome) = self.into_parts();
        proof {
            use_type_invariant(&state);
        }
        let ghost t = state.trace@;
        proof {
            lemma_safe_append_removal(t, b, Name::Staging, state.run@.own, outcome, state.run@);
            if outcome is Removed {
                assert((t + b).last() == b[1]);
            }
        }
        let verdict = match outcome {
            Removal::Removed => UndoVerdict::Undone,
            Removal::Left(left) => UndoVerdict::ReplacementNotRemoved(left),
        };
        UndoDone { verdict, trace: Ghost(t + b), run: state.run }
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
    own: Option<I>,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> Reprove<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.own, self.trace@, self.run@)
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
        Aside {
            v: self.v,
            p: self.p,
            own: self.own,
            trace: Ghost(t.push(e0).push(e1)),
            run: self.run,
        }
    }
}

/// A displacement whose destination was re-proved: production generates a
/// fresh displaced name and moves the destination onto it.
#[allow(dead_code)]
pub struct Aside<I: Identity> {
    v: I,
    p: I,
    own: Option<I>,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

/// The trace of a displacement right before the destination is moved aside.
pub open spec fn aside_prefix(v: Ino) -> Seq<Ev> {
    verified_prefix(v).push(Ev::Exchange { res: Res::Unsupported }).push(
        Ev::Probe { name: Name::Dest, ino: v, holds: true },
    ).push(Ev::PinnedMatches { holds: true })
}

/// The trace of a displacement right after the destination was moved aside
/// and the displaced name was identified as `v`.
pub open spec fn lstat_prefix(v: Ino) -> Seq<Ev> {
    aside_prefix(v).push(Ev::RenameAside { ok: true }).push(
        Ev::Lstat { name: Name::Displaced, found: Some(v) },
    )
}

/// The trace of a displacement whose displaced planned file was verified
/// intact: what the publication of the replacement starts from.
pub open spec fn publish_prefix(v: Ino) -> Seq<Ev> {
    lstat_prefix(v).push(Ev::PinnedMatches { holds: true })
}

proof fn lemma_quiet_aside_prefix(v: Ino)
    ensures
        quiet(aside_prefix(v)),
        quiet(aside_prefix(v).push(Ev::RenameAside { ok: true })),
        quiet(lstat_prefix(v)),
        quiet(publish_prefix(v)),
{
    lemma_quiet_verified_prefix(v);
    let a = verified_prefix(v).push(Ev::Exchange { res: Res::Unsupported });
    lemma_quiet_push(verified_prefix(v), Ev::Exchange { res: Res::Unsupported });
    let b = a.push(Ev::Probe { name: Name::Dest, ino: v, holds: true });
    lemma_quiet_push(a, Ev::Probe { name: Name::Dest, ino: v, holds: true });
    lemma_quiet_push(b, Ev::PinnedMatches { holds: true });
    lemma_quiet_push(aside_prefix(v), Ev::RenameAside { ok: true });
    lemma_quiet_push(
        aside_prefix(v).push(Ev::RenameAside { ok: true }),
        Ev::Lstat { name: Name::Displaced, found: Some(v) },
    );
    lemma_quiet_push(lstat_prefix(v), Ev::PinnedMatches { holds: true });
}

impl<I: Identity> Aside<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.own, self.trace@, self.run@)
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
    ///
    /// The guard is armed from here on, before the displaced entry has been
    /// identified as `v`, as waft's guard always was: an unwinding panic before
    /// the verdict would let its `Drop` remove `v` from the displaced name
    /// while the destination name is vacant. That path is outside the model
    /// (Verus does not model unwinding) and outside `safe`, whose entitlement
    /// for the displaced name starts only at `verified_at`.
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
                let lstat = Lstat {
                    v: self.v,
                    p: self.p,
                    own: self.own,
                    trace: Ghost(t.push(e)),
                    run: self.run,
                };
                Ok((lstat, guard))
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
    own: Option<I>,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> Lstat<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.own, self.trace@, self.run@)
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
        ensures
            r matches Lstated::Done(d) ==> {
                &&& d.run() == self.run()
                &&& (found matches Err(e) && d.verdict() == DisplaceVerdict::<E>::ReadBackFailed(e))
                &&& !final(displaced).is_armed()
            },
            r matches Lstated::Restore(restore) ==> {
                &&& found matches Ok(d)
                &&& d.ino() != self.run().verified->Some_0
                &&& restore.then().run() == self.run()
                &&& restore.from() == Name::Displaced
                &&& restore.ident() == Some(d.ino())
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
            lemma_quiet_aside_prefix(self.v.ino());
        }
        let ghost t = self.trace@;
        match found {
            Err(error) => {
                let ghost u = t.push(Ev::Lstat { name: Name::Displaced, found: None });
                proof {
                    lemma_safe_push_observation(t, u.last(), self.run@);
                    assert(u == t.push(u.last()));
                    lemma_quiet_push(t, u.last());
                    lemma_quiet_names(u, Name::Staging);
                    lemma_quiet_names(u, Name::Displaced);
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
                    lemma_quiet_push(t, u.last());
                }
                if d.same_inode(&self.v) {
                    Lstated::Pinned(
                        DisplacedPinned {
                            v: self.v,
                            p: self.p,
                            own: self.own,
                            d,
                            trace: Ghost(u),
                            run: self.run,
                        },
                    )
                } else {
                    displaced.disarm();
                    let restore = Restore {
                        v: self.v,
                        p: self.p,
                        own: self.own,
                        d,
                        trace: Ghost(u),
                        run: self.run,
                    };
                    Lstated::Restore(NoReplace::new(Some(d), restore))
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
    /// with a no-clobber publication, then feed the result back with
    /// `after_restore`.
    Restore(NoReplace<I, Restore<I>>),
    /// The planned inode was displaced: check its bytes.
    Pinned(DisplacedPinned<I>),
}

/// The planned inode was displaced; waiting for the read-back of the pinned
/// descriptor.
#[allow(dead_code)]
pub struct DisplacedPinned<I: Identity> {
    v: I,
    p: I,
    own: Option<I>,
    d: I,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> DisplacedPinned<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.own, self.trace@, self.run@)
        &&& self.d.ino() == self.v.ino()
        &&& self.trace@ == lstat_prefix(self.v.ino())
    }
}

impl<I: Identity> DisplacedPinned<I> {
    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// Feed whether the pinned descriptor still reads back as planned. If it
    /// does, the replacement is published next (entitled to clean up the
    /// run's own staging file); otherwise the displaced file is put back.
    pub fn after_pinned(self, displaced: &mut GuardState<I>, matches: bool) -> (r:
        DisplacedChecked<I>)
        ensures
            r is Publish <==> matches,
            r matches DisplacedChecked::Publish(publish) ==> {
                &&& publish.then().run() == self.run()
                &&& publish.from() == Name::Staging
                &&& publish.ident() == self.run().own
                &&& *final(displaced) == *old(displaced)
            },
            r matches DisplacedChecked::Restore(restore) ==> {
                &&& restore.then().run() == self.run()
                &&& restore.from() == Name::Displaced
                &&& restore.ident() == self.run().verified
                &&& !final(displaced).is_armed()
            },
            final(displaced).name() == old(displaced).name(),
            final(displaced).ident() == old(displaced).ident(),
    {
        proof {
            use_type_invariant(&self);
            lemma_quiet_aside_prefix(self.v.ino());
        }
        let ghost t = self.trace@;
        let ghost u = t.push(Ev::PinnedMatches { holds: matches });
        proof {
            lemma_safe_push_observation(t, u.last(), self.run@);
            assert(u == t.push(u.last()));
            lemma_quiet_push(t, u.last());
        }
        if matches {
            let publish = Publish {
                v: self.v,
                p: self.p,
                own: self.own,
                trace: Ghost(u),
                run: self.run,
            };
            DisplacedChecked::Publish(NoReplace::new(self.own, publish))
        } else {
            displaced.disarm();
            let restore = Restore {
                v: self.v,
                p: self.p,
                own: self.own,
                d: self.d,
                trace: Ghost(u),
                run: self.run,
            };
            DisplacedChecked::Restore(NoReplace::new(Some(self.d), restore))
        }
    }
}

/// After checking the displaced planned inode's bytes.
pub enum DisplacedChecked<I: Identity> {
    /// It is intact: publish the replacement onto the vacated name, then feed
    /// the result back with `after_publish`.
    Publish(NoReplace<I, Publish<I>>),
    /// It was rewritten in place: put it back, then feed the result back with
    /// `after_restore`.
    Restore(NoReplace<I, Restore<I>>),
}

/// A displaced file that is not the planned one (or not with the planned
/// bytes) is being put back; waiting for that publication.
#[allow(dead_code)]
pub struct Restore<I: Identity> {
    v: I,
    p: I,
    own: Option<I>,
    d: I,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> Restore<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.own, self.trace@, self.run@)
        &&& quiet(self.trace@)
    }

    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// The identity of the displaced entry being put back.
    pub closed spec fn found(&self) -> Ino {
        self.d.ino()
    }
}

impl<I: Identity> AwaitsPublication for Restore<I> {
    closed spec fn publication_from(&self) -> Name {
        Name::Displaced
    }

    closed spec fn publication_ident(&self) -> Option<Ino> {
        Some(self.d.ino())
    }
}

impl<I: Identity, E> NoReplaceDone<E, Restore<I>> {
    /// Feed back the restore of a displaced file that was not the planned
    /// one.
    pub fn after_restore(self) -> (d: DisplaceDone<E>)
        ensures
            d.run() == self.then().run(),
            self.result() == Ok::<Published<E>, E>(Published::Consumed) ==> d.verdict()
                == DisplaceVerdict::<E>::RestoredUntouched,
            self.result() matches Ok(Published::LeftBehind(l)) ==> d.verdict()
                == DisplaceVerdict::RestoredLeftBehind(l),
            self.result() matches Err(e) ==> d.verdict() == DisplaceVerdict::RestoreFailed(e),
    {
        let ghost b = self.trace();
        let ghost ident = self.ident();
        let (state, result) = self.into_parts();
        proof {
            use_type_invariant(&state);
        }
        let ghost t = state.trace@;
        proof {
            lemma_safe_append_noreplace(t, b, Name::Displaced, ident, result, state.run@);
            crate::publish::noreplace::theorem_noreplace(b, Name::Displaced, ident, result);
            lemma_names_append(t, b, Name::Staging);
            lemma_names_append(t, b, Name::Displaced);
            lemma_quiet_names(t, Name::Staging);
            lemma_quiet_names(t, Name::Displaced);
            lemma_noreplace_names(b, Name::Displaced, ident, result, Name::Staging);
        }
        let verdict = match result {
            Ok(Published::Consumed) => DisplaceVerdict::RestoredUntouched,
            Ok(Published::LeftBehind(left)) => DisplaceVerdict::RestoredLeftBehind(left),
            Err(error) => DisplaceVerdict::RestoreFailed(error),
        };
        DisplaceDone { verdict, trace: Ghost(t + b), run: state.run }
    }
}

/// The displaced file is the planned one, intact; waiting for the
/// publication of the replacement.
#[allow(dead_code)]
pub struct Publish<I: Identity> {
    v: I,
    p: I,
    own: Option<I>,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

impl<I: Identity> Publish<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& state_ok(self.v, self.p, self.own, self.trace@, self.run@)
        &&& self.trace@ == publish_prefix(self.v.ino())
    }

    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }
}

impl<I: Identity> AwaitsPublication for Publish<I> {
    closed spec fn publication_from(&self) -> Name {
        Name::Staging
    }

    closed spec fn publication_ident(&self) -> Option<Ino> {
        self.run@.own
    }
}

impl<I: Identity, E> NoReplaceDone<E, Publish<I>> {
    /// Feed back the publication of the replacement. Published: the staging
    /// guard is disarmed (its name is consumed or reported by the
    /// publication), the displaced guard is disarmed, and the kernel's proven
    /// removal of the displaced planned file is handed out. Not published:
    /// the displaced file is put back, and both guards are left as they were.
    pub fn after_publish(self, guard: &mut GuardState<I>, displaced: &mut GuardState<I>) -> (r:
        PublishedStep<I, E>)
        ensures
            r is Cleanup <==> self.result() is Ok,
            r matches PublishedStep::Cleanup(removal) ==> {
                &&& removal.then().run() == self.then().run()
                &&& self.result() == Ok::<Published<E>, E>(removal.then().staging())
                &&& removal.name() == Name::Displaced
                &&& removal.ident() == self.then().run().verified
                &&& !final(guard).is_armed()
                &&& !final(displaced).is_armed()
            },
            r matches PublishedStep::Restore(restore) ==> {
                &&& restore.then().run() == self.then().run()
                &&& self.result() == Err::<Published<E>, E>(restore.then().publish_error())
                &&& restore.from() == Name::Displaced
                &&& restore.ident() == self.then().run().verified
                &&& *final(guard) == *old(guard)
                &&& *final(displaced) == *old(displaced)
            },
            final(guard).name() == old(guard).name(),
            final(guard).ident() == old(guard).ident(),
            final(displaced).name() == old(displaced).name(),
            final(displaced).ident() == old(displaced).ident(),
    {
        let ghost b = self.trace();
        let ghost ident = self.ident();
        let (state, result) = self.into_parts();
        proof {
            use_type_invariant(&state);
        }
        let ghost t = state.trace@;
        let ghost u = t + b;
        proof {
            lemma_quiet_aside_prefix(state.v.ino());
            lemma_safe_append_noreplace(t, b, Name::Staging, ident, result, state.run@);
            crate::publish::noreplace::theorem_noreplace(b, Name::Staging, ident, result);
            assert(moved_then_verified(t, 5, Name::Displaced, state.v.ino()));
            lemma_verified_at_append(t, b, Name::Displaced, state.v.ino());
            lemma_names_append(t, b, Name::Staging);
            lemma_names_append(t, b, Name::Displaced);
            lemma_quiet_names(t, Name::Staging);
            lemma_quiet_names(t, Name::Displaced);
            lemma_noreplace_names(b, Name::Staging, ident, result, Name::Displaced);
        }
        match result {
            Ok(staging) => {
                guard.disarm();
                displaced.disarm();
                let ghost v = state.v.ino();
                let cleanup = Cleanup { v: Ghost(v), staging, trace: Ghost(u), run: state.run };
                PublishedStep::Cleanup(ProvenRemoval::new(Some(state.v), cleanup))
            },
            Err(error) => {
                let restore = RestoreAfterFailure {
                    v: state.v,
                    publish_error: error,
                    trace: Ghost(u),
                    run: state.run,
                };
                PublishedStep::Restore(NoReplace::new(Some(state.v), restore))
            },
        }
    }
}

/// After the publication of the replacement.
pub enum PublishedStep<I: Identity, E> {
    /// Published: remove the displaced planned file, then feed the result
    /// back with `after_removal`.
    Cleanup(ProvenRemoval<I, Cleanup<E>>),
    /// Not published: put the displaced file back, then feed the result back
    /// with `after_restore`.
    Restore(NoReplace<I, RestoreAfterFailure<I, E>>),
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
        &&& (self.staging is Consumed <==> consumed(self.trace@, Name::Staging))
        &&& !consumed(self.trace@, Name::Displaced)
    }

    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// What the publication left under the staging name.
    pub closed spec fn staging(&self) -> Published<E> {
        self.staging
    }
}

impl<E> AwaitsRemoval for Cleanup<E> {
    closed spec fn removal_name(&self) -> Name {
        Name::Displaced
    }

    closed spec fn removal_ident(&self) -> Option<Ino> {
        self.run@.verified
    }
}

impl<E> RemovalDone<E, Cleanup<E>> {
    /// Feed back the removal of the displaced planned file. Both cleanups
    /// matter: the displaced original may hold secrets, and a surviving
    /// staging name may belong to another writer.
    pub fn after_removal(self) -> (d: DisplaceDone<E>)
        ensures
            d.run() == self.then().run(),
            published_verdict(d.verdict()),
            displaced_left(d.verdict()) == left_of(self.outcome()),
            staging_left(d.verdict()) == left_behind(self.then().staging()),
    {
        let ghost b = self.trace();
        let (state, outcome) = self.into_parts();
        proof {
            use_type_invariant(&state);
        }
        let ghost t = state.trace@;
        let ghost u = t + b;
        proof {
            lemma_safe_append_removal(
                t,
                b,
                Name::Displaced,
                state.run@.verified,
                outcome,
                state.run@,
            );
            lemma_verified_at_append(t, b, Name::Displaced, state.v@);
            lemma_names_append(t, b, Name::Staging);
            lemma_names_append(t, b, Name::Displaced);
            lemma_removal_names(b, Name::Displaced, state.run@.verified, outcome, Name::Staging);
            lemma_removal_names(b, Name::Displaced, state.run@.verified, outcome, Name::Displaced);
            if outcome is Removed {
                assert(u[u.len() - 2] == b[0]);
                assert(u[u.len() - 1] == b[1]);
            }
        }
        let verdict = match (outcome, state.staging) {
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
        DisplaceDone { verdict, trace: Ghost(u), run: state.run }
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
        &&& !published(self.trace@, Name::Staging)
        &&& !consumed(self.trace@, Name::Displaced)
    }

    /// The run this replacement acts for.
    pub closed spec fn run(&self) -> Run {
        self.run@
    }

    /// Why the replacement could not be published.
    pub closed spec fn publish_error(&self) -> E {
        self.publish_error
    }
}

impl<I: Identity, E> AwaitsPublication for RestoreAfterFailure<I, E> {
    closed spec fn publication_from(&self) -> Name {
        Name::Displaced
    }

    closed spec fn publication_ident(&self) -> Option<Ino> {
        self.run@.verified
    }
}

impl<I: Identity, E> NoReplaceDone<E, RestoreAfterFailure<I, E>> {
    /// Feed back the restore of the displaced planned file after the
    /// replacement could not be published. Restored: the displaced guard is
    /// disarmed (its name was consumed or is reported) and the staging guard
    /// stays armed, since the replacement was never published. Not restored:
    /// both are disarmed and both files are kept.
    pub fn after_restore(self, guard: &mut GuardState<I>, displaced: &mut GuardState<I>) -> (d:
        DisplaceDone<E>)
        ensures
            d.run() == self.then().run(),
            self.result() matches Ok(p) ==> {
                &&& d.verdict() == DisplaceVerdict::PublishFailedRestored {
                    publish: self.then().publish_error(),
                    left: left_behind(p),
                }
                &&& !final(displaced).is_armed()
                &&& *final(guard) == *old(guard)
            },
            self.result() matches Err(e) ==> {
                &&& d.verdict() == DisplaceVerdict::PublishFailedStranded {
                    publish: self.then().publish_error(),
                    restore: e,
                }
                &&& !final(displaced).is_armed()
                &&& !final(guard).is_armed()
            },
            final(guard).name() == old(guard).name(),
            final(guard).ident() == old(guard).ident(),
            final(displaced).name() == old(displaced).name(),
            final(displaced).ident() == old(displaced).ident(),
    {
        let ghost b = self.trace();
        let ghost ident = self.ident();
        let (state, result) = self.into_parts();
        proof {
            use_type_invariant(&state);
        }
        let ghost t = state.trace@;
        proof {
            lemma_safe_append_noreplace(t, b, Name::Displaced, ident, result, state.run@);
            crate::publish::noreplace::theorem_noreplace(b, Name::Displaced, ident, result);
            lemma_names_append(t, b, Name::Staging);
            lemma_names_append(t, b, Name::Displaced);
            lemma_noreplace_names(b, Name::Displaced, ident, result, Name::Staging);
        }
        let verdict = match result {
            Ok(restored) => {
                displaced.disarm();
                let left = match restored {
                    Published::Consumed => None,
                    Published::LeftBehind(left) => Some(left),
                };
                DisplaceVerdict::PublishFailedRestored { publish: state.publish_error, left }
            },
            Err(restore_error) => {
                guard.disarm();
                displaced.disarm();
                DisplaceVerdict::PublishFailedStranded {
                    publish: state.publish_error,
                    restore: restore_error,
                }
            },
        };
        DisplaceDone { verdict, trace: Ghost(t + b), run: state.run }
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

/// Why a proven removal left its name, if it did.
pub open spec fn left_of<E>(r: Removal<E>) -> Option<Leftover<E>> {
    match r {
        Removal::Removed => None,
        Removal::Left(l) => Some(l),
    }
}

/// Why a no-clobber publication left the name it published from, if it did.
pub open spec fn left_behind<E>(p: Published<E>) -> Option<Leftover<E>> {
    match p {
        Published::Consumed => None,
        Published::LeftBehind(l) => Some(l),
    }
}

/// The verdict reports a published replacement.
pub open spec fn published_verdict<E>(v: DisplaceVerdict<E>) -> bool {
    v is Replaced || v is DisplacedLeft || v is StagingLeft || v is BothLeft
}

/// What the verdict says was left under the displaced name after a
/// publication.
pub open spec fn displaced_left<E>(v: DisplaceVerdict<E>) -> Option<Leftover<E>> {
    match v {
        DisplaceVerdict::DisplacedLeft(l) => Some(l),
        DisplaceVerdict::BothLeft { displaced, .. } => Some(displaced),
        _ => None,
    }
}

/// What the verdict says was left under the staging name after a
/// publication.
pub open spec fn staging_left<E>(v: DisplaceVerdict<E>) -> Option<Leftover<E>> {
    match v {
        DisplaceVerdict::StagingLeft(l) => Some(l),
        DisplaceVerdict::BothLeft { staging, .. } => Some(staging),
        _ => None,
    }
}

/// D5: the verdict says exactly which names the trace consumed — whether the
/// replacement was published at all, whether the staging name is gone after a
/// publication, and whether the displaced name is gone.
pub open spec fn names_left<E>(v: DisplaceVerdict<E>, t: Seq<Ev>) -> bool {
    let staged = published(t, Name::Staging);
    let staging_gone = consumed(t, Name::Staging);
    let displaced_gone = consumed(t, Name::Displaced);
    match v {
        DisplaceVerdict::Replaced => staged && staging_gone && displaced_gone,
        DisplaceVerdict::DisplacedLeft(_) => staged && staging_gone && !displaced_gone,
        DisplaceVerdict::StagingLeft(_) => staged && !staging_gone && displaced_gone,
        DisplaceVerdict::BothLeft { .. } => staged && !staging_gone && !displaced_gone,
        DisplaceVerdict::ReadBackFailed(_) => !staged && !displaced_gone,
        DisplaceVerdict::RestoredUntouched => !staged && displaced_gone,
        DisplaceVerdict::RestoredLeftBehind(_) => !staged && !displaced_gone,
        DisplaceVerdict::RestoreFailed(_) => !staged && !displaced_gone,
        DisplaceVerdict::PublishFailedRestored { left, .. } => !staged && (left is None
            <==> displaced_gone),
        DisplaceVerdict::PublishFailedStranded { .. } => !staged && !displaced_gone,
    }
}

/// A finished displacement.
#[allow(dead_code)]
pub struct DisplaceDone<E> {
    verdict: DisplaceVerdict<E>,
    trace: Ghost<Seq<Ev>>,
    run: Ghost<Run>,
}

/// D7: the displaced entry was verified as the planned inode, the
/// replacement was published, and the trace ends with a probe proving the
/// displaced name holds the planned inode immediately followed by its
/// successful unlink.
pub open spec fn displaced_and_replaced(t: Seq<Ev>, v: Option<Ino>) -> bool {
    &&& v is Some
    &&& verified_at(t, Name::Displaced, v->Some_0)
    &&& published(t, Name::Staging)
    &&& t.len() >= 2
    &&& t[t.len() - 2] == (Ev::Probe { name: Name::Displaced, ino: v->Some_0, holds: true })
    &&& t.last() == (Ev::Unlink { name: Name::Displaced, ok: true })
}

impl<E> DisplaceDone<E> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& safe(self.trace@, self.run@)
        &&& names_left(self.verdict, self.trace@)
        &&& (self.verdict is Replaced ==> displaced_and_replaced(self.trace@, self.run@.verified))
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
            names_left(r, self.trace()),
            r is Replaced ==> displaced_and_replaced(self.trace(), self.run().verified),
    {
        proof {
            use_type_invariant(&self);
        }
        self.verdict
    }
}

} // verus!
