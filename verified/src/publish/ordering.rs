//! The order of the checks that gate a prepared staging file's publication.
//!
//! A prepared staging file may only be published once, in this order, the
//! source handle was found unchanged after the copy, the staging file was
//! given the source's mode and synced, the caller's pre-publication callback
//! (the index-lock recheck) returned `Ok`, the destination parent was
//! revalidated, and the staging name was proved to still hold the prepared
//! file. Each check is one typestate; the last one yields [`Ready`], and
//! [`Ready`] is the only way to start either publication protocol
//! ([`crate::publish::noreplace::NoReplace::staging`] for a missing
//! destination, [`crate::publish::replace::Exchange::after_destination_verified`]
//! for a replacement). The permission-repair machine gates its `fchmod` the
//! same way.
//!
//! The chain starts when the staging file is created: [`Staged::created`]
//! returns both the first state and the staging name's guard, so production
//! gets exactly one staging guard per check chain.
//!
//! What is enforced, and by whom:
//!
//! * **O1** the order is enforced by Rust: each step consumes its typestate
//!   and is the only way to the next, so no destination mutation that needs a
//!   [`Ready`] can be reached in the code without passing every step;
//! * **O2** (Verus) each step passes an error through unchanged and is `Ok`
//!   exactly when the check it is fed was `Ok`;
//! * **O3** (Verus) each step records exactly its own check on top of the
//!   checks before it (`next.trace() == self.trace().push(..)`), so a state's
//!   trace is the sequence of steps actually taken to reach it — a step that
//!   skipped or reordered a check would break its postcondition;
//! * **O4** (Verus) hence `Ready::trace` is exactly the six checks in order:
//!   the staging name is proved after the callback and the parent
//!   revalidation, immediately before publication.
//!
//! Trust: the typestates fix the order in which results are fed back, not the
//! syscalls themselves; that each result is the real outcome of the named
//! check, run at that point, and that the callback is called once, is
//! production's part.

#[allow(unused_imports)]
use vstd::prelude::*;

use crate::publish::removal::GuardState;
#[cfg(verus_keep_ghost)]
use crate::publish::trace::ino_of;
#[allow(unused_imports)]
use crate::publish::trace::{Ev, Identity, Name};

verus! {

/// The checks a staging file passes before it may be published, in order.
pub open spec fn staging_checks() -> Seq<Ev> {
    seq![
        Ev::SourceUnchanged,
        Ev::StagingMode,
        Ev::StagingSynced,
        Ev::BeforePublish,
        Ev::ParentRevalidated,
        Ev::StagingIsPrepared,
    ]
}

/// A staging file has been created exclusively and guarded; production fills
/// it, and its next step is the source-unchanged check.
pub struct Staged {
    #[allow(dead_code)]
    trace: Ghost<Seq<Ev>>,
}

impl Staged {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        self.trace@ == staging_checks().take(0)
    }

    /// The checks that have passed: none yet.
    pub closed spec fn trace(&self) -> Seq<Ev> {
        self.trace@
    }

    /// Begin the checks for a staging file this run has just created with
    /// `O_EXCL`, whose identity is `identity` (`None` if it could not be
    /// read), together with the guard of its name: armed, and entitled to
    /// remove exactly that identity.
    pub fn created<I: Identity>(identity: Option<I>) -> (r: (Staged, GuardState<I>))
        ensures
            r.0.trace() == Seq::<Ev>::empty(),
            r.1.name() == Name::Staging,
            r.1.ident() == ino_of(identity),
            r.1.is_armed(),
    {
        let ghost trace = staging_checks().take(0);
        proof {
            assert(trace =~= Seq::<Ev>::empty());
        }
        (Staged { trace: Ghost(trace) }, GuardState::staging(identity))
    }

    /// Feed the source-unchanged check.
    pub fn after_source_check<E>(self, checked: Result<(), E>) -> (r: Result<StagingCheck<2>, E>)
        ensures
            r is Ok <==> checked is Ok,
            checked matches Err(e) ==> r == Err::<StagingCheck<2>, E>(e),
            r matches Ok(next) ==> next.trace() == self.trace().push(Ev::SourceUnchanged),
    {
        proof {
            use_type_invariant(&self);
        }
        match checked {
            Ok(()) => {
                let ghost trace = staging_checks().take(1);
                proof {
                    assert(trace =~= self.trace@.push(Ev::SourceUnchanged));
                }
                Ok(StagingCheck { trace: Ghost(trace) })
            },
            Err(error) => Err(error),
        }
    }
}

/// The check number `n` (2 to 6) a staging file passes next.
pub open spec fn staging_check(n: int) -> Ev {
    staging_checks()[n - 1]
}

/// A staging file that has passed the first `N - 1` checks; the next is
/// number `N`: 2 = applying the source's mode, 3 = the sync, 4 = the
/// pre-publication callback, 5 = the parent revalidation, 6 = the staging
/// name proof.
pub struct StagingCheck<const N: u64> {
    #[allow(dead_code)]
    trace: Ghost<Seq<Ev>>,
}

impl<const N: u64> StagingCheck<N> {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        &&& 2 <= N <= 6
        &&& self.trace@ == staging_checks().take(N - 1)
    }

    /// The checks that have passed.
    pub closed spec fn trace(&self) -> Seq<Ev> {
        self.trace@
    }

    fn advance<const M: u64, E>(self, checked: Result<(), E>) -> (r: Result<StagingCheck<M>, E>)
        requires
            M == N + 1,
            M <= 6,
        ensures
            r is Ok <==> checked is Ok,
            checked matches Err(e) ==> r == Err::<StagingCheck<M>, E>(e),
            r matches Ok(next) ==> next.trace() == self.trace().push(staging_check(N as int)),
    {
        proof {
            use_type_invariant(&self);
        }
        match checked {
            Ok(()) => {
                let ghost trace = staging_checks().take(N as int);
                proof {
                    assert(trace =~= self.trace@.push(staging_check(N as int)));
                }
                Ok(StagingCheck { trace: Ghost(trace) })
            },
            Err(error) => Err(error),
        }
    }
}

impl StagingCheck<2> {
    /// Feed the result of giving the staging file the source's mode.
    pub fn after_staging_mode<E>(self, applied: Result<(), E>) -> (r: Result<StagingCheck<3>, E>)
        ensures
            r is Ok <==> applied is Ok,
            applied matches Err(e) ==> r == Err::<StagingCheck<3>, E>(e),
            r matches Ok(next) ==> next.trace() == self.trace().push(Ev::StagingMode),
    {
        self.advance(applied)
    }
}

impl StagingCheck<3> {
    /// Feed the result of syncing the staging file.
    pub fn after_staging_sync<E>(self, synced: Result<(), E>) -> (r: Result<StagingCheck<4>, E>)
        ensures
            r is Ok <==> synced is Ok,
            synced matches Err(e) ==> r == Err::<StagingCheck<4>, E>(e),
            r matches Ok(next) ==> next.trace() == self.trace().push(Ev::StagingSynced),
    {
        self.advance(synced)
    }
}

impl StagingCheck<4> {
    /// Feed the pre-publication callback's result.
    pub fn after_before_publish<E>(self, called: Result<(), E>) -> (r: Result<StagingCheck<5>, E>)
        ensures
            r is Ok <==> called is Ok,
            called matches Err(e) ==> r == Err::<StagingCheck<5>, E>(e),
            r matches Ok(next) ==> next.trace() == self.trace().push(Ev::BeforePublish),
    {
        self.advance(called)
    }
}

impl StagingCheck<5> {
    /// Feed the destination parent revalidation.
    pub fn after_parent_check<E>(self, checked: Result<(), E>) -> (r: Result<StagingCheck<6>, E>)
        ensures
            r is Ok <==> checked is Ok,
            checked matches Err(e) ==> r == Err::<StagingCheck<6>, E>(e),
            r matches Ok(next) ==> next.trace() == self.trace().push(Ev::ParentRevalidated),
    {
        self.advance(checked)
    }
}

impl StagingCheck<6> {
    /// Feed the proof that the staging name still holds the prepared file.
    pub fn after_staging_proof<E>(self, checked: Result<(), E>) -> (r: Result<Ready, E>)
        ensures
            r is Ok <==> checked is Ok,
            checked matches Err(e) ==> r == Err::<Ready, E>(e),
            r matches Ok(ready) ==> ready.trace() == self.trace().push(Ev::StagingIsPrepared),
            r matches Ok(ready) ==> ready.trace() == staging_checks(),
    {
        proof {
            use_type_invariant(&self);
        }
        match checked {
            Ok(()) => {
                let ghost trace = staging_checks();
                proof {
                    assert(trace =~= self.trace@.push(Ev::StagingIsPrepared));
                }
                Ok(Ready { trace: Ghost(trace) })
            },
            Err(error) => Err(error),
        }
    }
}

/// Every check passed: the staging file may be published now. Consumed by
/// the first step of either publication protocol.
pub struct Ready {
    #[allow(dead_code)]
    trace: Ghost<Seq<Ev>>,
}

impl Ready {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        self.trace@ == staging_checks()
    }

    /// The checks that passed, in order.
    pub closed spec fn trace(&self) -> Seq<Ev> {
        self.trace@
    }
}

} // verus!
