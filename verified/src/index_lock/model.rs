//! Ghost model of the index-lock signal protocol, and its safety theorems.
//!
//! Everything here is specification and proof: nothing is executed. The
//! model is a transition system with three kinds of steps:
//!
//! * **main** — one step per atomic operation or system call of the main
//!   thread in `GitIndexLock::acquire` and `Drop for GitIndexLock`, i.e. one
//!   step per effect the verified drivers in [`super`] issue (each [`Pc`]
//!   variant cites the driver call and the `executor.rs` code it mirrors);
//! * **signal** — a `SIGINT`/`SIGTERM` delivery at any main-step boundary. The
//!   handler ([`super::on_signal`]) runs to completion as one step: waft is
//!   single-threaded, and the handler's `sa_mask` blocks both signals, so
//!   handlers never nest and the main thread is suspended meanwhile;
//! * **env** — another Git writer creating or removing its own `index.lock`
//!   whenever the name allows it; unless the environment is cooperative
//!   (`coop`), also a process renaming or deleting *our* lock.
//!
//! Parameters: the previous disposition the handler restores ([`Disposition`]:
//! `SIG_DFL`, `SIG_IGN` (handler never installed), or a handler that returns),
//! whether the lock path fits the signal-safe buffer (`stage_ok`), whether
//! the environment is cooperative, and the retry count.
//!
//! # Theorems (about the model, for every finite interleaving of steps)
//!
//! * `theorem_p1_never_remove_foreign_lock` — every removal attempt is
//!   made with the descriptor of the file this acquisition created and the
//!   path it staged, so the identity check can only ever unlink our own
//!   lock; a stale descriptor from an earlier acquisition is never consulted.
//! * `theorem_p2_one_removal_per_acquisition` — unless the previous
//!   disposition is a handler that returns, an acquisition attempts removal
//!   at most once. Exception (proved real by
//!   `example_double_removal_with_returning_handler`): with a returning
//!   previous handler, a second signal between `resume_pending`'s load and
//!   its store of `STAGED` makes the handler remove the lock, and the main
//!   thread then attempts a second removal (still identity-checked).
//! * `theorem_p3_no_stale_lock_on_signal_death` — if the lock path was
//!   staged, the process never dies by a signal with its lock on disk, and no
//!   acquisition ever leaves its lock behind. Exception (proved real by
//!   `example_unstaged_path_leaves_stale_lock`): a path too long for the
//!   buffer is never armed, so a signal while it is held, or even a normal
//!   release, leaves the lock on disk.
//! * `theorem_p4_no_stranded_deferral` — a recorded deferral always has a
//!   consuming check ahead of the main thread in the current acquisition or
//!   release, except one recorded while the main thread is inside
//!   `resume_pending` between its take and its disarm; with `SIG_DFL` that
//!   exception ends in termination by the first signal, so only a returning
//!   previous handler can carry a deferral past the resume (proved real by
//!   `example_stranded_deferral_with_returning_handler`).
//! * `theorem_p7_publication_under_lock` — with a staged path, a
//!   cooperative environment, and a previous disposition that does not
//!   return, publication only ever happens while our lock is on disk and
//!   armed, with no deferral pending.
//!
//! # Trust assumptions
//!
//! * The steps mirror the drivers and `executor.rs` one-to-one (checked by
//!   review; see each [`Pc`] variant). `SeqCst` atomics give one total order.
//! * Waft is single-threaded; the handler is atomic with respect to the main
//!   thread; `raise` under `SIG_DFL` terminates (synchronously outside a
//!   handler, at handler return inside one); under a returning handler it
//!   returns.
//! * The identity-checked unlink (`fstat(LOCK_FD)` vs `lstat(PATH)`, then
//!   `unlink`) is one atomic step: the TOCTOU window between `lstat` and
//!   `unlink` (acknowledged in `executor.rs`) is not modelled. While `LOCK_FD`
//!   is the open descriptor of the created file, identities match exactly
//!   when the path still names that file.
//! * One lock path; handler installation is not modelled separately (before
//!   the first stage the state is `EMPTY` and no lock of ours exists, so the
//!   handler and the default disposition act alike).

#[allow(unused_imports)]
use vstd::prelude::*;

use super::Attempt;
#[cfg(verus_keep_ghost)]
use super::{
    ARM, ARMED, BEGIN_RELEASE, CLAIM, EMPTY, NO_SIGNAL, STAGED, WRITING, spec_defers,
    spec_resume_removes,
};

verus! {

/// Where a deferred-signal check (`resume_pending`) sits.
pub ghost enum Site {
    /// After a `create_new` attempt (`acquire`'s loop check; holds the lock
    /// iff the attempt created the file).
    AfterAttempt,
    /// After giving up and disarming (holds no lock).
    AfterFailure,
    /// After arming (holds the lock).
    AfterArm,
    /// In release, after removal and disarm (holds no lock).
    Release,
}

/// The main thread's next step.
pub ghost enum Pc {
    /// No acquisition in progress. Next: `stage` stores `WRITING` (after
    /// installing the handlers), or `EMPTY` for a path that does not fit.
    Idle,
    /// Next: `StageOps::reset_fd` (`LOCK_FD.store(-1)`).
    ResetFd,
    /// Next: `StageOps::write_path` (copy the path into `PATH`).
    WritePath,
    /// Next: `stage` stores `STAGED`.
    StoreStaged,
    /// Next: `AcquireOps::try_create` (`create_new`).
    Create,
    /// Next: `record_file` (`LOCK_FD.store(fd)`), part of `try_create`.
    Record,
    /// Next: the `take_pending` swap of a `resume_pending` at this site.
    Check(Site),
    /// Next: `AcquireOps::wait` (sleep), then another attempt.
    Wait,
    /// Next: `disarm` after giving up.
    FailDisarm,
    /// Next: `arm` (compare-exchange `STAGED -> ARMED`).
    Arm,
    /// Lock acquired; next: publication (`copy_file`), then release.
    Holding,
    /// Next: `release`'s compare-exchange `ARMED -> STAGED`.
    BeginRelease,
    /// Next: `release`'s identity-checked removal.
    RelRemove,
    /// Next: `release`'s disarm.
    RelDisarm,
    /// Next: closing the lock file descriptor (`drop(self.file.take())`).
    Close,
    /// Next: `resume_pending`'s load of the state (only when holding).
    ResumeLoad(Site),
    /// Next: `resume_pending`'s store of `STAGED`.
    ResumeStoreStaged(Site),
    /// Next: `resume_pending`'s identity-checked removal.
    ResumeRemove(Site),
    /// Next: `resume_pending`'s disarm.
    ResumeDisarm(Site),
    /// Next: `resume_pending`'s restore-and-raise.
    ResumeRaise(Site),
}

/// What `LOCK_FD` holds.
pub ghost enum Fd {
    /// `-1`: `fstat` fails, so removal does nothing.
    Unset,
    /// The open descriptor of the file this acquisition created.
    Current,
    /// A descriptor number left from an earlier, closed lock file.
    Stale,
}

/// What the lock path names on disk.
pub ghost enum Disk {
    /// Nothing.
    Free,
    /// The file this acquisition created.
    Ours,
    /// Another writer's lock, or a lock an earlier acquisition left behind.
    Foreign,
}

/// The disposition waft's handler restores before re-raising.
pub ghost enum Disposition {
    /// `SIG_DFL`: the re-raise terminates the process.
    Default,
    /// `SIG_IGN`: waft never installs its handler; deliveries do nothing.
    Ignore,
    /// A handler that returns: the process survives the re-raise.
    Returning,
}

/// A step of the environment.
pub ghost enum EnvStep {
    /// Another writer creates its lock (if the name is free).
    ForeignCreate,
    /// Another writer removes its lock.
    ForeignRelease,
    /// Non-cooperative: our lock is renamed away and replaced.
    Replace,
    /// Non-cooperative: our lock is deleted.
    Delete,
}

/// One step of an interleaving.
pub ghost enum Step {
    /// A main-thread step; the flag makes a `create_new` fail with an error
    /// other than `AlreadyExists`.
    Main(bool),
    /// Delivery of signal number `n`.
    Signal(int),
    /// An environment step.
    Env(EnvStep),
}

/// The whole state of the model.
pub ghost struct LockModel {
    /// Parameter: the disposition waft's handler restores.
    pub prev: Disposition,
    /// Parameter: the lock path fits the signal-safe buffer.
    pub stage_ok: bool,
    /// Parameter: nobody but us touches our lock.
    pub coop: bool,
    /// Parameter: `INDEX_LOCK_RETRY_DELAYS.len()`.
    pub retries: nat,
    /// `STATE`.
    pub state: u8,
    /// `PENDING_SIGNAL`.
    pub pending: int,
    /// `LOCK_FD`.
    pub fd: Fd,
    /// `PATH` holds this acquisition's lock path.
    pub path_current: bool,
    /// The lock path on disk.
    pub disk: Disk,
    /// The main thread's next step.
    pub pc: Pc,
    /// Outcome of the latest `create_new` attempt.
    pub attempt_kind: Attempt,
    /// Attempts made so far in this acquisition, minus one.
    pub attempt: nat,
    /// The signal the running `resume_pending` took.
    pub resume_sig: int,
    /// Signals whose handler waft has replaced by the previous disposition.
    pub uninstalled: Set<int>,
    /// The process has been terminated by a signal.
    pub terminated: bool,
    /// Ghost: removal attempts in this acquisition.
    pub removals: nat,
    /// Ghost: a removal was attempted with a stale descriptor or path.
    pub unsafe_removal: bool,
    /// Ghost: an acquisition ended with its lock still on disk.
    pub leaked: bool,
    /// Ghost: the pending signal was recorded inside a running resume.
    pub stranded: bool,
    /// Ghost: a publication happened without our lock on disk.
    pub unlocked_publication: bool,
}

/// The initial state, before any acquisition. `disk` must not be ours.
pub open spec fn init(prev: Disposition, stage_ok: bool, coop: bool, retries: nat, disk: Disk) -> LockModel {
    LockModel {
        prev,
        stage_ok,
        coop,
        retries,
        state: EMPTY,
        pending: NO_SIGNAL as int,
        fd: Fd::Unset,
        path_current: false,
        disk: if disk == Disk::Ours {
            Disk::Free
        } else {
            disk
        },
        pc: Pc::Idle,
        attempt_kind: Attempt::Failed,
        attempt: 0,
        resume_sig: NO_SIGNAL as int,
        uninstalled: Set::empty(),
        terminated: false,
        removals: 0,
        unsafe_removal: false,
        leaked: false,
        stranded: false,
        unlocked_publication: false,
    }
}

// ---------------------------------------------------------------------------
// Transitions.
// ---------------------------------------------------------------------------

/// `remove_recorded_lock`: `fstat(LOCK_FD)`, `lstat(PATH)`, compare, unlink.
pub open spec fn remove(m: LockModel) -> LockModel {
    let m1 = LockModel { removals: m.removals + 1, ..m };
    match m.fd {
        Fd::Unset => m1,
        Fd::Current => if m.path_current {
            if m.disk == Disk::Ours {
                LockModel { disk: Disk::Free, ..m1 }
            } else {
                m1
            }
        } else {
            LockModel { unsafe_removal: true, ..m1 }
        },
        Fd::Stale => LockModel { unsafe_removal: true, ..m1 },
    }
}

/// `restore_previous_and_raise(sig)`.
pub open spec fn raise(m: LockModel, sig: int) -> LockModel {
    let m1 = LockModel { uninstalled: m.uninstalled.insert(sig), ..m };
    if m.prev == Disposition::Default {
        LockModel { terminated: true, ..m1 }
    } else {
        m1
    }
}

/// Closing the lock file's descriptor.
pub open spec fn close(fd: Fd) -> Fd {
    if fd == Fd::Current {
        Fd::Stale
    } else {
        fd
    }
}

/// Inside `resume_pending`, after its take and before its disarm.
pub open spec fn in_resume_window(pc: Pc) -> bool {
    pc is ResumeLoad || pc is ResumeStoreStaged || pc is ResumeRemove || pc is ResumeDisarm
}

/// Inside `resume_pending`, after its take.
pub open spec fn in_resume_tail(pc: Pc) -> bool {
    in_resume_window(pc) || pc is ResumeRaise
}

/// A deferred-signal check lies ahead of the main thread in the current
/// acquisition or release.
pub open spec fn redeemable(pc: Pc) -> bool {
    pc is ResetFd || pc is WritePath || pc is StoreStaged || pc is Create || pc is Record || pc is Check
        || pc is Wait || pc is FailDisarm || pc is Arm || pc is RelRemove || pc is RelDisarm
}

/// The handler ([`super::on_signal`]), run to completion.
pub open spec fn handler_run(m: LockModel, sig: int) -> LockModel {
    if spec_defers(m.state) {
        // Defer (compare-exchange `NO_SIGNAL -> sig`); the second load sees
        // the same state, because the main thread is suspended, so return.
        if m.pending == NO_SIGNAL as int {
            LockModel { pending: sig, stranded: m.stranded || in_resume_window(m.pc), ..m }
        } else {
            m
        }
    } else {
        // `claim_armed`, then restore and raise.
        let m1 = if m.state == CLAIM.from {
            remove(LockModel { state: CLAIM.to, ..m })
        } else {
            m
        };
        raise(m1, sig)
    }
}

/// A signal delivery.
pub open spec fn signal_step(m: LockModel, sig: int) -> LockModel {
    if m.terminated || sig <= 0 || m.prev == Disposition::Ignore {
        m
    } else if m.uninstalled.contains(sig) {
        // The previous disposition runs directly.
        if m.prev == Disposition::Default {
            LockModel { terminated: true, ..m }
        } else {
            m
        }
    } else {
        handler_run(m, sig)
    }
}

/// An environment step.
pub open spec fn env_step(m: LockModel, e: EnvStep) -> LockModel {
    if m.terminated {
        m
    } else {
        match e {
            EnvStep::ForeignCreate => if m.disk == Disk::Free {
                LockModel { disk: Disk::Foreign, ..m }
            } else {
                m
            },
            EnvStep::ForeignRelease => if m.disk == Disk::Foreign {
                LockModel { disk: Disk::Free, ..m }
            } else {
                m
            },
            EnvStep::Replace => if !m.coop && m.disk == Disk::Ours {
                LockModel { disk: Disk::Foreign, ..m }
            } else {
                m
            },
            EnvStep::Delete => if !m.coop && m.disk == Disk::Ours {
                LockModel { disk: Disk::Free, ..m }
            } else {
                m
            },
        }
    }
}

/// Whether the check at `site` is told the caller holds the lock.
pub open spec fn holds(site: Site, m: LockModel) -> bool {
    match site {
        Site::AfterAttempt => m.attempt_kind == Attempt::Created,
        Site::AfterFailure => false,
        Site::AfterArm => true,
        Site::Release => false,
    }
}

/// Where the main thread continues when a check finds nothing deferred.
pub open spec fn continue_ok(m: LockModel, site: Site) -> LockModel {
    match site {
        // `acquire`'s loop: arm on success; retry while contended with
        // retries left; otherwise give up and disarm.
        Site::AfterAttempt => if m.attempt_kind == Attempt::Created {
            LockModel { pc: Pc::Arm, ..m }
        } else if m.attempt_kind == Attempt::Contended && m.attempt < m.retries {
            LockModel { pc: Pc::Wait, ..m }
        } else {
            LockModel { pc: Pc::FailDisarm, ..m }
        },
        // `acquire` returns its error.
        Site::AfterFailure => LockModel { pc: Pc::Idle, ..m },
        // `acquire` returns the lock.
        Site::AfterArm => LockModel { pc: Pc::Holding, ..m },
        // `release` returns; `Drop` closes the file.
        Site::Release => LockModel { pc: Pc::Close, ..m },
    }
}

/// Where the main thread continues when it survived a re-raise.
pub open spec fn continue_interrupted(m: LockModel, site: Site) -> LockModel {
    match site {
        // `acquire` returns `Interrupted`, dropping a created file.
        Site::AfterAttempt => LockModel {
            fd: if m.attempt_kind == Attempt::Created {
                close(m.fd)
            } else {
                m.fd
            },
            pc: Pc::Idle,
            ..m
        },
        Site::AfterFailure => LockModel { pc: Pc::Idle, ..m },
        Site::AfterArm => LockModel { fd: close(m.fd), pc: Pc::Idle, ..m },
        Site::Release => LockModel { pc: Pc::Close, ..m },
    }
}

/// One main-thread step.
#[verifier::opaque]
pub open spec fn main_step(m: LockModel, other_error: bool) -> LockModel {
    if m.terminated {
        m
    } else {
        match m.pc {
            Pc::Idle => {
                // A new acquisition: a lock an earlier one left behind is now
                // someone else's problem (and recorded as leaked).
                let fresh = LockModel {
                    disk: if m.disk == Disk::Ours {
                        Disk::Foreign
                    } else {
                        m.disk
                    },
                    leaked: m.leaked || m.disk == Disk::Ours,
                    path_current: false,
                    removals: 0,
                    attempt: 0,
                    ..m
                };
                if m.stage_ok {
                    LockModel { state: WRITING, pc: Pc::ResetFd, ..fresh }
                } else {
                    LockModel { state: EMPTY, pc: Pc::Create, ..fresh }
                }
            },
            Pc::ResetFd => LockModel { fd: Fd::Unset, pc: Pc::WritePath, ..m },
            Pc::WritePath => LockModel { path_current: true, pc: Pc::StoreStaged, ..m },
            Pc::StoreStaged => LockModel { state: STAGED, pc: Pc::Create, ..m },
            Pc::Create => {
                let a = if other_error {
                    Attempt::Failed
                } else if m.disk == Disk::Free {
                    Attempt::Created
                } else {
                    Attempt::Contended
                };
                if a == Attempt::Created {
                    LockModel { attempt_kind: a, disk: Disk::Ours, pc: Pc::Record, ..m }
                } else {
                    LockModel { attempt_kind: a, pc: Pc::Check(Site::AfterAttempt), ..m }
                }
            },
            Pc::Record => LockModel { fd: Fd::Current, pc: Pc::Check(Site::AfterAttempt), ..m },
            Pc::Check(site) => {
                let sig = m.pending;
                let m1 = LockModel { pending: NO_SIGNAL as int, stranded: false, ..m };
                if sig == NO_SIGNAL as int {
                    continue_ok(m1, site)
                } else if holds(site, m) {
                    LockModel { resume_sig: sig, pc: Pc::ResumeLoad(site), ..m1 }
                } else {
                    LockModel { resume_sig: sig, pc: Pc::ResumeDisarm(site), ..m1 }
                }
            },
            Pc::ResumeLoad(site) => if spec_resume_removes(true, m.state) {
                LockModel { pc: Pc::ResumeStoreStaged(site), ..m }
            } else {
                LockModel { pc: Pc::ResumeDisarm(site), ..m }
            },
            Pc::ResumeStoreStaged(site) => LockModel { state: STAGED, pc: Pc::ResumeRemove(site), ..m },
            Pc::ResumeRemove(site) => LockModel { pc: Pc::ResumeDisarm(site), ..remove(m) },
            Pc::ResumeDisarm(site) => LockModel { state: EMPTY, pc: Pc::ResumeRaise(site), ..m },
            Pc::ResumeRaise(site) => {
                let m1 = raise(m, m.resume_sig);
                if m1.terminated {
                    m1
                } else {
                    continue_interrupted(m1, site)
                }
            },
            Pc::Wait => LockModel { attempt: m.attempt + 1, pc: Pc::Create, ..m },
            Pc::FailDisarm => LockModel { state: EMPTY, pc: Pc::Check(Site::AfterFailure), ..m },
            Pc::Arm => LockModel {
                state: if m.state == ARM.from {
                    ARM.to
                } else {
                    m.state
                },
                pc: Pc::Check(Site::AfterArm),
                ..m
            },
            Pc::Holding => LockModel {
                unlocked_publication: m.unlocked_publication || m.disk != Disk::Ours,
                pc: Pc::BeginRelease,
                ..m
            },
            Pc::BeginRelease => if m.state == BEGIN_RELEASE.from {
                LockModel { state: BEGIN_RELEASE.to, pc: Pc::RelRemove, ..m }
            } else {
                LockModel { pc: Pc::Close, ..m }
            },
            Pc::RelRemove => LockModel { pc: Pc::RelDisarm, ..remove(m) },
            Pc::RelDisarm => LockModel { state: EMPTY, pc: Pc::Check(Site::Release), ..m },
            Pc::Close => LockModel { fd: close(m.fd), pc: Pc::Idle, ..m },
        }
    }
}

/// One step of any kind.
pub open spec fn step(m: LockModel, s: Step) -> LockModel {
    match s {
        Step::Main(other_error) => main_step(m, other_error),
        Step::Signal(sig) => signal_step(m, sig),
        Step::Env(e) => env_step(m, e),
    }
}

/// The state after an interleaving.
pub open spec fn run(m: LockModel, steps: Seq<Step>) -> LockModel
    decreases steps.len(),
{
    if steps.len() == 0 {
        m
    } else {
        step(run(m, steps.drop_last()), steps.last())
    }
}

// ---------------------------------------------------------------------------
// The inductive invariant.
// ---------------------------------------------------------------------------

/// Our lock is on disk, if nobody but us touches it.
pub open spec fn owns(m: LockModel) -> bool {
    m.coop ==> m.disk == Disk::Ours
}

/// Our lock is not on disk.
pub open spec fn no_lock(m: LockModel) -> bool {
    m.disk != Disk::Ours
}

/// Armed and untouched, or claimed and removed by a handler that returned.
pub open spec fn armed_or_claimed(m: LockModel) -> bool {
    (m.state == ARMED && m.removals == 0 && owns(m)) || (m.state == EMPTY && m.removals == 1 && no_lock(m)
        && m.prev == Disposition::Returning)
}

/// Per-step facts when the path was staged.
pub open spec fn pc_inv(m: LockModel) -> bool {
    match m.pc {
        Pc::Idle => m.state == EMPTY && no_lock(m) && m.fd != Fd::Current,
        Pc::ResetFd => m.state == WRITING && no_lock(m) && m.fd != Fd::Current && m.removals == 0
            && m.attempt == 0,
        Pc::WritePath => m.state == WRITING && no_lock(m) && m.fd == Fd::Unset && m.removals == 0
            && m.attempt == 0,
        Pc::StoreStaged => m.state == WRITING && no_lock(m) && m.fd == Fd::Unset && m.path_current
            && m.removals == 0 && m.attempt == 0,
        Pc::Create => m.state == STAGED && no_lock(m) && m.fd == Fd::Unset && m.path_current && m.removals
            == 0 && m.attempt <= m.retries,
        Pc::Record => m.state == STAGED && m.attempt_kind == Attempt::Created && m.fd == Fd::Unset
            && m.path_current && m.removals == 0 && owns(m) && m.attempt <= m.retries,
        Pc::Check(Site::AfterAttempt) => m.state == STAGED && m.path_current && m.removals == 0 && m.attempt
            <= m.retries && if m.attempt_kind == Attempt::Created {
            m.fd == Fd::Current && owns(m)
        } else {
            m.fd == Fd::Unset && no_lock(m)
        },
        Pc::Wait => m.state == STAGED && no_lock(m) && m.fd == Fd::Unset && m.path_current && m.removals
            == 0 && m.attempt < m.retries,
        Pc::FailDisarm => m.state == STAGED && no_lock(m) && m.fd == Fd::Unset && m.path_current
            && m.removals == 0,
        Pc::Check(Site::AfterFailure) => m.state == EMPTY && no_lock(m) && m.fd == Fd::Unset && m.removals
            == 0,
        Pc::Arm => m.state == STAGED && m.attempt_kind == Attempt::Created && m.fd == Fd::Current
            && m.path_current && m.removals == 0 && owns(m),
        Pc::Check(Site::AfterArm) => m.fd == Fd::Current && m.path_current && armed_or_claimed(m),
        Pc::Holding => m.fd == Fd::Current && m.path_current && armed_or_claimed(m),
        Pc::BeginRelease => m.fd == Fd::Current && m.path_current && armed_or_claimed(m),
        Pc::RelRemove => m.state == STAGED && m.fd == Fd::Current && m.path_current && m.removals == 0,
        Pc::RelDisarm => m.state == STAGED && m.fd == Fd::Current && m.path_current && no_lock(m)
            && m.removals == 1,
        Pc::Check(Site::Release) => m.state == EMPTY && m.fd == Fd::Current && no_lock(m) && m.removals == 1,
        Pc::Close => m.state == EMPTY && m.fd == Fd::Current && no_lock(m) && m.removals == 1,
        Pc::ResumeLoad(site) => m.resume_sig > 0 && m.fd == Fd::Current && m.path_current && match site {
            Site::AfterAttempt => m.state == STAGED && m.attempt_kind == Attempt::Created && m.removals == 0
                && owns(m),
            Site::AfterArm => armed_or_claimed(m),
            _ => false,
        },
        Pc::ResumeStoreStaged(site) => m.resume_sig > 0 && m.fd == Fd::Current && m.path_current
            && match site {
            Site::AfterAttempt => m.state == STAGED && m.attempt_kind == Attempt::Created && m.removals == 0
                && owns(m),
            Site::AfterArm => armed_or_claimed(m),
            _ => false,
        },
        Pc::ResumeRemove(site) => m.resume_sig > 0 && m.state == STAGED && m.fd == Fd::Current
            && m.path_current && match site {
            Site::AfterAttempt => m.attempt_kind == Attempt::Created && m.removals == 0 && owns(m),
            Site::AfterArm => (m.removals == 0 && owns(m)) || (m.removals == 1 && no_lock(m) && m.prev
                == Disposition::Returning),
            _ => false,
        },
        Pc::ResumeDisarm(site) => m.resume_sig > 0 && no_lock(m) && match site {
            Site::AfterAttempt => m.state == STAGED && m.path_current && m.removals <= 1 && if m.attempt_kind
                == Attempt::Created {
                m.fd == Fd::Current
            } else {
                m.fd == Fd::Unset
            },
            Site::AfterFailure => m.state == EMPTY && m.fd == Fd::Unset && m.removals == 0,
            Site::AfterArm => (m.state == STAGED || m.state == EMPTY) && m.fd == Fd::Current && m.path_current
                && m.removals >= 1 && (m.prev != Disposition::Returning ==> m.removals == 1),
            Site::Release => m.state == EMPTY && m.fd == Fd::Current && m.removals == 1,
        },
        Pc::ResumeRaise(site) => m.resume_sig > 0 && m.state == EMPTY && no_lock(m) && match site {
            Site::AfterAttempt => m.removals <= 1 && if m.attempt_kind == Attempt::Created {
                m.fd == Fd::Current
            } else {
                m.fd == Fd::Unset
            },
            Site::AfterFailure => m.fd == Fd::Unset && m.removals == 0,
            Site::AfterArm => m.fd == Fd::Current && m.removals >= 1 && (m.prev != Disposition::Returning
                ==> m.removals == 1),
            Site::Release => m.fd == Fd::Current && m.removals == 1,
        },
    }
}

/// Main-thread steps reachable when the path could not be staged.
pub open spec fn unstaged_pc(pc: Pc) -> bool {
    pc is Idle || pc is Create || pc is Record || pc == Pc::Check(Site::AfterAttempt) || pc is Wait
        || pc is FailDisarm || pc == Pc::Check(Site::AfterFailure) || pc is Arm || pc == Pc::Check(
        Site::AfterArm,
    ) || pc is Holding || pc is BeginRelease || pc is Close
}

/// The inductive invariant.
#[verifier::opaque]
pub open spec fn inv(m: LockModel) -> bool {
    &&& !m.unsafe_removal
    &&& m.pending >= NO_SIGNAL as int
    &&& m.stranded ==> m.pending != NO_SIGNAL as int
    &&& m.pending != NO_SIGNAL as int && !m.terminated ==> redeemable(m.pc) || m.stranded
    &&& m.stranded && !m.terminated ==> m.prev == Disposition::Returning || in_resume_tail(m.pc)
    &&& m.prev == Disposition::Ignore ==> m.pending == NO_SIGNAL as int && !m.terminated
    &&& m.terminated ==> m.prev == Disposition::Default
    &&& m.prev == Disposition::Default && !m.terminated ==> m.uninstalled == Set::<int>::empty()
    &&& m.prev != Disposition::Returning ==> m.removals <= 1
    &&& if m.stage_ok {
        &&& !m.terminated ==> pc_inv(m)
        &&& !m.leaked
        &&& m.terminated ==> no_lock(m)
        &&& m.coop && m.prev != Disposition::Returning ==> !m.unlocked_publication
    } else {
        &&& unstaged_pc(m.pc)
        &&& m.state == EMPTY
        &&& m.pending == NO_SIGNAL as int
        &&& !m.stranded
        &&& m.removals == 0
        &&& !m.path_current
    }
}

// ---------------------------------------------------------------------------
// Proofs.
// ---------------------------------------------------------------------------

proof fn lemma_init(prev: Disposition, stage_ok: bool, coop: bool, retries: nat, disk: Disk)
    ensures
        inv(init(prev, stage_ok, coop, retries, disk)),
{
    reveal(inv);
}

/// The parameters a main step never changes.
pub open spec fn same_params(a: LockModel, b: LockModel) -> bool {
    a.prev == b.prev && a.stage_ok == b.stage_ok && a.coop == b.coop && a.retries == b.retries
}

proof fn lemma_main_preserves_check(m: LockModel, other_error: bool)
    requires
        inv(m),
        !m.terminated,
        m.pc is Check,
    ensures
        inv(main_step(m, other_error)),
        same_params(main_step(m, other_error), m),
{
    reveal(inv);
    reveal(main_step);
    match m.pc {
        Pc::Check(site) => {
            match site {
                Site::AfterAttempt => {},
                Site::AfterFailure => {},
                Site::AfterArm => {},
                Site::Release => {},
            }
        },
        _ => {},
    }
}

proof fn lemma_main_preserves_resume(m: LockModel, other_error: bool)
    requires
        inv(m),
        !m.terminated,
        in_resume_tail(m.pc),
    ensures
        inv(main_step(m, other_error)),
        same_params(main_step(m, other_error), m),
{
    reveal(inv);
    reveal(main_step);
    match m.pc {
        Pc::ResumeLoad(site) => {},
        Pc::ResumeStoreStaged(site) => {},
        Pc::ResumeRemove(site) => {},
        Pc::ResumeDisarm(site) => {},
        Pc::ResumeRaise(site) => {},
        _ => {},
    }
}

proof fn lemma_main_preserves_other(m: LockModel, other_error: bool)
    requires
        inv(m),
        !m.terminated,
        !(m.pc is Check),
        !in_resume_tail(m.pc),
    ensures
        inv(main_step(m, other_error)),
        same_params(main_step(m, other_error), m),
{
    reveal(inv);
    reveal(main_step);
}

proof fn lemma_main_preserves(m: LockModel, other_error: bool)
    requires
        inv(m),
    ensures
        inv(main_step(m, other_error)),
        same_params(main_step(m, other_error), m),
{
    if m.terminated {
        reveal(main_step);
    } else if m.pc is Check {
        lemma_main_preserves_check(m, other_error);
    } else if in_resume_tail(m.pc) {
        lemma_main_preserves_resume(m, other_error);
    } else {
        lemma_main_preserves_other(m, other_error);
    }
}

proof fn lemma_pc_inv_facts(m: LockModel)
    requires
        pc_inv(m),
    ensures
        m.state == EMPTY || m.state == WRITING ==> no_lock(m),
        m.state == ARMED ==> m.fd == Fd::Current && m.path_current && m.removals == 0,
        m.state == STAGED ==> redeemable(m.pc) || in_resume_window(m.pc),
{
    match m.pc {
        Pc::Check(site) => {},
        Pc::ResumeLoad(site) => {},
        Pc::ResumeStoreStaged(site) => {},
        Pc::ResumeRemove(site) => {},
        Pc::ResumeDisarm(site) => {},
        Pc::ResumeRaise(site) => {},
        _ => {},
    }
}

proof fn lemma_signal_preserves(m: LockModel, sig: int)
    requires
        inv(m),
    ensures
        inv(signal_step(m, sig)),
{
    reveal(inv);
    if m.stage_ok && !m.terminated {
        lemma_pc_inv_facts(m);
    }
}

proof fn lemma_env_preserves(m: LockModel, e: EnvStep)
    requires
        inv(m),
    ensures
        inv(env_step(m, e)),
{
    reveal(inv);
}

/// The invariant holds after every interleaving from an initial state, and
/// the parameters never change.
pub proof fn lemma_inv_run(prev: Disposition, stage_ok: bool, coop: bool, retries: nat, disk: Disk, steps: Seq<Step>)
    ensures
        inv(run(init(prev, stage_ok, coop, retries, disk), steps)),
        run(init(prev, stage_ok, coop, retries, disk), steps).prev == prev,
        run(init(prev, stage_ok, coop, retries, disk), steps).stage_ok == stage_ok,
        run(init(prev, stage_ok, coop, retries, disk), steps).coop == coop,
        run(init(prev, stage_ok, coop, retries, disk), steps).retries == retries,
    decreases steps.len(),
{
    let m0 = init(prev, stage_ok, coop, retries, disk);
    if steps.len() == 0 {
        lemma_init(prev, stage_ok, coop, retries, disk);
    } else {
        lemma_inv_run(prev, stage_ok, coop, retries, disk, steps.drop_last());
        let m = run(m0, steps.drop_last());
        match steps.last() {
            Step::Main(e) => lemma_main_preserves(m, e),
            Step::Signal(sig) => lemma_signal_preserves(m, sig),
            Step::Env(e) => lemma_env_preserves(m, e),
        }
    }
}


// ---------------------------------------------------------------------------
// Theorems about the model.
// ---------------------------------------------------------------------------

/// P1 (never remove a foreign lock). In every reachable state: no removal
/// has ever been attempted with a stale descriptor or a stale path (so the
/// `fstat`/`lstat` identity check always compares this acquisition's file
/// with what the path names now), and no main-thread or handler step turns
/// another writer's lock into anything else — only the environment does.
pub proof fn theorem_p1_never_remove_foreign_lock(
    prev: Disposition,
    stage_ok: bool,
    coop: bool,
    retries: nat,
    disk: Disk,
    steps: Seq<Step>,
    next: Step,
)
    ensures
        !run(init(prev, stage_ok, coop, retries, disk), steps).unsafe_removal,
        run(init(prev, stage_ok, coop, retries, disk), steps).disk == Disk::Foreign && step(
            run(init(prev, stage_ok, coop, retries, disk), steps),
            next,
        ).disk != Disk::Foreign ==> next is Env,
{
    lemma_inv_run(prev, stage_ok, coop, retries, disk, steps);
    reveal(inv);
    reveal(main_step);
    let m = run(init(prev, stage_ok, coop, retries, disk), steps);
    if !(next is Env) && !m.terminated && m.disk == Disk::Foreign {
        match next {
            Step::Main(e) => {
                match m.pc {
                    Pc::Check(site) => {},
                    Pc::ResumeLoad(site) => {},
                    Pc::ResumeStoreStaged(site) => {},
                    Pc::ResumeRemove(site) => {},
                    Pc::ResumeDisarm(site) => {},
                    Pc::ResumeRaise(site) => {},
                    _ => {},
                }
            },
            _ => {},
        }
    }
}

/// P2 (at most one removal attempt per acquisition). Unless the previous
/// disposition is a handler that returns, ownership of the lock is consumed
/// once — by the handler's claim `ARMED -> EMPTY`, release's claim
/// `ARMED -> STAGED`, or a resume that found the state not `EMPTY` — so after
/// our unlink, no path of this acquisition can unlink the name again, even
/// after another writer has created its own lock there.
pub proof fn theorem_p2_one_removal_per_acquisition(
    prev: Disposition,
    stage_ok: bool,
    coop: bool,
    retries: nat,
    disk: Disk,
    steps: Seq<Step>,
)
    requires
        prev != Disposition::Returning,
    ensures
        run(init(prev, stage_ok, coop, retries, disk), steps).removals <= 1,
{
    lemma_inv_run(prev, stage_ok, coop, retries, disk, steps);
    reveal(inv);
}

/// P3 (no stale lock on signal death). When the lock path fits the
/// signal-safe buffer: if a signal terminates the process, our lock is not on
/// disk; whenever no acquisition is in progress, our lock is not on disk; and
/// no acquisition ever ends with its lock left behind.
pub proof fn theorem_p3_no_stale_lock_on_signal_death(
    prev: Disposition,
    coop: bool,
    retries: nat,
    disk: Disk,
    steps: Seq<Step>,
)
    ensures
        run(init(prev, true, coop, retries, disk), steps).terminated ==> run(
            init(prev, true, coop, retries, disk),
            steps,
        ).disk != Disk::Ours,
        run(init(prev, true, coop, retries, disk), steps).pc == Pc::Idle ==> run(
            init(prev, true, coop, retries, disk),
            steps,
        ).disk != Disk::Ours,
        !run(init(prev, true, coop, retries, disk), steps).leaked,
{
    lemma_inv_run(prev, true, coop, retries, disk, steps);
    reveal(inv);
}

/// P4 (no stranded deferral). In every reachable state of a live process, a
/// recorded deferral has a consuming check ahead of the main thread in the
/// current acquisition or release (`redeemable`), unless it was recorded
/// while the main thread was inside `resume_pending` between its take and
/// its disarm (`stranded`). A stranded deferral outlives that resume only
/// under a previous handler that returns; under `SIG_DFL` the resume's
/// re-raise ends the process first.
pub proof fn theorem_p4_no_stranded_deferral(
    prev: Disposition,
    stage_ok: bool,
    coop: bool,
    retries: nat,
    disk: Disk,
    steps: Seq<Step>,
)
    ensures
        ({
            let m = run(init(prev, stage_ok, coop, retries, disk), steps);
            &&& m.pending != NO_SIGNAL as int && !m.terminated ==> redeemable(m.pc) || m.stranded
            &&& m.stranded && !m.terminated ==> m.prev == Disposition::Returning || in_resume_tail(m.pc)
            &&& m.stranded ==> m.pending != NO_SIGNAL as int
        }),
{
    lemma_inv_run(prev, stage_ok, coop, retries, disk, steps);
    reveal(inv);
}

/// P7 (publication only under the lock). With a staged path, a cooperative
/// environment, and a previous disposition that does not return, every
/// publication happens while our lock is on disk; whenever the main thread
/// holds the lock it is armed, on disk, and no signal is deferred.
pub proof fn theorem_p7_publication_under_lock(prev: Disposition, retries: nat, disk: Disk, steps: Seq<Step>)
    requires
        prev != Disposition::Returning,
    ensures
        !run(init(prev, true, true, retries, disk), steps).unlocked_publication,
        ({
            let m = run(init(prev, true, true, retries, disk), steps);
            m.pc == Pc::Holding && !m.terminated ==> m.state == ARMED && m.disk == Disk::Ours && m.pending
                == NO_SIGNAL as int
        }),
{
    lemma_inv_run(prev, true, true, retries, disk, steps);
    reveal(inv);
}

// ---------------------------------------------------------------------------
// The exceptions are real: concrete interleavings.
// ---------------------------------------------------------------------------

proof fn lemma_run_push(m: LockModel, steps: Seq<Step>, s: Step)
    ensures
        run(m, steps.push(s)) == step(run(m, steps), s),
{
    assert(steps.push(s).drop_last() =~= steps);
}

/// The P2 exception. Under a previous handler that returns: a signal is
/// deferred just before arming; the post-arm check takes it and loads
/// `ARMED`; a second signal then makes the handler claim and remove the lock;
/// and the resume, which already decided to remove, stores `STAGED` and
/// attempts a second removal. The identity check still protects a lock
/// another writer creates meanwhile, but only up to the `lstat`/`unlink`
/// window.
pub proof fn example_double_removal_with_returning_handler()
    ensures
        ({
            let m0 = init(Disposition::Returning, true, true, 0, Disk::Free);
            let m1 = main_step(main_step(main_step(main_step(m0, false), false), false), false);
            let m2 = main_step(main_step(main_step(m1, false), false), false);
            let m3 = main_step(signal_step(m2, 2), false);
            let m4 = main_step(main_step(m3, false), false);
            let m5 = main_step(main_step(signal_step(m4, 15), false), false);
            m5.removals == 2 && m5.pc == Pc::ResumeDisarm(Site::AfterArm) && !m5.terminated
        }),
{
    reveal(main_step);
}

/// The P3 exception. A lock path too long for the signal-safe buffer is
/// never staged, so it is never armed: a signal while the lock is held
/// terminates the process with the lock still on disk.
pub proof fn example_unstaged_path_leaves_stale_lock_on_signal()
    ensures
        ({
            let m0 = init(Disposition::Default, false, true, 0, Disk::Free);
            let m1 = main_step(main_step(main_step(main_step(m0, false), false), false), false);
            let m2 = main_step(main_step(m1, false), false);
            let m3 = signal_step(m2, 2);
            m2.pc == Pc::Holding && m3.terminated && m3.disk == Disk::Ours
        }),
{
    reveal(main_step);
}

/// The P3 exception, without any signal: release of an unstaged lock never
/// claims it, so the next acquisition finds it left behind.
pub proof fn example_unstaged_path_leaves_stale_lock()
    ensures
        ({
            let m0 = init(Disposition::Default, false, true, 0, Disk::Free);
            let m1 = main_step(main_step(main_step(main_step(m0, false), false), false), false);
            let m2 = main_step(main_step(m1, false), false);
            let m3 = main_step(main_step(main_step(main_step(m2, false), false), false), false);
            m2.pc == Pc::Holding && m3.leaked
        }),
{
    reveal(main_step);
}

/// The P4 exception. Under a previous handler that returns: a signal is
/// deferred while the file is being recorded; the check takes it and starts
/// removing the lock; a second signal arrives while the state is `STAGED`
/// for that removal and is deferred with no check ahead of it; the first
/// signal's re-raise returns, acquisition fails, and the second deferral is
/// left pending at `Idle`, to abort the next acquisition (or be lost at
/// exit).
pub proof fn example_stranded_deferral_with_returning_handler()
    ensures
        ({
            let m0 = init(Disposition::Returning, true, true, 0, Disk::Free);
            let m1 = main_step(main_step(main_step(main_step(m0, false), false), false), false);
            let m2 = signal_step(main_step(m1, false), 2);
            let m3 = signal_step(main_step(main_step(m2, false), false), 15);
            let m4 = main_step(main_step(main_step(main_step(main_step(m3, false), false), false), false), false);
            m4.pc == Pc::Idle && m4.pending == 15 && m4.stranded && !m4.terminated && m4.disk == Disk::Free
        }),
{
    reveal(main_step);
}

} // verus!
