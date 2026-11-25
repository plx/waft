//! No-clobber publication of a staging or displaced name onto the
//! destination: `RENAME_NOREPLACE`, with a hard link as the fallback where
//! the filesystem does not implement the flag.
//!
//! Production performs each step the machine names and feeds the answer
//! back; the typestates make any other order a compile error. Every
//! [`NoReplaceDone`] carries `noreplace_trace` as a type invariant, from
//! which `theorem_noreplace` derives:
//!
//! * **NR1** every unlink is issued right after a probe proved the extra name
//!   still holds the identity the publication was given, and only after that
//!   publication's own link succeeded; with no identity there is no unlink;
//! * **NR2** a hard link is made only after `RENAME_NOREPLACE` was reported
//!   unsupported — any other rename error ends the publication;
//! * **NR3** the result is an error exactly when nothing was published (no
//!   successful rename and no successful link), so a caller never reports a
//!   failure for a publication that happened, nor success for one that did
//!   not;
//! * **NR4** the result is `Consumed` exactly when the source name is gone:
//!   the rename succeeded, or the extra name's unlink did.
//!
//! The filesystem-level consequence (the destination entry present before
//! the call is never replaced or removed) is proven in
//! [`crate::publish::model`].
//!
//! Trust: production classifies the rename's errno with its unsupported-flag
//! table, and runs the link and the cleanup on the names this machine was
//! created for.

#[allow(unused_imports)]
use vstd::prelude::*;

use crate::publish::ordering::Ready;
use crate::publish::removal::{ProvenRemoval, RemovalDone};
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

/// A no-clobber publication of one name onto the destination, not yet
/// started: production's next step is `renameat2(.., RENAME_NOREPLACE)`.
pub struct NoReplace<I> {
    identity: Option<I>,
    #[allow(dead_code)]
    from: Ghost<Name>,
}

impl<I> NoReplace<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        self.from@ != Name::Dest
    }
}

impl<I: Identity> NoReplace<I> {
    /// The name being published.
    pub closed spec fn from(&self) -> Name {
        self.from@
    }

    /// The identity the publication may clean up after a link.
    pub closed spec fn ident(&self) -> Option<Ino> {
        ino_of(self.identity)
    }

    /// Publish the staging name of a run whose publication checks have all
    /// passed (`_ready`), entitled to clean up `identity` (the staging
    /// guard's) after a link.
    pub fn staging(_ready: Ready, identity: Option<I>) -> (r: Self)
        ensures
            r.from() == Name::Staging,
            r.ident() == ino_of(identity),
    {
        NoReplace { identity, from: Ghost(Name::Staging) }
    }

    /// Publish `from`; crate-private so only protocol machines choose it.
    pub(crate) fn new(identity: Option<I>, Ghost(from): Ghost<Name>) -> (r: Self)
        requires
            from != Name::Dest,
        ensures
            r.from() == from,
            r.ident() == ino_of(identity),
    {
        NoReplace { identity, from: Ghost(from) }
    }

    /// Feed the answer to `renameat2(from, Dest, RENAME_NOREPLACE)`.
    pub fn after_rename<E>(self, renamed: Attempt<E>) -> (r: NoReplaceRenamed<I, E>)
        ensures
            r is Link <==> renamed is Unsupported,
            r matches NoReplaceRenamed::Done(d) ==> {
                &&& d.from() == self.from()
                &&& d.ident() == self.ident()
                &&& d.trace() == seq![Ev::RenameNoReplace { from: self.from(), res: res_of(renamed) }]
                &&& renamed is Done ==> d.result() == Ok::<Published<E>, E>(Published::Consumed)
                &&& renamed matches Attempt::Failed(e) ==> d.result() == Err::<Published<E>, E>(e)
            },
            r matches NoReplaceRenamed::Link(l) ==> l.from() == self.from() && l.ident()
                == self.ident(),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost from = self.from@;
        let ghost ident = ino_of(self.identity);
        let ghost renamed_ev = Ev::RenameNoReplace { from, res: res_of(renamed) };
        let ghost seen = seq![renamed_ev];
        match renamed {
            Attempt::Done => NoReplaceRenamed::Done(
                NoReplaceDone {
                    result: Ok(Published::Consumed),
                    trace: Ghost(seen),
                    from: self.from,
                    ident: Ghost(ident),
                },
            ),
            Attempt::Unsupported => NoReplaceRenamed::Link(
                NoReplaceLink {
                    identity: self.identity,
                    from: self.from,
                    seen: Ghost(seen),
                },
            ),
            Attempt::Failed(e) => NoReplaceRenamed::Done(
                NoReplaceDone {
                    result: Err(e),
                    trace: Ghost(seen),
                    from: self.from,
                    ident: Ghost(ident),
                },
            ),
        }
    }
}

/// After the rename.
pub enum NoReplaceRenamed<I, E> {
    /// Published by the rename, or failed: nothing more to do.
    Done(NoReplaceDone<E>),
    /// `RENAME_NOREPLACE` is unsupported here: `linkat(from, Dest)` next.
    Link(NoReplaceLink<I>),
}

/// A publication waiting for its fallback hard link.
pub struct NoReplaceLink<I> {
    identity: Option<I>,
    #[allow(dead_code)]
    from: Ghost<Name>,
    #[allow(dead_code)]
    seen: Ghost<Seq<Ev>>,
}

impl<I> NoReplaceLink<I> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& self.from@ != Name::Dest
        &&& self.seen@ == seq![Ev::RenameNoReplace { from: self.from@, res: Res::Unsupported }]
    }
}

impl<I: Identity> NoReplaceLink<I> {
    /// The name being published.
    pub closed spec fn from(&self) -> Name {
        self.from@
    }

    /// The identity the publication may clean up.
    pub closed spec fn ident(&self) -> Option<Ino> {
        ino_of(self.identity)
    }

    /// Feed the answer to `linkat(from, Dest)`. A failed link published
    /// nothing; a successful one has published, and the extra name may then
    /// be removed only by a proven removal for the publication's identity.
    pub fn after_link<E>(self, linked: Result<(), E>) -> (r: NoReplaceLinked<I, E>)
        ensures
            r is Cleanup <==> linked is Ok,
            r matches NoReplaceLinked::Done(d) ==> {
                &&& d.from() == self.from()
                &&& d.ident() == self.ident()
                &&& linked matches Err(e) && d.result() == Err::<Published<E>, E>(e)
            },
            r matches NoReplaceLinked::Cleanup(c, removal) ==> {
                &&& c.from() == self.from()
                &&& c.ident() == self.ident()
                &&& removal.name() == self.from()
                &&& removal.ident() == self.ident()
            },
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost from = self.from@;
        let ghost ident = ino_of(self.identity);
        let ghost seen = self.seen@.push(Ev::Link { from, ok: linked is Ok });
        proof {
            assert(seen =~= seq![
                Ev::RenameNoReplace { from, res: Res::Unsupported },
                Ev::Link { from, ok: linked is Ok },
            ]);
        }
        match linked {
            Ok(()) => NoReplaceLinked::Cleanup(
                NoReplaceCleanup { from: self.from, ident: Ghost(ident), seen: Ghost(seen) },
                ProvenRemoval::new(self.identity, self.from),
            ),
            Err(e) => NoReplaceLinked::Done(
                NoReplaceDone {
                    result: Err(e),
                    trace: Ghost(seen),
                    from: self.from,
                    ident: Ghost(ident),
                },
            ),
        }
    }
}

/// After the hard link.
pub enum NoReplaceLinked<I, E> {
    /// The link failed: nothing was published.
    Done(NoReplaceDone<E>),
    /// The link published; run the proven removal of the extra name, then
    /// feed its result to the cleanup state.
    Cleanup(NoReplaceCleanup, ProvenRemoval<I>),
}

/// A publication that has linked and is waiting for its cleanup.
pub struct NoReplaceCleanup {
    #[allow(dead_code)]
    from: Ghost<Name>,
    #[allow(dead_code)]
    ident: Ghost<Option<Ino>>,
    #[allow(dead_code)]
    seen: Ghost<Seq<Ev>>,
}

impl NoReplaceCleanup {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& self.from@ != Name::Dest
        &&& self.seen@ == seq![
            Ev::RenameNoReplace { from: self.from@, res: Res::Unsupported },
            Ev::Link { from: self.from@, ok: true },
        ]
    }

    /// The name being published.
    pub closed spec fn from(&self) -> Name {
        self.from@
    }

    /// The identity the cleanup is entitled to.
    pub closed spec fn ident(&self) -> Option<Ino> {
        self.ident@
    }

    /// Feed the removal handed out with this state. The publication stands
    /// whatever the removal did; it only decides what was left behind.
    pub fn after_cleanup<E>(self, removal: RemovalDone<E>) -> (d: NoReplaceDone<E>)
        requires
            removal.name() == self.from(),
            removal.ident() == self.ident(),
        ensures
            d.from() == self.from(),
            d.ident() == self.ident(),
            d.result() matches Ok(p) && removal_of(p) == removal.outcome(),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost removal_trace_seen = removal.trace();
        let outcome = removal.into_outcome();
        let result = match outcome {
            Removal::Removed => Published::Consumed,
            Removal::Left(left) => Published::LeftBehind(left),
        };
        let ghost trace = self.seen@ + removal_trace_seen;
        proof {
            assert(trace.skip(2) =~= removal_trace_seen);
        }
        NoReplaceDone { result: Ok(result), trace: Ghost(trace), from: self.from, ident: self.ident }
    }
}

/// A finished no-clobber publication.
pub struct NoReplaceDone<E> {
    result: Result<Published<E>, E>,
    #[allow(dead_code)]
    trace: Ghost<Seq<Ev>>,
    #[allow(dead_code)]
    from: Ghost<Name>,
    #[allow(dead_code)]
    ident: Ghost<Option<Ino>>,
}

impl<E> NoReplaceDone<E> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& self.from@ != Name::Dest
        &&& noreplace_trace(self.trace@, self.from@, self.ident@, self.result)
    }

    /// What the publication asked for.
    pub closed spec fn trace(&self) -> Seq<Ev> {
        self.trace@
    }

    /// The name that was published.
    pub closed spec fn from(&self) -> Name {
        self.from@
    }

    /// The identity the cleanup was entitled to.
    pub closed spec fn ident(&self) -> Option<Ino> {
        self.ident@
    }

    /// Published (and what was left behind), or the error that kept anything
    /// from being published.
    pub closed spec fn result(&self) -> Result<Published<E>, E> {
        self.result
    }

    /// The result, together with the publication's contract.
    pub fn into_result(self) -> (r: Result<Published<E>, E>)
        ensures
            r == self.result(),
            self.from() != Name::Dest,
            noreplace_trace(self.trace(), self.from(), self.ident(), r),
    {
        proof {
            use_type_invariant(&self);
        }
        self.result
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
        assert forall|i: int| 0 <= i < t.len() && #[trigger] is_unlink_ev(t[i]) implies unlink_proven_at(
            t,
            i,
            None,
            None,
        ) by {
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
