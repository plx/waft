//! Proven removal of a name, and the guard state that decides when a
//! removal is owed.
//!
//! A *proven removal* is the only way any publication protocol removes a
//! directory entry: it is entitled to exactly one identity, it probes that
//! the name still holds that identity, and it unlinks only if the probe said
//! so. Without an identity it does nothing at all. Its whole contract is
//! `removal_trace`, which every [`RemovalDone`] carries as a type invariant.
//!
//! A removal carries the kernel state that asked for it (its *continuation*,
//! a type implementing [`AwaitsRemoval`]), and its finished record can only be
//! fed back to that same state: the record of one removal cannot be handed to
//! a state that asked for another, because the state travels inside the
//! removal. Which name the removal is for and which identity it is entitled to
//! are facts about that continuation, kept as type invariants of every step.
//!
//! [`GuardState`] is the decision state of production's `AnchoredTempGuard`:
//! which identity the guarded name may be unlinked for, and whether a removal
//! is still owed. Proven, as postconditions of every method, per guard state
//! value:
//!
//! * a guard's name and identity never change after it is created, and a
//!   guard is never re-armed (every `&mut` method keeps `name` and `ident` and
//!   ensures `final.is_armed() ==> old.is_armed()`);
//! * production can obtain a removal from a guard only through
//!   [`GuardState::drop_removal`], which hands one out only while the guard is
//!   armed and disarms it, so a guard issues at most one removal on drop and
//!   none after a kernel step disarmed it;
//! * every removal a guard hands out is entitled to exactly the guard's name
//!   and identity.
//!
//! A staging guard is only created together with the staging file's check
//! chain ([`crate::publish::ordering::Staged::created`]); a displaced guard
//! only by the displacement machine. Neither can be re-aimed: kernel steps
//! that remove a different inode from a guarded name build their own proven
//! removal for it and disarm the guard.
//!
//! Trust: production probes the name the removal names, against
//! [`RemovalProbe::identity`], and unlinks exactly that name when told to.
//! Production keeps one guard state per guarded name and never replaces it
//! with another value (the kernel's guarantees are about each value; Rust
//! cannot stop unverified code from overwriting a field).

#[allow(unused_imports)]
use vstd::prelude::*;

#[allow(unused_imports)]
use crate::publish::trace::{Ev, Identity, Ino, Leftover, Name, Removal};
#[cfg(verus_keep_ghost)]
use crate::publish::trace::{ino_of, removal_trace};

verus! {

/// A kernel state waiting for the proven removal it handed out.
///
/// It names the directory entry the removal is for and the only identity the
/// removal is entitled to unlink from it.
pub trait AwaitsRemoval: Sized {
    /// The name the removal is for.
    spec fn removal_name(&self) -> Name;

    /// The identity the removal is entitled to remove, if any.
    spec fn removal_ident(&self) -> Option<Ino>;
}

/// The continuation of a removal a guard owes when it is dropped (or, in
/// tests, removed explicitly): nothing waits for its result.
pub struct Owed {
    #[allow(dead_code)]
    name: Ghost<Name>,
    #[allow(dead_code)]
    ident: Ghost<Option<Ino>>,
}

impl AwaitsRemoval for Owed {
    closed spec fn removal_name(&self) -> Name {
        self.name@
    }

    closed spec fn removal_ident(&self) -> Option<Ino> {
        self.ident@
    }
}

/// A proven removal of one name, entitled to one identity, not yet started.
pub struct ProvenRemoval<I: Identity, K: AwaitsRemoval> {
    identity: Option<I>,
    then: K,
}

impl<I: Identity, K: AwaitsRemoval> ProvenRemoval<I, K> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        ino_of(self.identity) == self.then.removal_ident()
    }

    /// The state waiting for this removal.
    pub closed spec fn then(&self) -> K {
        self.then
    }

    /// The name this removal may unlink.
    pub open spec fn name(&self) -> Name {
        self.then().removal_name()
    }

    /// The identity this removal is entitled to remove, if any.
    pub open spec fn ident(&self) -> Option<Ino> {
        self.then().removal_ident()
    }

    /// A removal for `then`, entitled to `identity`.
    pub(crate) fn new(identity: Option<I>, then: K) -> (r: Self)
        requires
            then.removal_ident() == ino_of(identity),
        ensures
            r.then() == then,
    {
        ProvenRemoval { identity, then }
    }

    /// Begin: without an identity there is nothing that may be removed, so
    /// the removal ends at once; otherwise the name must be probed.
    pub fn start<E>(self) -> (r: RemovalStart<I, E, K>)
        ensures
            r matches RemovalStart::Done(d) ==> {
                &&& self.ident() is None
                &&& d.then() == self.then()
                &&& d.outcome() == Removal::<E>::Left(Leftover::Unchecked)
            },
            r matches RemovalStart::Probe(p) ==> {
                &&& p.then() == self.then()
                &&& self.ident() == Some(p.target())
            },
    {
        proof {
            use_type_invariant(&self);
        }
        match self.identity {
            None => RemovalStart::Done(
                RemovalDone {
                    outcome: Removal::Left(Leftover::Unchecked),
                    trace: Ghost(Seq::empty()),
                    then: self.then,
                },
            ),
            Some(identity) => RemovalStart::Probe(RemovalProbe { identity, then: self.then }),
        }
    }

    /// Give up without any syscall, reporting `error` as the removal's
    /// failure. Production uses this only for its test-only forced failure.
    pub fn abandon<E>(self, error: E) -> (d: RemovalDone<E, K>)
        ensures
            d.then() == self.then(),
            d.trace().len() == 0,
            d.outcome() == Removal::Left(Leftover::RemoveFailed(error)),
    {
        RemovalDone {
            outcome: Removal::Left(Leftover::RemoveFailed(error)),
            trace: Ghost(Seq::empty()),
            then: self.then,
        }
    }
}

/// The first step of a proven removal.
pub enum RemovalStart<I: Identity, E, K: AwaitsRemoval> {
    /// Nothing may be removed.
    Done(RemovalDone<E, K>),
    /// Probe whether the name holds [`RemovalProbe::identity`].
    Probe(RemovalProbe<I, K>),
}

/// A proven removal waiting for its probe.
pub struct RemovalProbe<I: Identity, K: AwaitsRemoval> {
    identity: I,
    then: K,
}

impl<I: Identity, K: AwaitsRemoval> RemovalProbe<I, K> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        self.then.removal_ident() == Some(self.identity.ino())
    }

    /// The state waiting for this removal.
    pub closed spec fn then(&self) -> K {
        self.then
    }

    /// The name to probe.
    pub open spec fn name(&self) -> Name {
        self.then().removal_name()
    }

    /// The inode the name must be proved to hold.
    pub closed spec fn target(&self) -> Ino {
        self.identity.ino()
    }

    /// The identity to probe the name against.
    pub fn identity(&self) -> (r: I)
        ensures
            r.ino() == self.target(),
    {
        self.identity
    }

    /// Feed the probe's answer (`false` for a probe that could not be
    /// completed): only a name proved to hold the identity may be unlinked.
    pub fn after_probe<E>(self, holds: bool) -> (r: RemovalProbed<E, K>)
        ensures
            r matches RemovalProbed::Done(d) ==> {
                &&& !holds
                &&& d.then() == self.then()
                &&& d.ident() == Some(self.target())
                &&& d.outcome() == Removal::<E>::Left(Leftover::NotHeld)
            },
            r matches RemovalProbed::Unlink(u) ==> {
                &&& holds
                &&& u.then() == self.then()
                &&& u.target() == self.target()
            },
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost name = self.then.removal_name();
        let ghost seen = seq![Ev::Probe { name, ino: self.identity.ino(), holds }];
        if holds {
            RemovalProbed::Unlink(
                RemovalUnlink { target: Ghost(self.identity.ino()), then: self.then },
            )
        } else {
            RemovalProbed::Done(
                RemovalDone {
                    outcome: Removal::Left(Leftover::NotHeld),
                    trace: Ghost(seen),
                    then: self.then,
                },
            )
        }
    }
}

/// The second step of a proven removal.
pub enum RemovalProbed<E, K: AwaitsRemoval> {
    /// The name could not be proved to hold the identity; it is left alone.
    Done(RemovalDone<E, K>),
    /// The name was just proved to hold the identity: unlink it.
    Unlink(RemovalUnlink<K>),
}

/// A proven removal whose probe has just succeeded, waiting for the unlink.
pub struct RemovalUnlink<K: AwaitsRemoval> {
    #[allow(dead_code)]
    target: Ghost<Ino>,
    then: K,
}

impl<K: AwaitsRemoval> RemovalUnlink<K> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        self.then.removal_ident() == Some(self.target@)
    }

    /// The state waiting for this removal.
    pub closed spec fn then(&self) -> K {
        self.then
    }

    /// The name to unlink.
    pub open spec fn name(&self) -> Name {
        self.then().removal_name()
    }

    /// The inode the name was just proved to hold.
    pub closed spec fn target(&self) -> Ino {
        self.target@
    }

    /// Feed the unlink's result.
    pub fn after_unlink<E>(self, unlinked: Result<(), E>) -> (d: RemovalDone<E, K>)
        ensures
            d.then() == self.then(),
            d.ident() == Some(self.target()),
            d.outcome() is Removed <==> unlinked is Ok,
            unlinked matches Err(e) ==> d.outcome() == Removal::Left(Leftover::RemoveFailed(e)),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost name = self.then.removal_name();
        let ghost trace = seq![
            Ev::Probe { name, ino: self.target@, holds: true },
            Ev::Unlink { name, ok: unlinked is Ok },
        ];
        match unlinked {
            Ok(()) => RemovalDone {
                outcome: Removal::Removed,
                trace: Ghost(trace),
                then: self.then,
            },
            Err(e) => RemovalDone {
                outcome: Removal::Left(Leftover::RemoveFailed(e)),
                trace: Ghost(trace),
                then: self.then,
            },
        }
    }
}

/// A finished proven removal: its outcome, (ghost) what it asked for, and
/// the state waiting for it.
pub struct RemovalDone<E, K: AwaitsRemoval> {
    outcome: Removal<E>,
    #[allow(dead_code)]
    trace: Ghost<Seq<Ev>>,
    then: K,
}

impl<E, K: AwaitsRemoval> RemovalDone<E, K> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        removal_trace(
            self.trace@,
            self.then.removal_name(),
            self.then.removal_ident(),
            self.outcome,
        )
    }

    /// The probes and syscalls the removal asked for.
    pub closed spec fn trace(&self) -> Seq<Ev> {
        self.trace@
    }

    /// The state waiting for this removal.
    pub closed spec fn then(&self) -> K {
        self.then
    }

    /// The name the removal was for.
    pub open spec fn name(&self) -> Name {
        self.then().removal_name()
    }

    /// The identity the removal was entitled to.
    pub open spec fn ident(&self) -> Option<Ino> {
        self.then().removal_ident()
    }

    /// How the removal ended.
    pub closed spec fn outcome(&self) -> Removal<E> {
        self.outcome
    }

    /// The waiting state and the outcome, together with the removal's
    /// contract; only the state's own kernel step calls this.
    pub(crate) fn into_parts(self) -> (r: (K, Removal<E>))
        ensures
            r.0 == self.then(),
            r.1 == self.outcome(),
            removal_trace(self.trace(), self.name(), self.ident(), r.1),
    {
        proof {
            use_type_invariant(&self);
        }
        (self.then, self.outcome)
    }
}

impl<E> RemovalDone<E, Owed> {
    /// The outcome of a removal nothing waits for, together with the
    /// removal's contract.
    pub fn into_outcome(self) -> (r: Removal<E>)
        ensures
            r == self.outcome(),
            removal_trace(self.trace(), self.name(), self.ident(), r),
    {
        proof {
            use_type_invariant(&self);
        }
        self.outcome
    }
}

/// The decision state of a guard over a staging or displaced name.
pub struct GuardState<I> {
    identity: Option<I>,
    armed: bool,
    #[allow(dead_code)]
    name: Ghost<Name>,
}

impl<I: Identity> GuardState<I> {
    /// The guarded name.
    pub closed spec fn name(&self) -> Name {
        self.name@
    }

    /// The only identity the guard may remove from its name.
    pub closed spec fn ident(&self) -> Option<Ino> {
        ino_of(self.identity)
    }

    /// Whether a removal is still owed.
    pub closed spec fn is_armed(&self) -> bool {
        self.armed
    }

    /// Guard the staging name, entitled to the identity of the file this run
    /// just created there (`None` if it could not be read: never remove).
    /// Crate-private: production gets a staging guard only together with the
    /// staging file's check chain.
    pub(crate) fn staging(identity: Option<I>) -> (g: Self)
        ensures
            g.name() == Name::Staging,
            g.ident() == ino_of(identity),
            g.is_armed(),
    {
        GuardState { identity, armed: true, name: Ghost(Name::Staging) }
    }

    /// Guard the displaced name, entitled to the planned destination inode.
    pub(crate) fn displaced(verified: I) -> (g: Self)
        ensures
            g.name() == Name::Displaced,
            g.ident() == Some(verified.ino()),
            g.is_armed(),
    {
        GuardState { identity: Some(verified), armed: true, name: Ghost(Name::Displaced) }
    }

    /// The identity the guard may remove, for kernel steps that publish the
    /// same file under another name.
    pub(crate) fn identity(&self) -> (r: Option<I>)
        ensures
            ino_of(r) == self.ident(),
    {
        self.identity
    }

    /// Stop guarding: the name has been consumed, or it holds a file this run
    /// must not touch.
    pub fn disarm(&mut self)
        ensures
            !final(self).is_armed(),
            final(self).name() == old(self).name(),
            final(self).ident() == old(self).ident(),
    {
        self.armed = false;
    }

    /// The removal a drop performs: one only while the guard is armed, and
    /// none after that, since it disarms the guard.
    pub fn drop_removal(&mut self) -> (r: Option<ProvenRemoval<I, Owed>>)
        ensures
            r is Some <==> old(self).is_armed(),
            !final(self).is_armed(),
            final(self).name() == old(self).name(),
            final(self).ident() == old(self).ident(),
            r matches Some(removal) ==> removal.name() == old(self).name() && removal.ident()
                == old(self).ident(),
    {
        if self.armed {
            self.armed = false;
            let owed = Owed { name: self.name, ident: Ghost(ino_of(self.identity)) };
            Some(ProvenRemoval::new(self.identity, owed))
        } else {
            None
        }
    }
}

} // verus!
