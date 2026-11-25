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
//! - `TempNameAttempts::try_another` grants an attempt iff fewer than
//!   [`TEMP_NAME_ATTEMPTS`] (128) were granted, and counts it; the count never
//!   exceeds 128 (type invariant), so `create_anchored_temp` issues at most
//!   128 exclusive creates and its loop terminates.
//! - `creates_missing_directory` holds iff the walk may create (`create`) and
//!   `openat` reported `ENOENT`; any other failure ends the walk with that
//!   error.
//! - `after_mkdir`: `Created` and `AlreadyExists` (a lost race) both continue
//!   with a re-open of the component through `openat(O_NOFOLLOW |
//!   O_DIRECTORY)` — what `mkdirat` made is never trusted directly — and the
//!   new directory and its parent are synced iff this run's `mkdirat` created
//!   it; any other `mkdirat` error ends the walk with that error.
//!
//! # Trust assumptions
//!
//! - Production maps `CopyStrategy` to [`Strategy`] variant for variant and
//!   passes `cfg!(target_os = "macos")` as the platform preference.
//! - Production calls `try_another` once before each exclusive create and
//!   retries only when that create failed with `EEXIST`.
//! - Production classifies `mkdirat`'s answer faithfully (`Ok`, `EEXIST`,
//!   other) and re-opens the component whenever `after_mkdir` returns `Ok`.

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

/// The attempts granted so far to `create_anchored_temp`.
pub struct TempNameAttempts {
    made: u32,
}

impl TempNameAttempts {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        self.made <= TEMP_NAME_ATTEMPTS
    }

    /// Attempts granted so far.
    pub closed spec fn made(&self) -> nat {
        self.made as nat
    }

    /// No attempt granted yet.
    pub fn new() -> (r: TempNameAttempts)
        ensures
            r.made() == 0,
    {
        TempNameAttempts { made: 0 }
    }

    /// Grant one more attempt, unless the budget is spent.
    pub fn try_another(&mut self) -> (r: bool)
        ensures
            r == (old(self).made() < TEMP_NAME_ATTEMPTS),
            final(self).made() == if r {
                old(self).made() + 1
            } else {
                old(self).made()
            },
            final(self).made() <= TEMP_NAME_ATTEMPTS,
    {
        proof {
            use_type_invariant(&*self);
        }
        if self.made < TEMP_NAME_ATTEMPTS {
            *self = TempNameAttempts { made: self.made + 1 };
            true
        } else {
            false
        }
    }
}

impl Default for TempNameAttempts {
    fn default() -> (r: TempNameAttempts)
        ensures
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
