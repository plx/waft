//! Verified kernel: the destination index lock's acquisition loop and its
//! signal-deferral protocol.
//!
//! # What it decides
//!
//! `executor::GitIndexLock` takes Git's `index.lock` before the final
//! tracked check and holds it through publication. On Unix a `SIGINT` or
//! `SIGTERM` handler removes a lock this process holds and re-raises the
//! signal. The protocol is a four-state atomic (`EMPTY`, `WRITING`, `STAGED`,
//! `ARMED`) plus a deferred-signal cell. This module ships every decision of
//! that protocol as verified code that production calls through small effect
//! traits, one trait method per atomic operation or system call:
//!
//! * [`stage`] — record the lock path before the file exists (or, if the path
//!   does not fit the buffer, leave the state `EMPTY`);
//! * [`acquire`] — the bounded `create_new` retry loop, with a deferred-signal
//!   check after every attempt and after arming or disarming;
//! * [`on_signal`] — the signal handler: defer while `STAGED`, otherwise claim
//!   an armed lock, remove it, and re-raise;
//! * [`resume_pending`] — act on a deferred signal once the caller knows
//!   whether it holds the lock;
//! * [`release`] — the `Drop` path: claim, remove, disarm, then resume;
//! * [`arm`], [`disarm`], [`claim_armed`] — the single atomic transitions.
//!
//! # Theorems about the shipped exec code
//!
//! Each driver's `ensures` gives the exact sequence of effects it issues as a
//! function of the values its loads and compare-exchanges observed
//! (`*_events` / `acquire_log`). From those:
//!
//! * `lemma_stage_writes_only_while_writing` — the descriptor is reset and
//!   the path written strictly between the stores of `WRITING` and `STAGED`.
//! * `lemma_handler_removes_only_after_claim` — the handler issues a removal
//!   only immediately after its compare-exchange `ARMED -> EMPTY` succeeded,
//!   and it returns without re-raising only when both of its loads saw
//!   `STAGED`.
//! * `lemma_resume_removes_only_held_live_lock` — a resumed signal removes the
//!   lock only when the caller holds it and the state was not `EMPTY`, and
//!   only after storing `STAGED`; the re-raise always follows a disarm.
//! * `lemma_release_order` — release removes only after its compare-exchange
//!   `ARMED -> STAGED` succeeded, and disarms before acting on any deferred
//!   signal.
//! * `lemma_acquire_bounded_and_checked` — `create_new` runs at most
//!   `retries + 1` times; every attempt is followed immediately by a
//!   deferred-signal check whose `holds_lock` is whether that attempt created
//!   the file; the `n`-th wait uses delay index `n < retries`; the "after N
//!   attempt(s)" count equals the number of attempts.
//!
//! # Theorems about the protocol model
//!
//! [`model`] is a ghost transition system of the main thread (one step per
//! effect above), handler runs at any step boundary, and other Git writers.
//! Its theorems — never remove a foreign lock (P1), at most one removal
//! attempt per acquisition (P2), no stale lock on signal death (P3), no
//! stranded deferral (P4), publication only under the lock (P7) — are
//! stated and proved there, with the exceptions each one has.
//!
//! # Trust assumptions
//!
//! * Each effect-trait method performs exactly the one operation its name
//!   says, on the process-global cells (`STATE`, `PENDING_SIGNAL`,
//!   `LOCK_FD`, `PATH`), with `SeqCst` ordering; its `ensures` only records
//!   that the call happened and what it observed.
//! * `remove_recorded_lock` unlinks the path only if `fstat(LOCK_FD)` and
//!   `lstat(PATH)` report the same `(st_dev, st_ino)`.
//! * The correspondence between the model's steps and these drivers is
//!   checked by review; each model step cites the driver call it mirrors.

#[allow(unused_imports)]
use vstd::prelude::*;

pub mod model;

verus! {

/// No lock path is recorded.
pub const EMPTY: u8 = 0;

/// The lock path is being written; nothing may read it.
pub const WRITING: u8 = 1;

/// The lock path is recorded, but whether the file exists (and is ours) is
/// still undecided. A signal arriving now is deferred.
pub const STAGED: u8 = 2;

/// The recorded path is this process's lock; a signal removes it.
pub const ARMED: u8 = 3;

/// No signal is deferred. Zero is not a valid signal number.
pub const NO_SIGNAL: i32 = 0;

/// Bytes the signal-safe path buffer holds, including the terminating NUL.
/// Longer than `PATH_MAX` on every platform waft supports.
pub const PATH_CAPACITY: usize = 4096;

/// A compare-exchange on the state cell: `from -> to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transition {
    /// The value the state must hold.
    pub from: u8,
    /// The value it is replaced with.
    pub to: u8,
}

/// `arm`: the staged path becomes the live lock.
pub const ARM: Transition = Transition { from: STAGED, to: ARMED };

/// The handler's claim of an armed lock before removing it.
pub const CLAIM: Transition = Transition { from: ARMED, to: EMPTY };

/// Release's claim of an armed lock: back to `STAGED`, so a signal arriving
/// during removal is deferred rather than acted on.
pub const BEGIN_RELEASE: Transition = Transition { from: ARMED, to: STAGED };

/// Whether a load of the state means "defer": the lock file's existence is
/// undecided.
pub open spec fn spec_defers(observed: u8) -> bool {
    observed == STAGED
}

/// Whether a resumed signal must try to remove the lock: the caller holds it
/// and cleanup has not already claimed it.
pub open spec fn spec_resume_removes(holds_lock: bool, observed: u8) -> bool {
    holds_lock && observed != EMPTY
}

// ---------------------------------------------------------------------------
// The signal cells.
// ---------------------------------------------------------------------------

/// One operation on the signal cells, with what it observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellEvent {
    /// `STATE.load()` returned this value.
    Load(u8),
    /// `STATE.store(value)`.
    Store(u8),
    /// `STATE.compare_exchange(t.from, t.to)` and whether it succeeded.
    Cas(Transition, bool),
    /// `PENDING_SIGNAL.swap(NO_SIGNAL)` returned this value.
    Take(i32),
    /// `PENDING_SIGNAL.compare_exchange(NO_SIGNAL, signal)`.
    Defer(i32),
    /// `PENDING_SIGNAL.compare_exchange(signal, NO_SIGNAL)`.
    Withdraw(i32),
    /// The identity-checked unlink of the recorded lock path.
    Remove,
    /// Restore the replaced disposition and re-raise this signal.
    Raise(i32),
}

/// The process-global cells of the protocol, implemented by
/// `executor::interrupt_cleanup` with async-signal-safe operations.
pub trait SignalCells {
    /// Ghost history of the operations performed through this value.
    spec fn events(&self) -> Seq<CellEvent>;

    /// Load the state.
    fn load_state(&mut self) -> (observed: u8)
        ensures
            final(self).events() == old(self).events().push(CellEvent::Load(observed)),
    ;

    /// Store the state.
    fn store_state(&mut self, value: u8)
        ensures
            final(self).events() == old(self).events().push(CellEvent::Store(value)),
    ;

    /// Compare-exchange the state.
    fn cas_state(&mut self, t: Transition) -> (ok: bool)
        ensures
            final(self).events() == old(self).events().push(CellEvent::Cas(t, ok)),
    ;

    /// Swap the deferred signal out, leaving `NO_SIGNAL`.
    fn take_pending(&mut self) -> (signal: i32)
        ensures
            final(self).events() == old(self).events().push(CellEvent::Take(signal)),
    ;

    /// Record `signal` as deferred unless one is already recorded.
    fn defer(&mut self, signal: i32)
        ensures
            final(self).events() == old(self).events().push(CellEvent::Defer(signal)),
    ;

    /// Take back a deferral of `signal`, if it is still the recorded one.
    fn withdraw(&mut self, signal: i32)
        ensures
            final(self).events() == old(self).events().push(CellEvent::Withdraw(signal)),
    ;

    /// Unlink the recorded path if it still names the open lock file.
    fn remove_recorded_lock(&mut self)
        ensures
            final(self).events() == old(self).events().push(CellEvent::Remove),
    ;

    /// A test seam between removal and disarming; no effect in production.
    fn after_lock_removed(&mut self)
        ensures
            final(self).events() == old(self).events(),
    ;

    /// Put back the replaced disposition and re-raise `signal`.
    fn restore_previous_and_raise(&mut self, signal: i32)
        ensures
            final(self).events() == old(self).events().push(CellEvent::Raise(signal)),
    ;
}

/// The events of [`claim_armed`].
pub open spec fn claim_events(claimed: bool) -> Seq<CellEvent> {
    if claimed {
        seq![CellEvent::Cas(CLAIM, true), CellEvent::Remove]
    } else {
        seq![CellEvent::Cas(CLAIM, false)]
    }
}

/// Remove the recorded lock if, and only if, this call is the one that
/// claims it from `ARMED`.
pub fn claim_armed<O: SignalCells>(o: &mut O)
    ensures
        exists|claimed: bool| final(o).events() == old(o).events() + #[trigger] claim_events(claimed),
{
    let claimed = o.cas_state(CLAIM);
    if claimed {
        o.remove_recorded_lock();
    }
    proof {
        assert(o.events() =~= old(o).events() + claim_events(claimed));
    }
}

/// `arm`: promote a staged path to the live lock. Does nothing unless a path
/// was staged, so a lock whose path could not be recorded never adopts an
/// earlier lock's path.
pub fn arm<O: SignalCells>(o: &mut O)
    ensures
        exists|ok: bool| final(o).events() == old(o).events() + #[trigger] arm_events(ok),
{
    let ghost e0 = o.events();
    let _ok = o.cas_state(ARM);
    proof {
        assert(o.events() =~= e0 + arm_events(_ok));
    }
}

/// The events of [`arm`].
pub open spec fn arm_events(ok: bool) -> Seq<CellEvent> {
    seq![CellEvent::Cas(ARM, ok)]
}

/// `disarm`: stop treating any path as the live lock.
pub fn disarm<O: SignalCells>(o: &mut O)
    ensures
        final(o).events() == old(o).events().push(CellEvent::Store(EMPTY)),
{
    o.store_state(EMPTY);
}

/// The events of [`on_signal`], given what its loads and its claim observed.
pub open spec fn handler_events(signal: i32, first: u8, second: u8, claimed: bool) -> Seq<CellEvent> {
    let act = claim_events(claimed).push(CellEvent::Raise(signal));
    if spec_defers(first) {
        if spec_defers(second) {
            seq![CellEvent::Load(first), CellEvent::Defer(signal), CellEvent::Load(second)]
        } else {
            seq![
                CellEvent::Load(first),
                CellEvent::Defer(signal),
                CellEvent::Load(second),
                CellEvent::Withdraw(signal),
            ] + act
        }
    } else {
        seq![CellEvent::Load(first)] + act
    }
}

/// The body of the `SIGINT`/`SIGTERM` handler.
///
/// While the state is `STAGED` the handler cannot tell whether the lock file
/// exists or whose it is, so it records the signal and returns; a second load
/// confirms that a check is still ahead of the acquire path (otherwise the
/// record is taken back and the signal handled here). In every other state it
/// claims an armed lock, removes it, and re-raises the signal.
pub fn on_signal<O: SignalCells>(o: &mut O, signal: i32)
    ensures
        exists|first: u8, second: u8, claimed: bool|
            final(o).events() == old(o).events() + #[trigger] handler_events(signal, first, second, claimed),
{
    let ghost e0 = o.events();
    let first = o.load_state();
    let ghost mut second: u8 = first;
    if spec_defers_exec(first) {
        o.defer(signal);
        let reloaded = o.load_state();
        proof {
            second = reloaded;
        }
        if spec_defers_exec(reloaded) {
            proof {
                assert(o.events() =~= e0 + handler_events(signal, first, second, false));
            }
            return;
        }
        o.withdraw(signal);
    }
    let ghost e1 = o.events();
    claim_armed(o);
    let ghost claimed = choose|claimed: bool| o.events() == e1 + #[trigger] claim_events(claimed);
    o.restore_previous_and_raise(signal);
    proof {
        assert(o.events() =~= e0 + handler_events(signal, first, second, claimed));
    }
}

/// Whether a load of the state means "defer" (exec form of `spec_defers`).
pub fn spec_defers_exec(observed: u8) -> (b: bool)
    ensures
        b == spec_defers(observed),
{
    observed == STAGED
}

/// The events of [`resume_pending`], given the signal it took and the state
/// it observed.
pub open spec fn resume_events(holds_lock: bool, taken: i32, observed: u8) -> Seq<CellEvent> {
    if taken == NO_SIGNAL {
        seq![CellEvent::Take(taken)]
    } else if holds_lock {
        let removal = if spec_resume_removes(holds_lock, observed) {
            seq![CellEvent::Store(STAGED), CellEvent::Remove]
        } else {
            Seq::<CellEvent>::empty()
        };
        seq![CellEvent::Take(taken), CellEvent::Load(observed)] + removal + seq![
            CellEvent::Store(EMPTY),
            CellEvent::Raise(taken),
        ]
    } else {
        seq![CellEvent::Take(taken), CellEvent::Store(EMPTY), CellEvent::Raise(taken)]
    }
}

/// Act on a signal the handler deferred, if there is one.
///
/// `holds_lock` says whether the staged path names a file this process just
/// created. A held lock is removed unless cleanup already claimed it; the
/// state is set to `STAGED` first so that a signal arriving during removal is
/// deferred rather than acted on. The state is then disarmed and the signal
/// re-raised. Returns the signal if the process survived the re-raise.
pub fn resume_pending<O: SignalCells>(o: &mut O, holds_lock: bool) -> (r: Option<i32>)
    ensures
        exists|taken: i32, observed: u8|
            {
                &&& final(o).events() == old(o).events() + #[trigger] resume_events(holds_lock, taken, observed)
                &&& r == if taken == NO_SIGNAL {
                    None
                } else {
                    Some(taken)
                }
            },
{
    let ghost e0 = o.events();
    let taken = o.take_pending();
    if taken == NO_SIGNAL {
        proof {
            assert(o.events() =~= e0 + resume_events(holds_lock, taken, EMPTY));
        }
        return None;
    }
    let ghost mut observed: u8 = EMPTY;
    if holds_lock {
        let loaded = o.load_state();
        proof {
            observed = loaded;
        }
        if loaded != EMPTY {
            o.store_state(STAGED);
            o.remove_recorded_lock();
        }
    }
    disarm(o);
    o.restore_previous_and_raise(taken);
    proof {
        assert(o.events() =~= e0 + resume_events(holds_lock, taken, observed));
    }
    Some(taken)
}

/// The events of [`release`], given what its claim, its take, and its load
/// observed.
pub open spec fn release_events(begun: bool, taken: i32, observed: u8) -> Seq<CellEvent> {
    if begun {
        seq![CellEvent::Cas(BEGIN_RELEASE, true), CellEvent::Remove, CellEvent::Store(EMPTY)]
            + resume_events(false, taken, observed)
    } else {
        seq![CellEvent::Cas(BEGIN_RELEASE, false)]
    }
}

/// Release the armed lock (the Unix `Drop` path): claim it back to `STAGED`
/// so signals defer during removal, remove it, disarm, and only then act on
/// a signal deferred meanwhile. Does nothing unless the lock is armed.
pub fn release<O: SignalCells>(o: &mut O)
    ensures
        exists|begun: bool, taken: i32, observed: u8|
            final(o).events() == old(o).events() + #[trigger] release_events(begun, taken, observed),
{
    let ghost e0 = o.events();
    if o.cas_state(BEGIN_RELEASE) {
        o.remove_recorded_lock();
        o.after_lock_removed();
        disarm(o);
        let ghost e1 = o.events();
        let _ = resume_pending(o, false);
        let ghost (taken, observed) = choose|taken: i32, observed: u8|
            o.events() == e1 + #[trigger] resume_events(false, taken, observed);
        proof {
            assert(o.events() =~= e0 + release_events(true, taken, observed));
        }
    } else {
        proof {
            assert(o.events() =~= e0 + release_events(false, NO_SIGNAL, EMPTY));
        }
    }
}

// ---------------------------------------------------------------------------
// Staging.
// ---------------------------------------------------------------------------

/// One staging operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageEvent {
    /// `STATE.store(value)`.
    Store(u8),
    /// Install the handlers (once per process).
    InstallHandlers,
    /// `LOCK_FD.store(-1)`.
    ResetFd,
    /// Copy the path and its NUL into the signal-safe buffer.
    WritePath,
}

/// The staging effects, implemented by `executor::interrupt_cleanup::stage`.
pub trait StageOps {
    /// Ghost history of the operations performed through this value.
    spec fn events(&self) -> Seq<StageEvent>;

    /// Store the state.
    fn store_state(&mut self, value: u8)
        ensures
            final(self).events() == old(self).events().push(StageEvent::Store(value)),
    ;

    /// Install the signal handlers, once per process.
    fn install_handlers(&mut self)
        ensures
            final(self).events() == old(self).events().push(StageEvent::InstallHandlers),
    ;

    /// Forget any descriptor recorded by an earlier lock.
    fn reset_fd(&mut self)
        ensures
            final(self).events() == old(self).events().push(StageEvent::ResetFd),
    ;

    /// Copy the path into the buffer and terminate it. [`stage`] calls this
    /// only when the path and its NUL fit.
    fn write_path(&mut self)
        ensures
            final(self).events() == old(self).events().push(StageEvent::WritePath),
    ;
}

/// Whether a path of `len` bytes, plus its NUL, fits the buffer.
pub fn path_fits(len: usize) -> (b: bool)
    ensures
        b == (len < PATH_CAPACITY),
{
    len < PATH_CAPACITY
}

/// The events of [`stage`].
pub open spec fn stage_events(fits: bool) -> Seq<StageEvent> {
    if fits {
        seq![
            StageEvent::InstallHandlers,
            StageEvent::Store(WRITING),
            StageEvent::ResetFd,
            StageEvent::WritePath,
            StageEvent::Store(STAGED),
        ]
    } else {
        seq![StageEvent::Store(EMPTY)]
    }
}

/// Record the lock path (of `path_len` bytes) before the file exists, so
/// that arming afterwards is a single atomic store. A path that does not fit
/// is not recorded and the state is left `EMPTY`, which keeps [`arm`] from
/// adopting a stale path.
pub fn stage<O: StageOps>(o: &mut O, path_len: usize)
    ensures
        final(o).events() == old(o).events() + stage_events(path_len < PATH_CAPACITY),
{
    let ghost e0 = o.events();
    if !path_fits(path_len) {
        o.store_state(EMPTY);
        proof {
            assert(o.events() =~= e0 + stage_events(false));
        }
        return;
    }
    o.install_handlers();
    o.store_state(WRITING);
    o.reset_fd();
    o.write_path();
    o.store_state(STAGED);
    proof {
        assert(o.events() =~= e0 + stage_events(true));
    }
}

// ---------------------------------------------------------------------------
// Acquisition.
// ---------------------------------------------------------------------------

/// What one `create_new` attempt on the lock path produced.
pub enum CreateOutcome<F, Er> {
    /// The lock file was created (and its descriptor recorded).
    Created(F),
    /// The name is taken (`AlreadyExists`): another writer holds the lock.
    Contended(Er),
    /// Any other error; never retried.
    Failed(Er),
}

/// The payload-free shape of a [`CreateOutcome`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attempt {
    /// The file was created.
    Created,
    /// The name was taken.
    Contended,
    /// Another error.
    Failed,
}

/// The shape of a create outcome.
pub open spec fn attempt_of<F, Er>(r: CreateOutcome<F, Er>) -> Attempt {
    match r {
        CreateOutcome::Created(_) => Attempt::Created,
        CreateOutcome::Contended(_) => Attempt::Contended,
        CreateOutcome::Failed(_) => Attempt::Failed,
    }
}

/// One step of [`acquire`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcquireEvent {
    /// [`stage`] ran.
    Stage,
    /// One `create_new` attempt, with its outcome.
    Create(Attempt),
    /// One deferred-signal check (`resume_deferred_signal(path, holds_lock)`)
    /// and whether it let acquisition continue.
    Check(bool, bool),
    /// A retry delay: sleep for `INDEX_LOCK_RETRY_DELAYS[index]`.
    Wait(usize),
    /// Build the "locked after N attempt(s)" error with this N.
    GiveUp(usize),
    /// [`arm`] ran (after the creation hook).
    Arm,
    /// [`disarm`] ran.
    Disarm,
}

/// The effects of `GitIndexLock::acquire`, implemented in `executor.rs`.
pub trait AcquireOps<F, Er> {
    /// Ghost history of the operations performed through this value.
    spec fn log(&self) -> Seq<AcquireEvent>;

    /// Stage the lock path.
    fn stage(&mut self)
        ensures
            final(self).log() == old(self).log().push(AcquireEvent::Stage),
    ;

    /// Try to create the lock file exclusively; record its descriptor if
    /// created.
    fn try_create(&mut self) -> (r: CreateOutcome<F, Er>)
        ensures
            final(self).log() == old(self).log().push(AcquireEvent::Create(attempt_of(r))),
    ;

    /// Act on a deferred signal, if any; `Err` means acquisition must stop.
    fn check_deferred(&mut self, holds_lock: bool) -> (r: Result<(), Er>)
        ensures
            final(self).log() == old(self).log().push(AcquireEvent::Check(holds_lock, r is Ok)),
    ;

    /// Sleep for the retry delay at `delay_index`.
    fn wait(&mut self, delay_index: usize)
        ensures
            final(self).log() == old(self).log().push(AcquireEvent::Wait(delay_index)),
    ;

    /// The error for a lock still held after `attempts` attempts.
    fn locked_error(&mut self, attempts: usize) -> Er
        ensures
            final(self).log() == old(self).log().push(AcquireEvent::GiveUp(attempts)),
    ;

    /// The error for a create attempt that failed for another reason.
    fn create_error(&mut self, error: Er) -> Er
        ensures
            final(self).log() == old(self).log(),
    ;

    /// Run the creation hook and arm the lock.
    fn arm(&mut self)
        ensures
            final(self).log() == old(self).log().push(AcquireEvent::Arm),
    ;

    /// Disarm.
    fn disarm(&mut self)
        ensures
            final(self).log() == old(self).log().push(AcquireEvent::Disarm),
    ;
}

/// How an acquisition ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcquireEnd {
    /// The check after an attempt with this outcome reported a deferred
    /// signal.
    Interrupted(Attempt),
    /// The lock was created and armed; the final check let it through or not.
    Armed(bool),
    /// Every attempt met contention; the final check let the error through or
    /// replaced it.
    Locked(bool),
    /// An attempt failed with another error.
    Failed(bool),
}

/// `k` contended attempts, each followed by its check and a wait.
pub open spec fn retry_log(k: nat) -> Seq<AcquireEvent>
    decreases k,
{
    if k == 0 {
        Seq::<AcquireEvent>::empty()
    } else {
        retry_log((k - 1) as nat) + seq![
            AcquireEvent::Create(Attempt::Contended),
            AcquireEvent::Check(false, true),
            AcquireEvent::Wait((k - 1) as usize),
        ]
    }
}

/// The events of the final attempt and what follows it.
pub open spec fn final_log(retries: usize, end: AcquireEnd) -> Seq<AcquireEvent> {
    match end {
        AcquireEnd::Interrupted(a) => seq![
            AcquireEvent::Create(a),
            AcquireEvent::Check(a == Attempt::Created, false),
        ],
        AcquireEnd::Armed(ok) => seq![
            AcquireEvent::Create(Attempt::Created),
            AcquireEvent::Check(true, true),
            AcquireEvent::Arm,
            AcquireEvent::Check(true, ok),
        ],
        AcquireEnd::Locked(ok) => seq![
            AcquireEvent::Create(Attempt::Contended),
            AcquireEvent::Check(false, true),
            AcquireEvent::GiveUp((retries + 1) as usize),
            AcquireEvent::Disarm,
            AcquireEvent::Check(false, ok),
        ],
        AcquireEnd::Failed(ok) => seq![
            AcquireEvent::Create(Attempt::Failed),
            AcquireEvent::Check(false, true),
            AcquireEvent::Disarm,
            AcquireEvent::Check(false, ok),
        ],
    }
}

/// The events of an acquisition whose last attempt is attempt `k`
/// (counting from zero) and that ended as `end`.
pub open spec fn acquire_log(retries: usize, k: nat, end: AcquireEnd) -> Seq<AcquireEvent> {
    seq![AcquireEvent::Stage] + retry_log(k) + final_log(retries, end)
}

/// Take the lock: stage, then try `create_new` up to `retries + 1` times,
/// checking for a deferred signal after every attempt; on success arm and
/// check again, otherwise disarm and check again.
pub fn acquire<F, Er, O: AcquireOps<F, Er>>(o: &mut O, retries: usize) -> (r: Result<F, Er>)
    requires
        retries < usize::MAX,
    ensures
        exists|k: nat, end: AcquireEnd|
            {
                &&& k <= retries
                &&& (end is Locked ==> k == retries)
                &&& final(o).log() == old(o).log() + #[trigger] acquire_log(retries, k, end)
                &&& (r is Ok <==> end == AcquireEnd::Armed(true))
            },
{
    let ghost log0 = o.log();
    o.stage();
    let mut attempt: usize = 0;
    proof {
        assert(o.log() =~= log0 + seq![AcquireEvent::Stage] + retry_log(0));
    }
    loop
        invariant
            attempt <= retries,
            retries < usize::MAX,
            log0 == old(o).log(),
            o.log() == log0 + seq![AcquireEvent::Stage] + retry_log(attempt as nat),
        decreases retries - attempt,
    {
        let ghost before = o.log();
        let created = o.try_create();
        let holds_lock = match &created {
            CreateOutcome::Created(_) => true,
            CreateOutcome::Contended(_) => false,
            CreateOutcome::Failed(_) => false,
        };
        let ghost a = attempt_of(created);
        let checked = o.check_deferred(holds_lock);
        if let Err(interrupted) = checked {
            proof {
                assert(o.log() =~= log0 + acquire_log(retries, attempt as nat, AcquireEnd::Interrupted(a)));
            }
            return Err(interrupted);
        }
        match created {
            CreateOutcome::Created(file) => {
                o.arm();
                let checked = o.check_deferred(true);
                let ghost ok = checked is Ok;
                proof {
                    assert(o.log() =~= log0 + acquire_log(retries, attempt as nat, AcquireEnd::Armed(ok)));
                }
                checked?;
                return Ok(file);
            },
            CreateOutcome::Contended(_) => {
                if attempt < retries {
                    o.wait(attempt);
                    proof {
                        assert(retry_log((attempt + 1) as nat) == retry_log(attempt as nat) + seq![
                            AcquireEvent::Create(Attempt::Contended),
                            AcquireEvent::Check(false, true),
                            AcquireEvent::Wait(attempt),
                        ]);
                        assert(o.log() =~= log0 + seq![AcquireEvent::Stage] + retry_log((attempt + 1) as nat));
                    }
                    attempt += 1;
                } else {
                    let error = o.locked_error(retries + 1);
                    o.disarm();
                    let checked = o.check_deferred(false);
                    let ghost ok = checked is Ok;
                    proof {
                        assert(o.log() =~= log0 + acquire_log(retries, attempt as nat, AcquireEnd::Locked(ok)));
                    }
                    checked?;
                    return Err(error);
                }
            },
            CreateOutcome::Failed(error) => {
                let error = o.create_error(error);
                o.disarm();
                let checked = o.check_deferred(false);
                let ghost ok = checked is Ok;
                proof {
                    assert(o.log() =~= log0 + acquire_log(retries, attempt as nat, AcquireEnd::Failed(ok)));
                }
                checked?;
                return Err(error);
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Theorems about the drivers' effect sequences.
// ---------------------------------------------------------------------------

/// Staging resets the descriptor and writes the path only while the state
/// is `WRITING`: strictly after the store of `WRITING` and strictly before
/// the store of `STAGED`. So no handler can act on a half-written path or an
/// earlier lock's descriptor.
pub proof fn lemma_stage_writes_only_while_writing(fits: bool)
    ensures
        fits ==> stage_events(fits)[1] == StageEvent::Store(WRITING) && stage_events(fits)[2]
            == StageEvent::ResetFd && stage_events(fits)[3] == StageEvent::WritePath
            && stage_events(fits)[4] == StageEvent::Store(STAGED) && stage_events(fits).len() == 5,
        !fits ==> stage_events(fits) == seq![StageEvent::Store(EMPTY)],
{
}

/// The handler removes the lock only immediately after its claim
/// `ARMED -> EMPTY` succeeded, and it returns without re-raising exactly when
/// both of its loads saw `STAGED` (the deferral is then left recorded).
pub proof fn lemma_handler_removes_only_after_claim(signal: i32, first: u8, second: u8, claimed: bool, j: int)
    ensures
        0 <= j < handler_events(signal, first, second, claimed).len() && handler_events(
            signal,
            first,
            second,
            claimed,
        )[j] == CellEvent::Remove ==> j >= 1 && handler_events(signal, first, second, claimed)[j - 1]
            == CellEvent::Cas(CLAIM, true),
        handler_events(signal, first, second, claimed).last() == CellEvent::Raise(signal) <==> !(
        spec_defers(first) && spec_defers(second)),
{
    let ev = handler_events(signal, first, second, claimed);
    let act = claim_events(claimed).push(CellEvent::Raise(signal));
    if spec_defers(first) && spec_defers(second) {
    } else if spec_defers(first) {
        let pre = seq![
            CellEvent::Load(first),
            CellEvent::Defer(signal),
            CellEvent::Load(second),
            CellEvent::Withdraw(signal),
        ];
        assert(ev == pre + act);
        if 0 <= j < ev.len() && ev[j] == CellEvent::Remove {
            assert(j >= 4);
            assert(ev[j] == act[j - 4]);
            assert(ev[j - 1] == act[j - 5]);
        }
    } else {
        let pre = seq![CellEvent::Load(first)];
        assert(ev == pre + act);
        if 0 <= j < ev.len() && ev[j] == CellEvent::Remove {
            assert(j >= 1);
            assert(ev[j] == act[j - 1]);
            if j >= 2 {
                assert(ev[j - 1] == act[j - 2]);
            }
        }
    }
}

/// A resumed signal removes the lock only when the caller holds it and the
/// state it observed was not `EMPTY`, and only right after storing `STAGED`;
/// whenever it re-raises, the store of `EMPTY` (disarm) comes right before.
pub proof fn lemma_resume_removes_only_held_live_lock(holds_lock: bool, taken: i32, observed: u8)
    ensures
        resume_events(holds_lock, taken, observed).contains(CellEvent::Remove) ==> spec_resume_removes(
            holds_lock,
            observed,
        ) && taken != NO_SIGNAL,
        spec_resume_removes(holds_lock, observed) && taken != NO_SIGNAL ==> resume_events(
            holds_lock,
            taken,
            observed,
        ) == seq![
            CellEvent::Take(taken),
            CellEvent::Load(observed),
            CellEvent::Store(STAGED),
            CellEvent::Remove,
            CellEvent::Store(EMPTY),
            CellEvent::Raise(taken),
        ],
        taken != NO_SIGNAL ==> {
            let ev = resume_events(holds_lock, taken, observed);
            &&& ev.last() == CellEvent::Raise(taken)
            &&& ev[ev.len() - 2] == CellEvent::Store(EMPTY)
        },
        taken == NO_SIGNAL ==> resume_events(holds_lock, taken, observed) == seq![CellEvent::Take(taken)],
{
    let ev = resume_events(holds_lock, taken, observed);
    if taken != NO_SIGNAL {
        if holds_lock {
            if spec_resume_removes(holds_lock, observed) {
                assert(ev =~= seq![
                    CellEvent::Take(taken),
                    CellEvent::Load(observed),
                    CellEvent::Store(STAGED),
                    CellEvent::Remove,
                    CellEvent::Store(EMPTY),
                    CellEvent::Raise(taken),
                ]);
            } else {
                assert(ev =~= seq![
                    CellEvent::Take(taken),
                    CellEvent::Load(observed),
                    CellEvent::Store(EMPTY),
                    CellEvent::Raise(taken),
                ]);
                assert(ev[0] != CellEvent::Remove && ev[1] != CellEvent::Remove && ev[2]
                    != CellEvent::Remove && ev[3] != CellEvent::Remove);
            }
        } else {
            assert(ev[0] != CellEvent::Remove && ev[1] != CellEvent::Remove && ev[2]
                != CellEvent::Remove);
        }
    } else {
        assert(ev[0] != CellEvent::Remove);
    }
}

/// Release removes the lock only right after its claim `ARMED -> STAGED`
/// succeeded, and stores `EMPTY` (disarms) before it takes any deferred
/// signal; a release that did not claim the lock does nothing else.
pub proof fn lemma_release_order(begun: bool, taken: i32, observed: u8)
    ensures
        begun ==> release_events(begun, taken, observed).subrange(0, 4) == seq![
            CellEvent::Cas(BEGIN_RELEASE, true),
            CellEvent::Remove,
            CellEvent::Store(EMPTY),
            CellEvent::Take(taken),
        ],
        begun ==> !release_events(begun, taken, observed).subrange(4, release_events(begun, taken, observed).len() as int).contains(CellEvent::Remove),
        !begun ==> release_events(begun, taken, observed) == seq![CellEvent::Cas(BEGIN_RELEASE, false)],
{
    if begun {
        let ev = release_events(begun, taken, observed);
        let tail = resume_events(false, taken, observed);
        lemma_resume_removes_only_held_live_lock(false, taken, observed);
        assert(ev.subrange(4, ev.len() as int) =~= tail.subrange(1, tail.len() as int));
        assert(ev.subrange(0, 4) =~= seq![
            CellEvent::Cas(BEGIN_RELEASE, true),
            CellEvent::Remove,
            CellEvent::Store(EMPTY),
            CellEvent::Take(taken),
        ]);
        if tail.subrange(1, tail.len() as int).contains(CellEvent::Remove) {
            let i = choose|i: int| 0 <= i < tail.subrange(1, tail.len() as int).len() && tail.subrange(1, tail.len() as int)[i] == CellEvent::Remove;
            assert(tail[i + 1] == CellEvent::Remove);
            assert(tail.contains(CellEvent::Remove));
        }
    }
}

/// Number of create attempts in an acquisition log.
pub open spec fn count_creates(log: Seq<AcquireEvent>) -> nat
    decreases log.len(),
{
    if log.len() == 0 {
        0
    } else {
        count_creates(log.drop_last()) + if log.last() is Create {
            1nat
        } else {
            0nat
        }
    }
}

/// Every attempt is immediately followed by a deferred-signal check whose
/// `holds_lock` is whether that attempt created the file.
pub open spec fn attempts_checked(log: Seq<AcquireEvent>) -> bool {
    forall|j: int|
        0 <= j < log.len() && #[trigger] log[j] is Create ==> j + 1 < log.len() && log[j + 1] is Check
            && log[j + 1]->Check_0 == (log[j]->Create_0 == Attempt::Created)
}

proof fn lemma_count_creates_concat(a: Seq<AcquireEvent>, b: Seq<AcquireEvent>)
    ensures
        count_creates(a + b) == count_creates(a) + count_creates(b),
    decreases b.len(),
{
    if b.len() == 0 {
        assert(a + b =~= a);
    } else {
        assert((a + b).drop_last() =~= a + b.drop_last());
        lemma_count_creates_concat(a, b.drop_last());
    }
}

proof fn lemma_count_creates_small(s: Seq<AcquireEvent>)
    requires
        s.len() <= 5,
    ensures
        count_creates(s) == (if s.len() > 0 && s[0] is Create { 1nat } else { 0nat }) + (if s.len() > 1
            && s[1] is Create { 1nat } else { 0nat }) + (if s.len() > 2 && s[2] is Create { 1nat } else { 0nat })
            + (if s.len() > 3 && s[3] is Create { 1nat } else { 0nat }) + (if s.len() > 4 && s[4] is Create {
            1nat
        } else {
            0nat
        }),
    decreases s.len(),
{
    if s.len() > 0 {
        lemma_count_creates_small(s.drop_last());
    }
}

proof fn lemma_attempts_checked_concat(a: Seq<AcquireEvent>, b: Seq<AcquireEvent>)
    requires
        attempts_checked(a),
        attempts_checked(b),
    ensures
        attempts_checked(a + b),
{
    assert forall|j: int|
        0 <= j < (a + b).len() && #[trigger] (a + b)[j] is Create implies j + 1 < (a + b).len() && (a
            + b)[j + 1] is Check && (a + b)[j + 1]->Check_0 == ((a + b)[j]->Create_0 == Attempt::Created) by {
        if j < a.len() {
            assert((a + b)[j] == a[j]);
            assert(a[j] is Create);
            assert((a + b)[j + 1] == a[j + 1]);
        } else {
            assert((a + b)[j] == b[j - a.len()]);
            assert(b[j - a.len()] is Create);
            assert((a + b)[j + 1] == b[j + 1 - a.len()]);
        }
    }
}

proof fn lemma_retry_log(k: nat)
    ensures
        retry_log(k).len() == 3 * k,
        count_creates(retry_log(k)) == k,
        attempts_checked(retry_log(k)),
        forall|j: int| 0 <= j < retry_log(k).len() && #[trigger] retry_log(k)[j] is Wait ==> retry_log(k)[j]->Wait_0 < k,
    decreases k,
{
    if k > 0 {
        let prev = retry_log((k - 1) as nat);
        let block = seq![
            AcquireEvent::Create(Attempt::Contended),
            AcquireEvent::Check(false, true),
            AcquireEvent::Wait((k - 1) as usize),
        ];
        lemma_retry_log((k - 1) as nat);
        lemma_count_creates_concat(prev, block);
        lemma_count_creates_small(block);
        assert(attempts_checked(block)) by {
            assert forall|j: int| 0 <= j < block.len() && #[trigger] block[j] is Create implies j + 1 < block.len()
                && block[j + 1] is Check && block[j + 1]->Check_0 == (block[j]->Create_0 == Attempt::Created) by {
                assert(j == 0);
            }
        }
        lemma_attempts_checked_concat(prev, block);
        assert forall|j: int| 0 <= j < retry_log(k).len() && #[trigger] retry_log(k)[j] is Wait implies retry_log(
            k,
        )[j]->Wait_0 < k by {
            if j < prev.len() {
                assert(retry_log(k)[j] == prev[j]);
            } else {
                assert(retry_log(k)[j] == block[j - prev.len()]);
            }
        }
    }
}

proof fn lemma_final_log(retries: usize, end: AcquireEnd)
    requires
        retries < usize::MAX,
    ensures
        count_creates(final_log(retries, end)) == 1,
        attempts_checked(final_log(retries, end)),
        forall|j: int| 0 <= j < final_log(retries, end).len() ==> !(#[trigger] final_log(retries, end)[j] is Wait),
        forall|j: int|
            0 <= j < final_log(retries, end).len() && #[trigger] final_log(retries, end)[j] is GiveUp
                ==> final_log(retries, end)[j]->GiveUp_0 == retries + 1 && end is Locked,
{
    let t = final_log(retries, end);
    lemma_count_creates_small(t);
    assert forall|j: int| 0 <= j < t.len() && #[trigger] t[j] is Create implies j + 1 < t.len() && t[j + 1] is Check
        && t[j + 1]->Check_0 == (t[j]->Create_0 == Attempt::Created) by {
        assert(j == 0);
    }
}

/// The acquisition loop's guarantees, for every log [`acquire`] can produce:
/// `create_new` runs `k + 1 <= retries + 1` times; every attempt is followed
/// immediately by a deferred-signal check whose `holds_lock` is whether it
/// created the file; every wait uses a delay index below `retries` (so
/// `INDEX_LOCK_RETRY_DELAYS[index]` is in bounds); and a "locked" error
/// reports exactly the number of attempts made.
pub proof fn lemma_acquire_bounded_and_checked(retries: usize, k: nat, end: AcquireEnd)
    requires
        k <= retries,
        retries < usize::MAX,
        end is Locked ==> k == retries,
    ensures
        count_creates(acquire_log(retries, k, end)) == k + 1,
        k + 1 <= retries + 1,
        attempts_checked(acquire_log(retries, k, end)),
        forall|j: int|
            0 <= j < acquire_log(retries, k, end).len() && #[trigger] acquire_log(retries, k, end)[j] is Wait
                ==> acquire_log(retries, k, end)[j]->Wait_0 < retries,
        forall|j: int|
            0 <= j < acquire_log(retries, k, end).len() && #[trigger] acquire_log(retries, k, end)[j] is GiveUp
                ==> acquire_log(retries, k, end)[j]->GiveUp_0 == count_creates(acquire_log(retries, k, end)),
{
    let head = seq![AcquireEvent::Stage];
    let mid = retry_log(k);
    let tail = final_log(retries, end);
    let log = acquire_log(retries, k, end);
    lemma_retry_log(k);
    lemma_final_log(retries, end);
    lemma_count_creates_small(head);
    lemma_count_creates_concat(head, mid);
    lemma_count_creates_concat(head + mid, tail);
    assert(attempts_checked(head));
    lemma_attempts_checked_concat(head, mid);
    lemma_attempts_checked_concat(head + mid, tail);
    assert(log == (head + mid) + tail);
    assert forall|j: int| 0 <= j < log.len() && #[trigger] log[j] is Wait implies log[j]->Wait_0 < retries by {
        if j < 1 {
            assert(log[j] == head[j]);
        } else if j < 1 + mid.len() {
            assert(log[j] == mid[j - 1]);
        } else {
            assert(log[j] == tail[j - 1 - mid.len()]);
        }
    }
    assert forall|j: int| 0 <= j < log.len() && #[trigger] log[j] is GiveUp implies log[j]->GiveUp_0 == count_creates(log) by {
        if j < 1 {
            assert(log[j] == head[j]);
        } else if j < 1 + mid.len() {
            assert(log[j] == mid[j - 1]);
            lemma_retry_log_kinds(k, j - 1);
        } else {
            assert(log[j] == tail[j - 1 - mid.len()]);
        }
    }
}

proof fn lemma_retry_log_kinds(k: nat, j: int)
    requires
        0 <= j < retry_log(k).len(),
    ensures
        retry_log(k)[j] is Create || retry_log(k)[j] is Check || retry_log(k)[j] is Wait,
    decreases k,
{
    if k > 0 {
        let prev = retry_log((k - 1) as nat);
        lemma_retry_log((k - 1) as nat);
        if j < prev.len() {
            lemma_retry_log_kinds((k - 1) as nat, j);
        }
    }
}

} // verus!
