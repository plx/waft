//! Proven removal of a name, and the guard state that decides when a
//! removal is owed.
//!
//! A *proven removal* is the only way any publication protocol removes a
//! directory entry: it is entitled to exactly one identity, it probes that
//! the name still holds that identity, and it unlinks only if the probe said
//! so. Without an identity it does nothing at all. Its whole contract is
//! `removal_trace`, which every [`RemovalDone`] carries as a type invariant.
//!
//! [`GuardState`] is the decision state of production's `AnchoredTempGuard`:
//! which identity the guarded name may be unlinked for, and whether a removal
//! is still owed. Theorems, stated as postconditions:
//!
//! * a guard is never re-armed: every `&mut` method ensures
//!   `final.is_armed() ==> old.is_armed()`;
//! * taking a removal (explicitly or on drop) disarms the guard, and a drop
//!   asks for a removal only while the guard is armed, so a guard issues at
//!   most one removal and none after an explicit one;
//! * a removal is always entitled to exactly the guard's current identity
//!   and name;
//! * the guard's identity is re-aimed only by kernel protocol steps
//!   (`GuardState::now_holds` is crate-private), which call it only after
//!   proving the name holds the new inode — see the replacement machine.
//!
//! Trust: production probes the name the removal names, against
//! [`RemovalProbe::identity`], and unlinks exactly that name when told to.

#[allow(unused_imports)]
use vstd::prelude::*;

#[allow(unused_imports)]
use crate::publish::trace::{Ev, Identity, Ino, Leftover, Name, Removal};
#[cfg(verus_keep_ghost)]
use crate::publish::trace::{ino_of, removal_trace};

verus! {

/// A proven removal of one name, entitled to one identity, not yet started.
pub struct ProvenRemoval<I> {
    identity: Option<I>,
    #[allow(dead_code)]
    name: Ghost<Name>,
}

impl<I: Identity> ProvenRemoval<I> {
    /// The name this removal may unlink.
    pub closed spec fn name(&self) -> Name {
        self.name@
    }

    /// The identity this removal is entitled to remove, if any.
    pub closed spec fn ident(&self) -> Option<Ino> {
        ino_of(self.identity)
    }

    /// A removal of `name` entitled to `identity`.
    pub(crate) fn new(identity: Option<I>, Ghost(name): Ghost<Name>) -> (r: Self)
        ensures
            r.name() == name,
            r.ident() == ino_of(identity),
    {
        ProvenRemoval { identity, name: Ghost(name) }
    }

    /// Begin: without an identity there is nothing that may be removed, so
    /// the removal ends at once; otherwise the name must be probed.
    pub fn start<E>(self) -> (r: RemovalStart<I, E>)
        ensures
            r matches RemovalStart::Done(d) ==> {
                &&& self.ident() is None
                &&& d.name() == self.name()
                &&& d.ident() == self.ident()
                &&& d.outcome() == Removal::<E>::Left(Leftover::Unchecked)
            },
            r matches RemovalStart::Probe(p) ==> {
                &&& p.name() == self.name()
                &&& self.ident() == Some(p.target())
            },
    {
        match self.identity {
            None => RemovalStart::Done(
                RemovalDone {
                    outcome: Removal::Left(Leftover::Unchecked),
                    trace: Ghost(Seq::empty()),
                    name: self.name,
                    ident: Ghost(None),
                },
            ),
            Some(identity) => RemovalStart::Probe(RemovalProbe { identity, name: self.name }),
        }
    }

    /// Give up without any syscall, reporting `error` as the removal's
    /// failure. Production uses this only for its test-only forced failure.
    pub fn abandon<E>(self, error: E) -> (d: RemovalDone<E>)
        ensures
            d.name() == self.name(),
            d.ident() == self.ident(),
            d.trace().len() == 0,
            d.outcome() == Removal::Left(Leftover::RemoveFailed(error)),
    {
        RemovalDone {
            outcome: Removal::Left(Leftover::RemoveFailed(error)),
            trace: Ghost(Seq::empty()),
            name: self.name,
            ident: Ghost(ino_of(self.identity)),
        }
    }
}

/// The first step of a proven removal.
pub enum RemovalStart<I, E> {
    /// Nothing may be removed.
    Done(RemovalDone<E>),
    /// Probe whether the name holds [`RemovalProbe::identity`].
    Probe(RemovalProbe<I>),
}

/// A proven removal waiting for its probe.
pub struct RemovalProbe<I> {
    identity: I,
    #[allow(dead_code)]
    name: Ghost<Name>,
}

impl<I: Identity> RemovalProbe<I> {
    /// The name to probe.
    pub closed spec fn name(&self) -> Name {
        self.name@
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
    pub fn after_probe<E>(self, holds: bool) -> (r: RemovalProbed<E>)
        ensures
            r matches RemovalProbed::Done(d) ==> {
                &&& !holds
                &&& d.name() == self.name()
                &&& d.ident() == Some(self.target())
                &&& d.outcome() == Removal::<E>::Left(Leftover::NotHeld)
            },
            r matches RemovalProbed::Unlink(u) ==> {
                &&& holds
                &&& u.name() == self.name()
                &&& u.target() == self.target()
            },
    {
        let ghost probe = Ev::Probe { name: self.name@, ino: self.identity.ino(), holds };
        let ghost seen = seq![probe];
        if holds {
            RemovalProbed::Unlink(
                RemovalUnlink {
                    name: self.name,
                    target: Ghost(self.identity.ino()),
                    seen: Ghost(seen),
                },
            )
        } else {
            RemovalProbed::Done(
                RemovalDone {
                    outcome: Removal::Left(Leftover::NotHeld),
                    trace: Ghost(seen),
                    name: self.name,
                    ident: Ghost(Some(self.identity.ino())),
                },
            )
        }
    }
}

/// The second step of a proven removal.
pub enum RemovalProbed<E> {
    /// The name could not be proved to hold the identity; it is left alone.
    Done(RemovalDone<E>),
    /// The name was just proved to hold the identity: unlink it.
    Unlink(RemovalUnlink),
}

/// A proven removal whose probe has just succeeded, waiting for the unlink.
pub struct RemovalUnlink {
    #[allow(dead_code)]
    name: Ghost<Name>,
    #[allow(dead_code)]
    target: Ghost<Ino>,
    #[allow(dead_code)]
    seen: Ghost<Seq<Ev>>,
}

impl RemovalUnlink {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        self.seen@ == seq![Ev::Probe { name: self.name@, ino: self.target@, holds: true }]
    }

    /// The name to unlink.
    pub closed spec fn name(&self) -> Name {
        self.name@
    }

    /// The inode the name was just proved to hold.
    pub closed spec fn target(&self) -> Ino {
        self.target@
    }

    /// Feed the unlink's result.
    pub fn after_unlink<E>(self, unlinked: Result<(), E>) -> (d: RemovalDone<E>)
        ensures
            d.name() == self.name(),
            d.ident() == Some(self.target()),
            d.outcome() is Removed <==> unlinked is Ok,
            unlinked matches Err(e) ==> d.outcome() == Removal::Left(Leftover::RemoveFailed(e)),
    {
        proof {
            use_type_invariant(&self);
        }
        let ghost trace = self.seen@.push(Ev::Unlink { name: self.name@, ok: unlinked is Ok });
        let ghost ident = Some(self.target@);
        proof {
            assert(trace =~= seq![
                Ev::Probe { name: self.name@, ino: self.target@, holds: true },
                Ev::Unlink { name: self.name@, ok: unlinked is Ok },
            ]);
        }
        match unlinked {
            Ok(()) => RemovalDone {
                outcome: Removal::Removed,
                trace: Ghost(trace),
                name: self.name,
                ident: Ghost(ident),
            },
            Err(e) => RemovalDone {
                outcome: Removal::Left(Leftover::RemoveFailed(e)),
                trace: Ghost(trace),
                name: self.name,
                ident: Ghost(ident),
            },
        }
    }
}

/// A finished proven removal: its outcome, and (ghost) what it asked for.
pub struct RemovalDone<E> {
    outcome: Removal<E>,
    #[allow(dead_code)]
    trace: Ghost<Seq<Ev>>,
    #[allow(dead_code)]
    name: Ghost<Name>,
    #[allow(dead_code)]
    ident: Ghost<Option<Ino>>,
}

impl<E> RemovalDone<E> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        removal_trace(self.trace@, self.name@, self.ident@, self.outcome)
    }

    /// The probes and syscalls the removal asked for.
    pub closed spec fn trace(&self) -> Seq<Ev> {
        self.trace@
    }

    /// The name the removal was for.
    pub closed spec fn name(&self) -> Name {
        self.name@
    }

    /// The identity the removal was entitled to.
    pub closed spec fn ident(&self) -> Option<Ino> {
        self.ident@
    }

    /// How the removal ended.
    pub closed spec fn outcome(&self) -> Removal<E> {
        self.outcome
    }

    /// The outcome, together with the removal's contract.
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
    pub fn staging(identity: Option<I>) -> (g: Self)
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

    /// The identity the guard may remove, for publishing the same file under
    /// another name.
    pub fn identity(&self) -> (r: Option<I>)
        ensures
            ino_of(r) == self.ident(),
    {
        self.identity
    }

    /// Stop guarding: the caller has consumed the name, or it holds a file
    /// this run must not touch.
    pub fn disarm(&mut self)
        ensures
            !final(self).is_armed(),
            final(self).name() == old(self).name(),
            final(self).ident() == old(self).ident(),
    {
        self.armed = false;
    }

    /// The removal an explicit `remove_now` performs. Tracking stops either
    /// way, so the guard can never ask for a second removal.
    pub fn take_removal(&mut self) -> (r: ProvenRemoval<I>)
        ensures
            !final(self).is_armed(),
            final(self).name() == old(self).name(),
            final(self).ident() == old(self).ident(),
            r.name() == old(self).name(),
            r.ident() == old(self).ident(),
    {
        self.armed = false;
        ProvenRemoval::new(self.identity, self.name)
    }

    /// The removal a drop performs: one only while the guard is armed.
    pub fn drop_removal(&mut self) -> (r: Option<ProvenRemoval<I>>)
        ensures
            r is Some <==> old(self).is_armed(),
            !final(self).is_armed(),
            final(self).name() == old(self).name(),
            final(self).ident() == old(self).ident(),
            r matches Some(removal) ==> removal.name() == old(self).name() && removal.ident()
                == old(self).ident(),
    {
        if self.armed {
            Some(self.take_removal())
        } else {
            None
        }
    }

    /// Re-aim the guard at `verified` after an exchange moved it under the
    /// guarded name. Crate-private: only protocol steps that have just proved
    /// the name holds `verified` call it.
    pub(crate) fn now_holds(&mut self, verified: I)
        ensures
            final(self).ident() == Some(verified.ino()),
            final(self).is_armed() == old(self).is_armed(),
            final(self).name() == old(self).name(),
    {
        self.identity = Some(verified);
    }
}

} // verus!
