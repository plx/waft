//! Small decisions around anchored publication: reflink attempts, the
//! staging-name retry budget, and what a `mkdirat` answer means during the
//! anchored directory walk.
//!
//! # What the kernel decides
//!
//! - [`attempts_reflink`]: whether a copy first tries a copy-on-write clone
//!   (`try_reflink` in `copy_file_anchored_unix` and the non-Unix
//!   `copy_file_path_fallback`).
//! - [`TempNameAttempts`]: how many fresh `.waft-copy-*` names
//!   `create_anchored_temp` may try before giving up.
//! - [`creates_missing_directory`] and [`after_mkdir`]: when
//!   `open_relative_parent` creates a missing intermediate directory, and
//!   whether the directory it then re-opens must be synced.
//!
//! # Theorems
//!
//! - `attempts_reflink` equals `spec_attempts_reflink`, the documented
//!   strategy table: `SimpleCopy` never attempts a reflink on any platform,
//!   `CowCopy` always does, and `Auto` does exactly where the platform default
//!   is a reflink (macOS) — `theorem_strategy_table`.
//! - `TempNameAttempts::try_another`, on a counter that has answered `calls`
//!   calls since `TempNameAttempts::new`, grants an attempt iff `calls < 128`:
//!   a fresh counter grants exactly its first 128 calls and refuses every
//!   later one. The attempts granted so far are `min(calls, 128)`
//!   (`granted_after`, kept by the type invariant). The budget is stated as
//!   the literal 128 in these postconditions, so [`TEMP_NAME_ATTEMPTS`] cannot
//!   change without the specification changing with it.
//! - `creates_missing_directory` holds iff the walk may create (`create`) and
//!   `openat` reported `ENOENT`.
//! - `after_mkdir` is `Ok(true)` iff `mkdirat` answered `Created`, `Ok(false)`
//!   iff it answered `AlreadyExists` (a lost race), and `Err(e)` iff it
//!   answered `Failed(e)`.
//!
//! # Trust assumptions
//!
//! - Production maps `CopyStrategy` to [`Strategy`] variant for variant and
//!   passes `cfg!(target_os = "macos")` as the platform preference.
//! - Production creates one fresh `TempNameAttempts` per
//!   `create_anchored_temp` call, calls `try_another` once before each
//!   exclusive create, and retries only when that create failed with
//!   `EEXIST`. With the `try_another` postconditions above, this bounds each
//!   call to at most 128 exclusive creates and makes its loop terminate;
//!   neither bound is proved about production itself.
//! - Production classifies `mkdirat`'s answer faithfully (`Ok`, `EEXIST`,
//!   other) and acts on the kernel's answers as follows, none of which the
//!   kernel proves: when `creates_missing_directory` is false, any `openat`
//!   failure ends the walk with that error; when `after_mkdir` is `Ok`, the
//!   component is re-opened through `openat(O_NOFOLLOW | O_DIRECTORY)` (what
//!   `mkdirat` made is never trusted directly), and on `Ok(true)` the new
//!   directory and its parent are synced; on `Err(e)` the walk ends with
//!   `e`.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// The copy mechanism requested (production `CopyStrategy`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    /// The platform default: a reflink on macOS, a plain copy elsewhere.
    Auto,
    /// Always a plain byte-for-byte copy.
    SimpleCopy,
    /// Attempt a reflink, falling back to a plain copy if unsupported.
    CowCopy,
}

/// Whether a copy with `strategy` first attempts a reflink, on a platform
/// whose default is (`platform_prefers_reflink`) or is not a reflink.
pub open spec fn spec_attempts_reflink(strategy: Strategy, platform_prefers_reflink: bool) -> bool {
    strategy == Strategy::CowCopy || (strategy == Strategy::Auto && platform_prefers_reflink)
}

/// Whether a copy with `strategy` first attempts a reflink.
pub fn attempts_reflink(strategy: Strategy, platform_prefers_reflink: bool) -> (r: bool)
    ensures
        r == spec_attempts_reflink(strategy, platform_prefers_reflink),
{
    match strategy {
        Strategy::SimpleCopy => false,
        Strategy::CowCopy => true,
        Strategy::Auto => platform_prefers_reflink,
    }
}

/// The strategy table as documented on `CopyStrategy`.
pub proof fn theorem_strategy_table(platform_prefers_reflink: bool)
    ensures
        !spec_attempts_reflink(Strategy::SimpleCopy, platform_prefers_reflink),
        spec_attempts_reflink(Strategy::CowCopy, platform_prefers_reflink),
        spec_attempts_reflink(Strategy::Auto, platform_prefers_reflink)
            == platform_prefers_reflink,
{
}

/// How many fresh staging names `create_anchored_temp` tries.
pub const TEMP_NAME_ATTEMPTS: u32 = 128;

/// The attempts a fresh counter has granted after answering `calls` calls
/// of `try_another`: one per call until 128 were granted, then none.
pub open spec fn granted_after(calls: nat) -> nat {
    if calls < 128 {
        calls
    } else {
        128
    }
}

/// The attempts granted so far to `create_anchored_temp`.
pub struct TempNameAttempts {
    made: u32,
    #[allow(dead_code)]
    calls: Ghost<nat>,
}

impl TempNameAttempts {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        self.made as nat == granted_after(self.calls@)
    }

    /// Attempts granted so far.
    pub closed spec fn made(&self) -> nat {
        self.made as nat
    }

    /// Calls of `try_another` answered so far.
    pub closed spec fn calls(&self) -> nat {
        self.calls@
    }

    /// No attempt asked for or granted yet.
    pub fn new() -> (r: TempNameAttempts)
        ensures
            r.calls() == 0,
            r.made() == 0,
    {
        TempNameAttempts { made: 0, calls: Ghost(0) }
    }

    /// Grant one more attempt, unless the budget is spent: the call is
    /// granted iff fewer than 128 calls were answered before it.
    pub fn try_another(&mut self) -> (r: bool)
        ensures
            r == (old(self).calls() < 128),
            final(self).calls() == old(self).calls() + 1,
            final(self).made() == granted_after(final(self).calls()),
            final(self).made() == if r {
                old(self).made() + 1
            } else {
                old(self).made()
            },
            final(self).made() <= 128,
    {
        proof {
            use_type_invariant(&*self);
        }
        let ghost calls = self.calls@ + 1;
        if self.made < TEMP_NAME_ATTEMPTS {
            *self = TempNameAttempts { made: self.made + 1, calls: Ghost(calls) };
            true
        } else {
            *self = TempNameAttempts { made: self.made, calls: Ghost(calls) };
            false
        }
    }
}

impl Default for TempNameAttempts {
    fn default() -> (r: TempNameAttempts)
        ensures
            r.calls() == 0,
            r.made() == 0,
    {
        TempNameAttempts::new()
    }
}

/// Whether `open_relative_parent` creates a missing intermediate directory
/// after `openat` failed.
pub fn creates_missing_directory(create: bool, not_found: bool) -> (r: bool)
    ensures
        r == (create && not_found),
{
    create && not_found
}

/// What `mkdirat` of a missing intermediate directory answered.
pub enum MkdirAnswer<E> {
    /// This run created the directory.
    Created,
    /// The name exists already (another process won the race).
    AlreadyExists,
    /// Any other failure.
    Failed(E),
}

/// After `mkdirat`: `Ok(sync)` means re-open the component (never trusting
/// what `mkdirat` made) and, iff `sync`, fsync it and its parent; `Err`
/// ends the walk with `mkdirat`'s error.
pub fn after_mkdir<E>(answer: MkdirAnswer<E>) -> (r: Result<bool, E>)
    ensures
        r is Ok <==> !(answer is Failed),
        r == Ok::<bool, E>(true) <==> answer is Created,
        answer matches MkdirAnswer::Failed(e) ==> r == Err::<bool, E>(e),
{
    match answer {
        MkdirAnswer::Created => Ok(true),
        MkdirAnswer::AlreadyExists => Ok(false),
        MkdirAnswer::Failed(error) => Err(error),
    }
}

} // verus!
