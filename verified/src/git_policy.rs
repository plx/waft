//! Verified kernel: Git backend selection and configuration value policy.
//!
//! # What it decides
//!
//! - Which Git backend `WAFT_GIT_BACKEND` selects ([`select_backend`]).
//! - Whether a checkout folds case ([`case_folding_applies`]) and how
//!   `git config --bool` output reads ([`parse_bool_output`]).
//! - When the per-repository index snapshot cache may answer, and when a
//!   freshly built snapshot may be stored ([`cached_state_is_current`],
//!   [`may_cache_build`]).
//! - Whether a `git` failure message reports a missing repository
//!   ([`mentions_missing_repository`]).
//! - Whether a walked entry counts as an existing `.worktreeinclude`, which
//!   gates the `when_missing` behaviour ([`rule_file_counts`]).
//!
//! # Theorems
//!
//! - `theorem_backend_selection`: unset selects gix; a non-Unicode value is
//!   an error; a value selects gix or cli exactly when its ASCII lowercase is
//!   `gix` or `cli`, and is otherwise an error (no silent fallback).
//! - `theorem_configured_case_policy_wins`: an explicit `core.ignoreCase`
//!   decides; only an absent key defers to the platform.
//! - `theorem_cache_serves_only_attributable_snapshots`: over any sequence
//!   of cache calls and index changes, a snapshot is served only when the
//!   index stamp observed at the start of the serving call is present and
//!   equals the stamps taken immediately before and after that snapshot was
//!   built. A missing index is never served from, or stored in, the cache.
//! - `theorem_rule_file_existence`: directories and special files never
//!   count as rule files, and under the `Ignore` symlink policy a symlinked
//!   one never counts, so existence agrees with selection.
//! - `mentions_missing_repository` is exactly contiguous substring search
//!   for `not a git repository`.
//!
//! # Trust assumptions
//!
//! - Production trims the environment value with `str::trim` and reports a
//!   non-Unicode value as such; error text is formatted in production.
//! - [`IndexStamp`] faithfully and injectively encodes the index file's
//!   identity, size and modification time, and equal stamps mean equal index
//!   contents (Git publishes the index by rename; inode reuse within the
//!   timestamp granularity is the residual risk).
//! - The discovery phrase is matched against the message production
//!   lowercased with `str::to_lowercase`.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

// ---------------------------------------------------------------------------
// Backend selection
// ---------------------------------------------------------------------------
/// The Git backends waft can use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum BackendKind {
    /// The in-process `gix` backend (the default).
    Gix,
    /// The `git` subprocess backend.
    Cli,
}

/// The state of the `WAFT_GIT_BACKEND` environment variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum BackendSetting {
    /// The variable is not set.
    Unset,
    /// The variable is set but not valid Unicode.
    NotUnicode,
    /// The variable is set; its trimmed value is passed alongside.
    Set,
}

/// Why `WAFT_GIT_BACKEND` selects no backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum BackendError {
    /// The value is not valid Unicode.
    NotUnicode,
    /// The value names no known backend.
    Unknown,
}

/// ASCII lowercase of one byte.
pub open spec fn ascii_lower(b: u8) -> u8 {
    if b'A' <= b <= b'Z' {
        (b + 32) as u8
    } else {
        b
    }
}

/// Bytewise ASCII lowercase (what `str::to_ascii_lowercase` does to bytes).
pub open spec fn ascii_lower_seq(s: Seq<u8>) -> Seq<u8> {
    s.map_values(|b: u8| ascii_lower(b))
}

/// `"gix"`.
pub open spec fn gix_name() -> Seq<u8> {
    seq![b'g', b'i', b'x']
}

/// `"cli"`.
pub open spec fn cli_name() -> Seq<u8> {
    seq![b'c', b'l', b'i']
}

/// The backend a setting selects; `trimmed` is the trimmed value when set.
pub open spec fn backend_spec(setting: BackendSetting, trimmed: Seq<u8>) -> Result<
    BackendKind,
    BackendError,
> {
    match setting {
        BackendSetting::Unset => Ok(BackendKind::Gix),
        BackendSetting::NotUnicode => Err(BackendError::NotUnicode),
        BackendSetting::Set => if ascii_lower_seq(trimmed) == gix_name() {
            Ok(BackendKind::Gix)
        } else if ascii_lower_seq(trimmed) == cli_name() {
            Ok(BackendKind::Cli)
        } else {
            Err(BackendError::Unknown)
        },
    }
}

#[allow(clippy::manual_is_ascii_check)]
fn lower_byte(b: u8) -> (r: u8)
    ensures
        r == ascii_lower(b),
{
    if (b'A'..=b'Z').contains(&b) {
        b + 32
    } else {
        b
    }
}

/// Whether `s` equals the lowercase ASCII `name` ignoring ASCII case.
fn eq_ignore_ascii_case(s: &[u8], name: &[u8]) -> (r: bool)
    ensures
        r == (ascii_lower_seq(s@) == name@),
{
    if s.len() != name.len() {
        assert(ascii_lower_seq(s@).len() != name@.len());
        return false;
    }
    let mut i: usize = 0;
    while i < s.len()
        invariant
            i <= s@.len(),
            s@.len() == name@.len(),
            forall|j: int| 0 <= j < i ==> ascii_lower(s@[j]) == #[trigger] name@[j],
        decreases s.len() - i,
    {
        if lower_byte(s[i]) != name[i] {
            assert(ascii_lower_seq(s@)[i as int] != name@[i as int]);
            return false;
        }
        i += 1;
    }
    assert(ascii_lower_seq(s@) =~= name@);
    true
}

/// Select the Git backend from `WAFT_GIT_BACKEND`.
pub fn select_backend(setting: BackendSetting, trimmed: &[u8]) -> (r: Result<
    BackendKind,
    BackendError,
>)
    ensures
        r == backend_spec(setting, trimmed@),
{
    match setting {
        BackendSetting::Unset => Ok(BackendKind::Gix),
        BackendSetting::NotUnicode => Err(BackendError::NotUnicode),
        BackendSetting::Set => {
            let gix: [u8; 3] = [b'g', b'i', b'x'];
            let cli: [u8; 3] = [b'c', b'l', b'i'];
            assert(gix@ =~= gix_name());
            assert(cli@ =~= cli_name());
            if eq_ignore_ascii_case(trimmed, gix.as_slice()) {
                Ok(BackendKind::Gix)
            } else if eq_ignore_ascii_case(trimmed, cli.as_slice()) {
                Ok(BackendKind::Cli)
            } else {
                Err(BackendError::Unknown)
            }
        },
    }
}

/// Backend selection never falls back silently: an unset variable selects
/// gix, and a set value selects a backend only when it is one of the two
/// names (in any ASCII case); everything else is an error.
pub proof fn theorem_backend_selection(setting: BackendSetting, trimmed: Seq<u8>)
    ensures
        setting == BackendSetting::Unset ==> backend_spec(setting, trimmed) == Ok::<
            BackendKind,
            BackendError,
        >(BackendKind::Gix),
        setting == BackendSetting::NotUnicode ==> backend_spec(setting, trimmed) is Err,
        setting == BackendSetting::Set ==> (backend_spec(setting, trimmed) is Ok <==> (
        ascii_lower_seq(trimmed) == gix_name() || ascii_lower_seq(trimmed) == cli_name())),
        setting == BackendSetting::Set && backend_spec(setting, trimmed) == Ok::<
            BackendKind,
            BackendError,
        >(BackendKind::Cli) ==> ascii_lower_seq(trimmed) == cli_name(),
{
    if setting == BackendSetting::Set && ascii_lower_seq(trimmed) == gix_name() {
        assert(gix_name()[0] != cli_name()[0]);
    }
}

// ---------------------------------------------------------------------------
// Case policy and boolean config output
// ---------------------------------------------------------------------------
/// Whether differently-cased spellings name the same repository path:
/// an explicit `core.ignoreCase` decides, otherwise the platform default.
pub open spec fn case_folding_spec(configured: Option<bool>, platform_folds_case: bool) -> bool {
    match configured {
        Some(b) => b,
        None => platform_folds_case,
    }
}

/// Decide `case_folding_spec`.
pub fn case_folding_applies(configured: Option<bool>, platform_folds_case: bool) -> (r: bool)
    ensures
        r == case_folding_spec(configured, platform_folds_case),
{
    match configured {
        Some(b) => b,
        None => platform_folds_case,
    }
}

/// An explicitly configured value is authoritative in both directions; only
/// an absent key defers to the platform.
pub proof fn theorem_configured_case_policy_wins(configured: Option<bool>, platform: bool)
    ensures
        configured == Some(true) ==> case_folding_spec(configured, platform),
        configured == Some(false) ==> !case_folding_spec(configured, platform),
        configured is None ==> case_folding_spec(configured, platform) == platform,
{
}

/// `"true"`.
pub open spec fn true_literal() -> Seq<u8> {
    seq![b't', b'r', b'u', b'e']
}

/// `"false"`.
pub open spec fn false_literal() -> Seq<u8> {
    seq![b'f', b'a', b'l', b's', b'e']
}

/// Read trimmed `git config --bool` output: exactly `true` or `false`,
/// anything else is no answer.
pub open spec fn bool_output_spec(trimmed: Seq<u8>) -> Option<bool> {
    if trimmed == true_literal() {
        Some(true)
    } else if trimmed == false_literal() {
        Some(false)
    } else {
        None
    }
}

/// Decide `bool_output_spec`.
pub fn parse_bool_output(trimmed: &[u8]) -> (r: Option<bool>)
    ensures
        r == bool_output_spec(trimmed@),
{
    let t: [u8; 4] = [b't', b'r', b'u', b'e'];
    let f: [u8; 5] = [b'f', b'a', b'l', b's', b'e'];
    assert(t@ =~= true_literal());
    assert(f@ =~= false_literal());
    if trimmed == t.as_slice() {
        assert(trimmed@ =~= t@);
        Some(true)
    } else if trimmed == f.as_slice() {
        assert(trimmed@ =~= f@);
        Some(false)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Index snapshot cache
// ---------------------------------------------------------------------------
/// Identifying stat data of an index file, as plain data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub struct IndexStamp {
    /// Whether the file's identity was available.
    pub has_identity: bool,
    /// Device (or volume serial number); 0 without an identity.
    pub dev: u64,
    /// Inode (or file index); 0 without an identity.
    pub ino: u64,
    /// File size in bytes.
    pub size: u64,
    /// Whether the modification time was available.
    pub has_mtime: bool,
    /// Whether the modification time lies before the Unix epoch.
    pub mtime_before_epoch: bool,
    /// Whole seconds between the modification time and the epoch.
    pub mtime_secs: u64,
    /// Remaining nanoseconds between the modification time and the epoch.
    pub mtime_nanos: u32,
}

/// A cached snapshot may answer only when the index was observed now and
/// its stamp is the one the snapshot was stored under.
pub open spec fn serve_spec(now: Option<IndexStamp>, stored: Option<IndexStamp>) -> bool {
    now is Some && stored == now
}

/// A freshly built snapshot may be stored only when the index was observed
/// before the build and did not change while it was being read.
pub open spec fn store_spec(before: Option<IndexStamp>, after: Option<IndexStamp>) -> bool {
    before is Some && after == before
}

/// Decide `serve_spec`.
pub fn cached_state_is_current(now: Option<IndexStamp>, stored: Option<IndexStamp>) -> (r: bool)
    ensures
        r == serve_spec(now, stored),
{
    match (now, stored) {
        (Some(a), Some(b)) => a == b,
        (Some(_), None) => false,
        (None, _) => false,
    }
}

/// Decide `store_spec`.
pub fn may_cache_build(before: Option<IndexStamp>, after: Option<IndexStamp>) -> (r: bool)
    ensures
        r == store_spec(before, after),
{
    match (before, after) {
        (Some(a), Some(b)) => a == b,
        (Some(_), None) => false,
        (None, _) => false,
    }
}

/// A cache slot: the stamp a snapshot was stored under, and the stamps
/// taken immediately before and after that snapshot was built.
pub struct CacheSlot {
    /// The stamp the snapshot is stored under.
    pub stored: IndexStamp,
    /// The stamp taken before the build.
    pub built_before: Option<IndexStamp>,
    /// The stamp taken after the build.
    pub built_after: Option<IndexStamp>,
}

/// One `index_state` call: with the stamp `now` observed at its start and,
/// when it builds, the stamp `after` observed once the build finished.
/// Returns whether the cached snapshot was served, and the slot afterwards.
pub open spec fn cache_step(slot: Option<CacheSlot>, now: Option<IndexStamp>, after: Option<
    IndexStamp,
>) -> (bool, Option<CacheSlot>) {
    let stored = match slot {
        Some(s) => Some(s.stored),
        None => None,
    };
    if serve_spec(now, stored) {
        (true, slot)
    } else if store_spec(now, after) {
        (false, Some(CacheSlot { stored: now->Some_0, built_before: now, built_after: after }))
    } else {
        (false, slot)
    }
}

/// Every slot is attributable: its snapshot was built between two equal,
/// present stamps, and it is stored under exactly that stamp.
pub open spec fn slot_attributable(slot: Option<CacheSlot>) -> bool {
    match slot {
        Some(s) => s.built_before == Some(s.stored) && s.built_after == Some(s.stored),
        None => true,
    }
}

/// Over any sequence of calls (with arbitrary index changes in between), the
/// cache stays attributable, and whenever it serves a snapshot, that
/// snapshot was built while the index carried exactly the stamp observed at
/// the start of the serving call. A missing index (`now == None`) is never
/// served and never stored.
pub proof fn theorem_cache_serves_only_attributable_snapshots(
    slot: Option<CacheSlot>,
    now: Option<IndexStamp>,
    after: Option<IndexStamp>,
)
    requires
        slot_attributable(slot),
    ensures
        slot_attributable(cache_step(slot, now, after).1),
        cache_step(slot, now, after).0 ==> now is Some && slot is Some && slot->Some_0.built_before
            == now && slot->Some_0.built_after == now,
        now is None ==> !cache_step(slot, now, after).0 && cache_step(slot, now, after).1 == slot,
{
}

// ---------------------------------------------------------------------------
// Discovery failures
// ---------------------------------------------------------------------------
/// `"not a git repository"`.
pub open spec fn missing_repository_phrase() -> Seq<u8> {
    seq![
        b'n', b'o', b't', b' ', b'a', b' ', b'g', b'i', b't', b' ', b'r', b'e', b'p', b'o', b's',
        b'i', b't', b'o', b'r', b'y'
    ]
}

/// `needle` occurs contiguously in `hay`.
pub open spec fn contains_bytes(hay: Seq<u8>, needle: Seq<u8>) -> bool {
    exists|i: int|
        0 <= i && i + needle.len() <= hay.len() && #[trigger] hay.subrange(i, i + needle.len())
            == needle
}

/// Whether the (already lowercased) message says the path is not inside a
/// Git repository.
pub fn mentions_missing_repository(lowercased: &[u8]) -> (r: bool)
    ensures
        r == contains_bytes(lowercased@, missing_repository_phrase()),
{
    let phrase: [u8; 20] = [
        b'n', b'o', b't', b' ', b'a', b' ', b'g', b'i', b't', b' ', b'r', b'e', b'p', b'o', b's',
        b'i', b't', b'o', b'r', b'y',
    ];
    assert(phrase@ =~= missing_repository_phrase());
    let n = phrase.len();
    if lowercased.len() < n {
        return false;
    }
    let last = lowercased.len() - n;
    let mut i: usize = 0;
    while i <= last
        invariant
            n == phrase@.len(),
            phrase@ == missing_repository_phrase(),
            last + n == lowercased@.len(),
            lowercased@.len() <= usize::MAX,
            i <= last + 1,
            forall|j: int|
                0 <= j < i ==> #[trigger] lowercased@.subrange(j, j + n as int) != phrase@,
        decreases last + 1 - i,
    {
        let window = &lowercased[i..i + n];
        if window == phrase.as_slice() {
            assert(window@ =~= lowercased@.subrange(i as int, i + n));
            assert(window@ =~= phrase@);
            return true;
        }
        assert(window@ =~= lowercased@.subrange(i as int, i + n));
        i += 1;
    }
    false
}

// ---------------------------------------------------------------------------
// Rule-file existence
// ---------------------------------------------------------------------------
/// What a walked directory entry is (without following symlinks).
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum EntryKind {
    /// A directory.
    Dir,
    /// A regular file.
    File,
    /// A symbolic link.
    Symlink,
    /// Anything else (socket, FIFO, device).
    Other,
}

/// How symlinked rule files are treated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum RuleSymlinks {
    /// Followed as rule files.
    Follow,
    /// Treated as absent.
    Ignore,
    /// Reported as errors later; they still exist.
    Error,
}

/// Whether a walked entry counts as an existing `.worktreeinclude`: its
/// name must match, and it must be a regular file, or a symlink when
/// symlinked rule files are not ignored.
pub open spec fn rule_file_counts_spec(kind: EntryKind, name_matches: bool, policy: RuleSymlinks) -> bool {
    name_matches && (kind == EntryKind::File || (kind == EntryKind::Symlink && policy
        != RuleSymlinks::Ignore))
}

/// Decide [`rule_file_counts_spec`].
pub fn rule_file_counts(kind: EntryKind, name_matches: bool, policy: RuleSymlinks) -> (r: bool)
    ensures
        r == rule_file_counts_spec(kind, name_matches, policy),
{
    if !name_matches {
        return false;
    }
    match kind {
        EntryKind::File => true,
        EntryKind::Symlink => match policy {
            RuleSymlinks::Ignore => false,
            RuleSymlinks::Follow => true,
            RuleSymlinks::Error => true,
        },
        EntryKind::Dir => false,
        EntryKind::Other => false,
    }
}

/// Directories and special files never count as rule files; under
/// `Ignore` a symlinked rule file never counts, so existence agrees with
/// selection, which treats it as absent.
pub proof fn theorem_rule_file_existence(kind: EntryKind, name_matches: bool, policy: RuleSymlinks)
    ensures
        kind == EntryKind::Dir || kind == EntryKind::Other ==> !rule_file_counts_spec(
            kind,
            name_matches,
            policy,
        ),
        kind == EntryKind::Symlink && policy == RuleSymlinks::Ignore ==> !rule_file_counts_spec(
            kind,
            name_matches,
            policy,
        ),
        !name_matches ==> !rule_file_counts_spec(kind, name_matches, policy),
{
}

} // verus!
