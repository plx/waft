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
//! Proven (O1–O4): no destination mutation of any kind — no-clobber
//! publication, exchange, displacement, or `fchmod` — can be issued before
//! the pre-publication callback has returned `Ok`, and the staging name is
//! proved after the callback and the parent revalidation, immediately before
//! publication; `Ready::trace` is exactly that sequence.
//!
//! Trust: production feeds each check's real result, and calls the callback
//! once (the token for the next step is consumed by the step).

#[allow(unused_imports)]
use vstd::prelude::*;

#[allow(unused_imports)]
use crate::publish::trace::Ev;

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

/// A staging file has been created exclusively, filled, and guarded;
/// production's next step is the source-unchanged check.
pub struct Staged {
    #[allow(dead_code)]
    trace: Ghost<Seq<Ev>>,
}

impl Staged {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        self.trace@ == staging_checks().take(0)
    }

    /// Begin the checks for a freshly prepared and guarded staging file.
    pub fn prepared() -> (r: Staged) {
        let ghost trace = staging_checks().take(0);
        Staged { trace: Ghost(trace) }
    }

    /// Feed the source-unchanged check.
    pub fn after_source_check<E>(self, checked: Result<(), E>) -> (r: Result<StagingCheck<2>, E>)
        ensures
            r is Ok <==> checked is Ok,
            checked matches Err(e) ==> r == Err::<StagingCheck<2>, E>(e),
    {
        match checked {
            Ok(()) => {
                let ghost trace = staging_checks().take(1);
                Ok(StagingCheck { trace: Ghost(trace) })
            },
            Err(error) => Err(error),
        }
    }
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
    {
        proof {
            use_type_invariant(&self);
        }
        match checked {
            Ok(()) => {
                let ghost trace = staging_checks().take(N as int);
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
            r matches Ok(ready) ==> ready.trace() == staging_checks(),
    {
        match checked {
            Ok(()) => {
                proof {
                    assert(staging_checks().take(6) =~= staging_checks());
                }
                let ghost trace = staging_checks();
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
