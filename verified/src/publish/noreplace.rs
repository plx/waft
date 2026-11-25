//! No-clobber publication of a staging or displaced name onto the
//! destination: `RENAME_NOREPLACE`, with a hard link as the fallback where
//! the filesystem does not implement the flag.
//!
//! Production feeds back the answer to each step the machine names; the
//! typestates fix the order in which answers can be fed. A publication
//! carries the kernel state that asked for it (a type implementing
//! [`AwaitsPublication`]), which names the published name and the identity
//! the link cleanup is entitled to, so its finished record can only be fed
//! back to that state. Every [`NoReplaceDone`] carries `noreplace_trace` as a
//! type invariant, from which `theorem_noreplace` derives:
//!
//! * **NR1** every unlink is issued right after a probe proved the extra name
//!   still holds the identity the publication was given, and right after that
//!   publication's own successful link; with no identity there is no unlink;
//! * **NR2** a hard link is made only after `RENAME_NOREPLACE` was reported
//!   unsupported — any other rename error ends the publication;
//! * **NR3** the result is an error exactly when nothing was published (no
//!   successful rename and no successful link), so a caller never reports a
//!   failure for a publication that happened, nor success for one that did
//!   not;
//! * **NR4** the result is `Consumed` exactly when the source name is gone:
//!   the rename succeeded, or the extra name's unlink did.
//!
//! A publication of a missing destination ([`NoReplace::staging`]) ends with
//! [`NoReplaceDone::into_result`], which disarms the staging guard exactly
//! when something was published: the staging name is then either consumed or
//! reported, and the guard must not act on it.
//!
//! The filesystem-level consequence (the destination entry present before
//! the call is never replaced or removed) is proven in
//! [`crate::publish::model`].
//!
//! Trust: production classifies the rename's errno with its unsupported-flag
//! table, and runs the rename, the link and the cleanup on the names this
//! machine was created for.

#[allow(unused_imports)]
use vstd::prelude::*;

use crate::publish::ordering::Ready;
use crate::publish::removal::{AwaitsRemoval, GuardState, ProvenRemoval, RemovalDone};
#[allow(unused_imports)]
use crate::publish::trace::{Ev, Identity, Ino, Name, Published, Removal, Res};
#[cfg(verus_keep_ghost)]
use crate::publish::trace::{
    entitled, ino_of, links_are_fallbacks, noreplace_trace, proven_unlinks, removal_of,
    removal_trace, unlink_proven_at,
};

verus! {

/// How a rename carrying a `RENAME_*` flag was answered.
pub enum Attempt<E> {
    /// It succeeded.
    Done,
    /// The filesystem does not implement the flag.
    Unsupported,
    /// It failed for any other reason.
    Failed(E),
}

/// The ghost view of an [`Attempt`].
pub open spec fn res_of<E>(a: Attempt<E>) -> Res {
    match a {
        Attempt::Done => Res::Ok,
        Attempt::Unsupported => Res::Unsupported,
        Attempt::Failed(_) => Res::Failed,
    }
}

/// A kernel state waiting for the no-clobber publication it handed out.
///
/// It names the name being published onto the destination and the only
/// identity the link fallback's cleanup may remove from that name.
pub trait AwaitsPublication: Sized {
    /// The name being published.
    spec fn publication_from(&self) -> Name;

    /// The identity the publication may clean up after a link.
    spec fn publication_ident(&self) -> Option<Ino>;
}

/// The continuation of the publication of a missing destination.
pub struct Create {
    #[allow(dead_code)]
    ident: Ghost<Option<Ino>>,
}

impl AwaitsPublication for Create {
    closed spec fn publication_from(&self) -> Name {
        Name::Staging
    }

    closed spec fn publication_ident(&self) -> Option<Ino> {
        self.ident@
    }
}

/// A no-clobber publication of one name onto the destination, not yet
/// started: production's next step is `renameat2(.., RENAME_NOREPLACE)`.
pub struct NoReplace<I: Identity, K: AwaitsPublication> {
    identity: Option<I>,
    then: K,
}

impl<I: Identity, K: AwaitsPublication> NoReplace<I, K> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& self.then.publication_from() != Name::Dest
        &&& ino_of(self.identity) == self.then.publication_ident()
    }

    /// The state waiting for this publication.
    pub closed spec fn then(&self) -> K {
        self.then
    }

    /// The name being published.
    pub open spec fn from(&self) -> Name {
        self.then().publication_from()
    }

    /// The identity the publication may clean up after a link.
    pub open spec fn ident(&self) -> Option<Ino> {
        self.then().publication_ident()
    }

    /// Publish for `then`; crate-private so only protocol machines choose
    /// what is published and with which identity.
    pub(crate) fn new(identity: Option<I>, then: K) -> (r: Self)
        requires
            then.publication_from() != Name::Dest,
            then.publication_ident() == ino_of(identity),
        ensures
            r.then() == then,
    {
        NoReplace { identity, then }
    }

    /// Feed the answer to `renameat2(from, Dest, RENAME_NOREPLACE)`.
    pub fn after_rename<E>(self, renamed: Attempt<E>) -> (r: NoReplaceRenamed<I, E, K>)
        ensures
            r is Link <==> renamed is Unsupported,
            r matches NoReplaceRenamed::Done(d) ==> {
                &&& d.then() == self.then()
                &&& d.trace() == seq![
                    Ev::RenameNoReplace { from: self.from(), res: res_of(renamed) },
                ]
                &&& renamed is Done ==> d.result() == Ok::<Published<E>, E>(Published::Consumed)
                &&& renamed matches Attempt::Failed(e) ==> d.result() == Err::<Published<E>, E>(e)
            },
            r matches NoReplaceRenamed::Link(l) ==> l.then() == self.then(),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost from = self.then.publication_from();
        let ghost seen = seq![Ev::RenameNoReplace { from, res: res_of(renamed) }];
        match renamed {
            Attempt::Done => NoReplaceRenamed::Done(
                NoReplaceDone {
                    result: Ok(Published::Consumed),
                    trace: Ghost(seen),
                    then: self.then,
                },
            ),
            Attempt::Unsupported => NoReplaceRenamed::Link(
                NoReplaceLink { identity: self.identity, then: self.then },
            ),
            Attempt::Failed(e) => NoReplaceRenamed::Done(
                NoReplaceDone { result: Err(e), trace: Ghost(seen), then: self.then },
            ),
        }
    }
}

impl<I: Identity> NoReplace<I, Create> {
    /// Publish the staging name of a run whose publication checks have all
    /// passed (`_ready`) onto a missing destination, entitled to clean up the
    /// identity `guard` (the staging guard) is entitled to after a link.
    pub fn staging(_ready: Ready, guard: &GuardState<I>) -> (r: Self)
        ensures
            r.from() == Name::Staging,
            r.ident() == guard.ident(),
    {
        let identity = guard.identity();
        NoReplace::new(identity, Create { ident: Ghost(ino_of(identity)) })
    }
}

/// After the rename.
pub enum NoReplaceRenamed<I: Identity, E, K: AwaitsPublication> {
    /// Published by the rename, or failed: nothing more to do.
    Done(NoReplaceDone<E, K>),
    /// `RENAME_NOREPLACE` is unsupported here: `linkat(from, Dest)` next.
    Link(NoReplaceLink<I, K>),
}

/// A publication waiting for its fallback hard link.
pub struct NoReplaceLink<I: Identity, K: AwaitsPublication> {
    identity: Option<I>,
    then: K,
}

impl<I: Identity, K: AwaitsPublication> NoReplaceLink<I, K> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& self.then.publication_from() != Name::Dest
        &&& ino_of(self.identity) == self.then.publication_ident()
    }

    /// The state waiting for this publication.
    pub closed spec fn then(&self) -> K {
        self.then
    }

    /// The name being published.
    pub open spec fn from(&self) -> Name {
        self.then().publication_from()
    }

    /// Feed the answer to `linkat(from, Dest)`. A failed link published
    /// nothing; a successful one has published, and the extra name may then
    /// be removed only by a proven removal for the publication's identity.
    pub fn after_link<E>(self, linked: Result<(), E>) -> (r: NoReplaceLinked<I, E, K>)
        ensures
            r is Cleanup <==> linked is Ok,
            r matches NoReplaceLinked::Done(d) ==> {
                &&& d.then() == self.then()
                &&& linked matches Err(e) && d.result() == Err::<Published<E>, E>(e)
            },
            r matches NoReplaceLinked::Cleanup(removal) ==> removal.then().then() == self.then(),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost from = self.then.publication_from();
        let ghost seen = seq![
            Ev::RenameNoReplace { from, res: Res::Unsupported },
            Ev::Link { from, ok: linked is Ok },
        ];
        match linked {
            Ok(()) => NoReplaceLinked::Cleanup(
                ProvenRemoval::new(
                    self.identity,
                    NoReplaceCleanup { seen: Ghost(seen), then: self.then },
                ),
            ),
            Err(e) => NoReplaceLinked::Done(
                NoReplaceDone { result: Err(e), trace: Ghost(seen), then: self.then },
            ),
        }
    }
}

/// After the hard link.
pub enum NoReplaceLinked<I: Identity, E, K: AwaitsPublication> {
    /// The link failed: nothing was published.
    Done(NoReplaceDone<E, K>),
    /// The link published; run the proven removal of the extra name, then
    /// feed its result back with [`RemovalDone::after_cleanup`].
    Cleanup(ProvenRemoval<I, NoReplaceCleanup<K>>),
}

/// A publication that has linked and is waiting for its cleanup: the
/// continuation of the removal of the extra name.
pub struct NoReplaceCleanup<K: AwaitsPublication> {
    #[allow(dead_code)]
    seen: Ghost<Seq<Ev>>,
    then: K,
}

impl<K: AwaitsPublication> NoReplaceCleanup<K> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& self.then.publication_from() != Name::Dest
        &&& self.seen@ == seq![
            Ev::RenameNoReplace { from: self.then.publication_from(), res: Res::Unsupported },
            Ev::Link { from: self.then.publication_from(), ok: true },
        ]
    }

    /// The state waiting for the publication.
    pub closed spec fn then(&self) -> K {
        self.then
    }
}

impl<K: AwaitsPublication> AwaitsRemoval for NoReplaceCleanup<K> {
    closed spec fn removal_name(&self) -> Name {
        self.then.publication_from()
    }

    closed spec fn removal_ident(&self) -> Option<Ino> {
        self.then.publication_ident()
    }
}

impl<E, K: AwaitsPublication> RemovalDone<E, NoReplaceCleanup<K>> {
    /// Feed back the cleanup of the extra name. The publication stands
    /// whatever the removal did; it only decides what was left behind.
    pub fn after_cleanup(self) -> (d: NoReplaceDone<E, K>)
        ensures
            d.then() == self.then().then(),
            d.result() matches Ok(p) && removal_of(p) == self.outcome(),
    {
        let ghost removal_seen = self.trace();
        let (cleanup, outcome) = self.into_parts();
        proof {
            use_type_invariant(&cleanup);
        }
        let result = match outcome {
            Removal::Removed => Published::Consumed,
            Removal::Left(left) => Published::LeftBehind(left),
        };
        let ghost trace = cleanup.seen@ + removal_seen;
        proof {
            assert(trace.skip(2) =~= removal_seen);
        }
        NoReplaceDone { result: Ok(result), trace: Ghost(trace), then: cleanup.then }
    }
}

/// A finished no-clobber publication, with the state waiting for it.
pub struct NoReplaceDone<E, K: AwaitsPublication> {
    result: Result<Published<E>, E>,
    #[allow(dead_code)]
    trace: Ghost<Seq<Ev>>,
    then: K,
}

impl<E, K: AwaitsPublication> NoReplaceDone<E, K> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& self.then.publication_from() != Name::Dest
        &&& noreplace_trace(
            self.trace@,
            self.then.publication_from(),
            self.then.publication_ident(),
            self.result,
        )
    }

    /// What the publication asked for.
    pub closed spec fn trace(&self) -> Seq<Ev> {
        self.trace@
    }

    /// The state waiting for this publication.
    pub closed spec fn then(&self) -> K {
        self.then
    }

    /// The name that was published.
    pub open spec fn from(&self) -> Name {
        self.then().publication_from()
    }

    /// The identity the cleanup was entitled to.
    pub open spec fn ident(&self) -> Option<Ino> {
        self.then().publication_ident()
    }

    /// Published (and what was left behind), or the error that kept anything
    /// from being published.
    pub closed spec fn result(&self) -> Result<Published<E>, E> {
        self.result
    }

    /// The waiting state and the result, together with the publication's
    /// contract; only the state's own kernel step calls this.
    pub(crate) fn into_parts(self) -> (r: (K, Result<Published<E>, E>))
        ensures
            r.0 == self.then(),
            r.1 == self.result(),
            self.from() != Name::Dest,
            noreplace_trace(self.trace(), self.from(), self.ident(), r.1),
    {
        proof {
            use_type_invariant(&self);
        }
        (self.then, self.result)
    }
}

impl<E> NoReplaceDone<E, Create> {
    /// The result of publishing a missing destination, together with the
    /// publication's contract. Something was published exactly when the
    /// result is `Ok`, and then the staging name is either consumed or
    /// reported as left behind, so `guard` (the staging guard) is disarmed;
    /// otherwise it is left exactly as it was, still owing the removal of
    /// this run's unpublished file.
    pub fn into_result<I: Identity>(self, guard: &mut GuardState<I>) -> (r: Result<Published<E>, E>)
        ensures
            r == self.result(),
            self.from() == Name::Staging,
            noreplace_trace(self.trace(), self.from(), self.ident(), r),
            r is Ok ==> !final(guard).is_armed(),
            r is Err ==> *final(guard) == *old(guard),
            final(guard).name() == old(guard).name(),
            final(guard).ident() == old(guard).ident(),
    {
        let (_create, result) = self.into_parts();
        if result.is_ok() {
            guard.disarm();
        }
        result
    }
}

/// The publication made `n` visible at the destination.
pub open spec fn published(t: Seq<Ev>, n: Name) -> bool {
    t.contains(Ev::RenameNoReplace { from: n, res: Res::Ok }) || t.contains(
        Ev::Link { from: n, ok: true },
    )
}

/// The name `n` was consumed: renamed away, or unlinked after a link.
pub open spec fn consumed(t: Seq<Ev>, n: Name) -> bool {
    t.contains(Ev::RenameNoReplace { from: n, res: Res::Ok }) || t.contains(
        Ev::Unlink { name: n, ok: true },
    )
}

/// NR1–NR4 for every trace a no-clobber publication can produce.
pub proof fn theorem_noreplace<E>(
    t: Seq<Ev>,
    n: Name,
    x: Option<Ino>,
    result: Result<Published<E>, E>,
)
    requires
        n != Name::Dest,
        noreplace_trace(t, n, x, result),
    ensures
        proven_unlinks(t, None, None),
        forall|i: int|
            0 <= i < t.len() && #[trigger] is_unlink_ev(t[i]) ==> {
                &&& x is Some
                &&& 2 <= i
                &&& t[i - 2] == (Ev::Link { from: n, ok: true })
                &&& t[i - 1] == (Ev::Probe { name: n, ino: x->Some_0, holds: true })
                &&& t[i] matches Ev::Unlink { name, .. } && name == n
            },
        x is None ==> !t.contains(Ev::Unlink { name: n, ok: true }) && !t.contains(
            Ev::Unlink { name: n, ok: false },
        ),
        links_are_fallbacks(t),
        result is Err <==> !published(t, n),
        result == Ok::<Published<E>, E>(Published::Consumed) <==> consumed(t, n),
{
    let rnr_ok = Ev::RenameNoReplace { from: n, res: Res::Ok };
    let link_ok = Ev::Link { from: n, ok: true };
    let unlink_ok = Ev::Unlink { name: n, ok: true };
    let unlink_err = Ev::Unlink { name: n, ok: false };
    if t.len() >= 2 && t[0] == (Ev::RenameNoReplace { from: n, res: Res::Unsupported }) && t[1]
        == link_ok && result is Ok {
        let p = result->Ok_0;
        let c = t.skip(2);
        assert(removal_trace(c, n, x, removal_of(p)));
        assert(t =~= seq![t[0], t[1]] + c);
        assert(t.contains(link_ok)) by {
            assert(t[1] == link_ok);
        }
        assert forall|i: int| 0 <= i < t.len() && #[trigger] is_unlink_ev(t[i]) implies {
            &&& unlink_proven_at(t, i, None, None)
            &&& x is Some
            &&& 2 <= i
            &&& t[i - 2] == (Ev::Link { from: n, ok: true })
            &&& t[i - 1] == (Ev::Probe { name: n, ino: x->Some_0, holds: true })
            &&& t[i] matches Ev::Unlink { name, .. } && name == n
        } by {
            assert(i == 3);
            assert(t.take(2) =~= seq![t[0], t[1]]);
            assert(entitled(t.take(2), n, x->Some_0, None, None));
        }
        assert forall|i: int|
            0 <= i < t.len() && #[trigger] crate::publish::trace::is_link(
                t[i],
            ) implies crate::publish::trace::link_after_unsupported(t, i) by {
            if i >= 2 {
                assert(t[i] == c[i - 2]);
            }
        }
        assert(!t.contains(rnr_ok)) by {
            if t.contains(rnr_ok) {
                let i = choose|i: int| 0 <= i < t.len() && t[i] == rnr_ok;
                assert(t[i] == c[i - 2]);
            }
        }
        if t.contains(unlink_ok) {
            let i = choose|i: int| 0 <= i < t.len() && t[i] == unlink_ok;
            assert(t[i] == c[i - 2]);
        }
        if p is Consumed {
            assert(t[3] == unlink_ok);
        }
        if x is None {
            assert(c.len() == 0);
            if t.contains(unlink_err) {
                let i = choose|i: int| 0 <= i < t.len() && t[i] == unlink_err;
                assert(t[i] == c[i - 2]);
            }
        }
    } else {
        if t.contains(link_ok) {
            let i = choose|i: int| 0 <= i < t.len() && t[i] == link_ok;
        }
        if t.contains(unlink_ok) {
            let i = choose|i: int| 0 <= i < t.len() && t[i] == unlink_ok;
        }
        if t.contains(rnr_ok) {
            let i = choose|i: int| 0 <= i < t.len() && t[i] == rnr_ok;
        }
        if result is Ok {
            assert(t[0] == rnr_ok);
        }
        assert forall|i: int| 0 <= i < t.len() && #[trigger] is_unlink_ev(t[i]) implies unlink_proven_at(
            t,
            i,
            None,
            None,
        ) by {}
    }
}

/// Trigger alias, so the theorem's quantifiers fire on the same term as
/// [`crate::publish::trace::proven_unlinks`].
pub open spec fn is_unlink_ev(e: Ev) -> bool {
    crate::publish::trace::is_unlink(e)
}

} // verus!
