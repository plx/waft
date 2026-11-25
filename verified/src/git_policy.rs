//! Verified kernel: Git backend selection and configuration value policy.
//!
//! # What it decides
//!
//! - Which Git backend `WAFT_GIT_BACKEND` selects ([`select_backend`]).
//! - Whether a checkout folds case ([`case_folding_applies`]) and how
//!   `git config --bool` output reads ([`parse_bool_output`]).
//! - The whole per-repository index snapshot cache step ([`index_state`]):
//!   when to stat the index, when the cached snapshot may answer, when to
//!   build, and when a freshly built snapshot may be stored. Production only
//!   supplies the effects ([`IndexCacheIo`]).
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
//! - `index_state` performs exactly the effects `cache_call_trace`
//!   describes, in that order, and returns the served or built snapshot.
//! - `theorem_cache_call_effects`: the index is stat-ed before anything
//!   else; a missing index is never looked up or stored; a snapshot is
//!   stored only under a stamp observed both at the start of the call and
//!   immediately after that very snapshot was built; a call that builds
//!   nothing serves a snapshot stored under exactly the stamp it observed
//!   first.
//! - `theorem_cache_serves_only_attributable_snapshots`: over any sequence
//!   of calls, with the index changing arbitrarily between and during them,
//!   a served snapshot was built by an earlier call that observed the
//!   serving call's first stamp both before and right after the build. A
//!   missing index is never served from, or stored in, the cache.
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
//! - [`IndexCacheIo`]: production's effects do what their names say:
//!   `observe_index` stats the index when called, `build` reads the index as
//!   it stands, and the slot (`look_up`, `store`) returns what was last
//!   stored for the repository.
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

/// One effect of an [`index_state`] call, with what it returned.
pub enum CacheEvent<T> {
    /// The index file was stat-ed: its stamp, or `None` when it could not be.
    Observed(Option<IndexStamp>),
    /// The repository's cache slot was read: the stamp and snapshot stored
    /// in it, if any.
    LookedUp(Option<(IndexStamp, T)>),
    /// A fresh snapshot was built from the index: the snapshot, or `None`
    /// when the build failed.
    Built(Option<T>),
    /// The snapshot was stored in the repository's slot under the stamp.
    Stored(IndexStamp, T),
}

/// The snapshot a build produced, or `None` when it failed.
pub open spec fn built_snapshot<T, E>(r: Result<T, E>) -> Option<T> {
    match r {
        Ok(x) => Some(x),
        Err(_) => None,
    }
}

/// The index snapshot cache's effects for one repository, implemented by
/// production.
///
/// Trusted: `observe_index` stats the index file at the moment it is
/// called; `look_up` returns what the most recent `store` for this
/// repository put in its slot, or `None`; `build` reads a fresh snapshot
/// from the index as it stands; `store` replaces the slot's contents.
/// `events` is a ghost log of every effect and what it returned, which lets
/// the kernel state exactly which effects it performs, and in which order.
pub trait IndexCacheIo<T, E> {
    /// Every effect so far, in order.
    spec fn events(&self) -> Seq<CacheEvent<T>>;

    /// Stat the index file.
    fn observe_index(&mut self) -> (stamp: Option<IndexStamp>)
        ensures
            final(self).events() == old(self).events().push(CacheEvent::Observed(stamp)),
    ;

    /// Read the repository's cache slot.
    fn look_up(&mut self) -> (slot: Option<(IndexStamp, T)>)
        ensures
            final(self).events() == old(self).events().push(CacheEvent::LookedUp(slot)),
    ;

    /// Build a fresh snapshot from the index.
    fn build(&mut self) -> (snapshot: Result<T, E>)
        ensures
            final(self).events() == old(self).events().push(
                CacheEvent::Built(built_snapshot(snapshot)),
            ),
    ;

    /// Store `snapshot` in the repository's slot under `stamp`.
    fn store(&mut self, stamp: IndexStamp, snapshot: &T)
        ensures
            final(self).events() == old(self).events().push(CacheEvent::Stored(stamp, *snapshot)),
    ;
}

/// Whether the slot `hit` answers a call that first observed `now`: the
/// slot holds a snapshot stored under exactly that stamp.
pub open spec fn slot_serves<T>(now: Option<IndexStamp>, hit: Option<(IndexStamp, T)>) -> bool {
    match hit {
        Some(h) => serve_spec(now, Some(h.0)),
        None => false,
    }
}

/// The effects one [`index_state`] call performs, given what they returned:
/// the stamp `now` observed first, the slot `hit` read, the snapshot `built`
/// (`None`: the build failed), and the stamp `after` observed once the build
/// finished. Arguments the call never reaches are ignored.
///
/// The index is stat-ed first. When that fails, a snapshot is built and
/// nothing else happens. Otherwise the slot is read and a snapshot stored
/// under exactly the observed stamp is served. Otherwise a snapshot is built
/// (a failed build ends the call), the index is stat-ed again, and the build
/// is stored under the first stamp when the second stat observed it too.
pub open spec fn cache_call_trace<T>(
    now: Option<IndexStamp>,
    hit: Option<(IndexStamp, T)>,
    built: Option<T>,
    after: Option<IndexStamp>,
) -> Seq<CacheEvent<T>> {
    match now {
        None => seq![CacheEvent::Observed(now), CacheEvent::Built(built)],
        Some(stamp) => if slot_serves(now, hit) {
            seq![CacheEvent::Observed(now), CacheEvent::LookedUp(hit)]
        } else {
            match built {
                None => seq![
                    CacheEvent::Observed(now),
                    CacheEvent::LookedUp(hit),
                    CacheEvent::Built(built),
                ],
                Some(x) => if store_spec(now, after) {
                    seq![
                        CacheEvent::Observed(now),
                        CacheEvent::LookedUp(hit),
                        CacheEvent::Built(built),
                        CacheEvent::Observed(after),
                        CacheEvent::Stored(stamp, x),
                    ]
                } else {
                    seq![
                        CacheEvent::Observed(now),
                        CacheEvent::LookedUp(hit),
                        CacheEvent::Built(built),
                        CacheEvent::Observed(after),
                    ]
                },
            }
        },
    }
}

/// The snapshot the call returns (`None`: it returns the build's error):
/// the served one, or else whatever the build produced.
pub open spec fn cache_call_result<T>(
    now: Option<IndexStamp>,
    hit: Option<(IndexStamp, T)>,
    built: Option<T>,
) -> Option<T> {
    if now is Some && slot_serves(now, hit) {
        Some(hit->Some_0.1)
    } else {
        built
    }
}

/// Answer one index snapshot request for a repository: serve the cached
/// snapshot when the index still carries the stamp it was stored under;
/// otherwise build a fresh one, and store it only when the index did not
/// change while it was being built. A missing index is never cached.
///
/// Its `ensures` pins the exact effects and their order
/// (`cache_call_trace`) and the returned snapshot.
// Verus does not support `let` chains, so the serve check stays nested.
#[allow(clippy::collapsible_if)]
pub fn index_state<T, E, Io: IndexCacheIo<T, E>>(io: &mut Io) -> (r: Result<T, E>)
    ensures
        exists|
            now: Option<IndexStamp>,
            hit: Option<(IndexStamp, T)>,
            built: Option<T>,
            after: Option<IndexStamp>,
        |
            final(io).events() == old(io).events() + #[trigger] cache_call_trace(
                now,
                hit,
                built,
                after,
            ) && cache_call_result(now, hit, built) == built_snapshot(r),
{
    let ghost e0 = io.events();
    let now = io.observe_index();
    let stamp = match now {
        Some(stamp) => stamp,
        None => {
            let r = io.build();
            proof {
                let built = built_snapshot(r);
                assert(io.events() =~= e0 + cache_call_trace(
                    now,
                    None::<(IndexStamp, T)>,
                    built,
                    None,
                ));
                assert(cache_call_result(now, None::<(IndexStamp, T)>, built) == built);
            }
            return r;
        },
    };
    let hit = io.look_up();
    let ghost slot = hit;
    if let Some((stored, snapshot)) = hit {
        if cached_state_is_current(now, Some(stored)) {
            proof {
                assert(slot_serves(now, slot));
                assert(io.events() =~= e0 + cache_call_trace(now, slot, None::<T>, None));
                assert(cache_call_result(now, slot, None::<T>) == Some(snapshot));
            }
            return Ok(snapshot);
        }
    }
    proof {
        assert(!slot_serves(now, slot));
    }
    let r = io.build();
    let snapshot = match r {
        Ok(snapshot) => snapshot,
        Err(e) => {
            proof {
                assert(io.events() =~= e0 + cache_call_trace(now, slot, None::<T>, None));
                assert(cache_call_result(now, slot, None::<T>) == None::<T>);
            }
            return Err(e);
        },
    };
    let after = io.observe_index();
    if may_cache_build(now, after) {
        io.store(stamp, &snapshot);
    }
    proof {
        assert(io.events() =~= e0 + cache_call_trace(now, slot, Some(snapshot), after));
        assert(cache_call_result(now, slot, Some(snapshot)) == Some(snapshot));
    }
    Ok(snapshot)
}

/// The trace `t` built `x` and stored it under `s`, after observing `s`
/// at the start of the call and again immediately after the build.
pub open spec fn built_and_stored_under<T>(t: Seq<CacheEvent<T>>, s: IndexStamp, x: T) -> bool {
    &&& t.len() == 5
    &&& t[0] == CacheEvent::<T>::Observed(Some(s))
    &&& t[2] == CacheEvent::<T>::Built(Some(x))
    &&& t[3] == CacheEvent::<T>::Observed(Some(s))
    &&& t[4] == CacheEvent::<T>::Stored(s, x)
}

/// What one call does, for every combination of answers: the index is
/// stat-ed before anything else; a missing index is never looked up or
/// stored, and the snapshot is rebuilt; a snapshot is stored only under a
/// stamp observed both at the start of the call and immediately after that
/// very snapshot was built, and only as the call's last effect; a call that
/// builds nothing serves the slot's snapshot, which was stored under exactly
/// the stamp it observed first; and a call that builds returns its build.
pub proof fn theorem_cache_call_effects<T>(
    now: Option<IndexStamp>,
    hit: Option<(IndexStamp, T)>,
    built: Option<T>,
    after: Option<IndexStamp>,
)
    ensures
        ({
            let t = cache_call_trace(now, hit, built, after);
            let r = cache_call_result(now, hit, built);
            &&& t.len() > 0 && t[0] == CacheEvent::<T>::Observed(now)
            &&& now is None ==> t == seq![
                CacheEvent::<T>::Observed(None),
                CacheEvent::<T>::Built(built),
            ] && r == built
            &&& forall|k: int|
                #![trigger t[k]]
                0 <= k < t.len() && t[k] is Stored ==> k == t.len() - 1 && built_and_stored_under(
                    t,
                    t[k]->Stored_0,
                    t[k]->Stored_1,
                )
            &&& (forall|k: int| #![trigger t[k]] 0 <= k < t.len() ==> !(t[k] is Built)) ==> (
            now is Some && hit is Some && now == Some(hit->Some_0.0) && r == Some(hit->Some_0.1))
            &&& forall|k: int|
                #![trigger t[k]]
                0 <= k < t.len() && t[k] is Built ==> r == t[k]->Built_0
        }),
{
    let t = cache_call_trace(now, hit, built, after);
    if now is Some && !slot_serves(now, hit) {
        assert(t[2] is Built);
    }
}

/// The slot after a call whose effects were `t`: what it stored last, or
/// else what the slot held before.
pub open spec fn slot_after<T>(slot: Option<(IndexStamp, T)>, t: Seq<CacheEvent<T>>) -> Option<
    (IndexStamp, T),
> {
    if t.len() > 0 && t.last() is Stored {
        Some((t.last()->Stored_0, t.last()->Stored_1))
    } else {
        slot
    }
}

/// The slot before call `i` of a sequence of calls on one repository, where
/// call `j` observed `calls[j].0` first, built `calls[j].1`, and observed
/// `calls[j].2` after its build, and every read of the slot returns what was
/// last stored in it (the trusted map semantics). The slot starts empty.
pub open spec fn slot_before<T>(
    calls: Seq<(Option<IndexStamp>, Option<T>, Option<IndexStamp>)>,
    i: nat,
) -> Option<(IndexStamp, T)>
    decreases i,
{
    if i == 0 {
        None
    } else {
        let prev = slot_before(calls, (i - 1) as nat);
        let c = calls[i - 1];
        slot_after(prev, cache_call_trace(c.0, prev, c.1, c.2))
    }
}

/// The effects of call `i` of the sequence `calls`.
pub open spec fn call_trace<T>(
    calls: Seq<(Option<IndexStamp>, Option<T>, Option<IndexStamp>)>,
    i: nat,
) -> Seq<CacheEvent<T>> {
    cache_call_trace(calls[i as int].0, slot_before(calls, i), calls[i as int].1, calls[i as int].2)
}

/// Whatever the slot holds before call `i` was built and stored by an
/// earlier call that observed its stamp before and after building it.
proof fn lemma_slot_provenance<T>(
    calls: Seq<(Option<IndexStamp>, Option<T>, Option<IndexStamp>)>,
    i: nat,
)
    requires
        i <= calls.len(),
    ensures
        slot_before(calls, i) is Some ==> exists|j: int|
            0 <= j < i && built_and_stored_under(
                #[trigger] call_trace(calls, j as nat),
                slot_before(calls, i)->Some_0.0,
                slot_before(calls, i)->Some_0.1,
            ),
    decreases i,
{
    if i > 0 {
        let m = (i - 1) as nat;
        lemma_slot_provenance(calls, m);
        let prev = slot_before(calls, m);
        let t = call_trace(calls, m);
        let c = calls[m as int];
        theorem_cache_call_effects(c.0, prev, c.1, c.2);
        if t.len() > 0 && t.last() is Stored {
            let k = t.len() - 1;
            assert(t[k] == t.last());
            assert(built_and_stored_under(t, t.last()->Stored_0, t.last()->Stored_1));
            assert(slot_before(calls, i) == Some((t.last()->Stored_0, t.last()->Stored_1)));
        } else {
            assert(slot_before(calls, i) == prev);
            if prev is Some {
                let j = choose|j: int|
                    0 <= j < m && built_and_stored_under(
                        #[trigger] call_trace(calls, j as nat),
                        prev->Some_0.0,
                        prev->Some_0.1,
                    );
                assert(0 <= j < i);
            }
        }
    }
}

/// Over any sequence of calls on one repository, with the index changing
/// arbitrarily between and during them, a call that serves a cached
/// snapshot (builds nothing) serves one that an earlier call built while the
/// index carried exactly the stamp this call observed first: that earlier
/// call observed the stamp at its start and again right after the build.
/// A missing index is never served.
pub proof fn theorem_cache_serves_only_attributable_snapshots<T>(
    calls: Seq<(Option<IndexStamp>, Option<T>, Option<IndexStamp>)>,
    i: int,
)
    requires
        0 <= i < calls.len(),
    ensures
        ({
            let t = call_trace(calls, i as nat);
            let now = calls[i].0;
            let r = cache_call_result(now, slot_before(calls, i as nat), calls[i].1);
            (forall|k: int| #![trigger t[k]] 0 <= k < t.len() ==> !(t[k] is Built)) ==> {
                &&& now is Some
                &&& r is Some
                &&& exists|j: int|
                    0 <= j < i && built_and_stored_under(
                        #[trigger] call_trace(calls, j as nat),
                        now->Some_0,
                        r->Some_0,
                    )
            }
        }),
{
    let slot = slot_before(calls, i as nat);
    let t = call_trace(calls, i as nat);
    theorem_cache_call_effects(calls[i].0, slot, calls[i].1, calls[i].2);
    lemma_slot_provenance(calls, i as nat);
    let u = cache_call_trace(calls[i].0, slot, calls[i].1, calls[i].2);
    assert(t == u);
    if forall|k: int| #![trigger t[k]] 0 <= k < t.len() ==> !(t[k] is Built) {
        assert forall|k: int| #![trigger u[k]] 0 <= k < u.len() implies !(u[k] is Built) by {
            assert(t[k] == u[k]);
        }
    }
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

/// Decide `rule_file_counts_spec`.
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
