//! Verified kernel: tracked-path lookup with case folding.
//!
//! # What it decides
//!
//! Whether a repository path is tracked by the index, which is the decision
//! behind waft's central guarantee that it never selects or overwrites a
//! tracked file ([`TrackedLookup::contains`] and [`TrackedLookup::select`]);
//! whether two repository paths are equivalent under the checkout's case
//! policy ([`paths_equivalent`]); whether a non-exact spelling is a
//! filesystem alias of another path ([`alias_resolves_to`]); and the fold
//! keys of UTF-8 and non-UTF-8 paths ([`utf8_fold_key`],
//! [`non_utf8_fold_key`]).
//!
//! The lookup is a verified hash index ([`KeyTable`]) with one bucket per
//! key: bucket `b` holds exactly the ascending positions whose key hashes to
//! `b`, so a query scans one bucket and the answer never depends on the hash.
//! Only the bucket count is proved. How evenly keys spread is not: the hash
//! is seeded FNV-1a with a murmur finalizer, not a keyed PRF, so one bucket's
//! worth of work per query is the expected cost, not a guaranteed bound
//! against crafted index names.
//!
//! # Theorems
//!
//! - `TrackedLookup::contains` returns exactly `is_tracked`, the naive
//!   linear-scan specification: an exact index spelling is tracked;
//!   otherwise the query must fold onto an index entry, which suffices on a
//!   case-folding checkout and, on a case-sensitive one, also needs the
//!   filesystem to resolve the query and a colliding entry to the same
//!   object. Its `ensures` also pins the exact sequence of filesystem probes
//!   (`probe_trace`).
//! - `TrackedLookup::select` returns exactly the positions of the tracked
//!   queries, and its `ensures` pins its filesystem probes too
//!   (`select_trace`: each query's `probe_trace`, in query order, nothing
//!   else).
//! - `theorem_exact_spelling_always_tracked`: an exact spelling is tracked
//!   whatever the fold function, case policy or filesystem say, and is
//!   decided without probing the filesystem. A wrong Unicode table can never
//!   untrack an exact entry.
//! - `theorem_folding_checkout_protects_without_probing`: on a case-folding
//!   checkout a folded collision alone protects, and nothing is probed.
//! - `theorem_case_sensitive_requires_identity_evidence`: on a
//!   case-sensitive checkout a non-exact spelling is tracked only when it
//!   resolves to the same object as a colliding index entry.
//! - `theorem_protection_is_monotone`: folded-name protection only adds
//!   protection.
//! - `theorem_probes_stay_in_the_collision`: the only paths probed are the
//!   query (first) and index entries that fold onto it, in index order.
//! - `theorem_select_probes_stay_in_the_collisions`: a batch probes only its
//!   queries and index entries that fold onto one of them;
//!   `theorem_select_on_folding_checkout_never_probes`: on a case-folding
//!   checkout a batch probes nothing.
//! - `lemma_positions_of_exact` (bucket exactness): `KeyTable::positions`
//!   lists every position holding the key, ascending, and nothing else.
//! - `lemma_tracked_positions_exact`: `select` is exactly a filter of the
//!   queries by `is_tracked`, in the caller's order.
//! - `theorem_fold_key_kinds_never_collide`, `theorem_utf8_fold_key_is_injective`,
//!   `theorem_ascii_fold_shape`: UTF-8 and non-UTF-8 keys never collide, the
//!   tag adds no collisions, and the non-UTF-8 fold is length-preserving,
//!   idempotent, and touches only ASCII uppercase letters.
//! - `theorem_equivalence_relation`: path equivalence is reflexive,
//!   symmetric and transitive for each case policy, is byte equality on a
//!   case-sensitive checkout, and folding only merges more paths.
//! - `alias_resolves_to` never reports a path as an alias of itself, and
//!   probes the filesystem only when the probed path resolved and the
//!   spellings differ.
//!
//! # Trust assumptions
//!
//! - [`FoldOracle`]: production's Unicode fold (NFD, full case fold, NFD) is
//!   a deterministic function of the path bytes, applied identically to
//!   index entries and queries. Its Unicode correctness is not verified and
//!   not relied on for exact-spelling protection.
//! - [`IdentityOracle`]: production reports the filesystem identity
//!   (device/inode or volume/file index) of a repository path, or `None`, and
//!   the answers are stable during one call.
//! - The entries are the index's path spellings as Git records them.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

// ---------------------------------------------------------------------------
// Specification vocabulary
// ---------------------------------------------------------------------------
/// The byte strings held by a vector of byte vectors.
pub open spec fn views(v: Seq<Vec<u8>>) -> Seq<Seq<u8>> {
    v.map_values(|x: Vec<u8>| x@)
}

/// The positions `j < n` with `keys[j] == q`, ascending.
pub open spec fn positions_of(keys: Seq<Seq<u8>>, n: nat, q: Seq<u8>) -> Seq<usize>
    decreases n,
{
    if n == 0 {
        Seq::empty()
    } else {
        let rest = positions_of(keys, (n - 1) as nat, q);
        if keys[n - 1] == q {
            rest.push((n - 1) as usize)
        } else {
            rest
        }
    }
}

// ---------------------------------------------------------------------------
// Hashing (performance only: no theorem depends on the hash function)
// ---------------------------------------------------------------------------
pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

pub const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

pub const MIX: u64 = 0xff51_afd7_ed55_8ccd;

/// Seeded FNV-1a over a byte string.
pub open spec fn fnv_spec(seed: u64, s: Seq<u8>) -> u64
    decreases s.len(),
{
    if s.len() == 0 {
        FNV_OFFSET ^ seed
    } else {
        (fnv_spec(seed, s.drop_last()) ^ (s.last() as u64)).wrapping_mul(FNV_PRIME)
    }
}

/// A murmur-style finalizer so every bit of the FNV state reaches the bucket.
pub open spec fn mix_spec(h: u64) -> u64 {
    let a = h ^ (h >> 33u64);
    let b = a.wrapping_mul(MIX);
    b ^ (b >> 33u64)
}

/// The bucket a key lands in for a table of `m` buckets.
pub open spec fn bucket_spec(seed: u64, s: Seq<u8>, m: nat) -> int {
    (mix_spec(fnv_spec(seed, s)) as int) % (m as int)
}

/// The positions `j < n` whose key lands in bucket `b`, ascending.
pub open spec fn positions_in_bucket(
    keys: Seq<Seq<u8>>,
    n: nat,
    seed: u64,
    m: nat,
    b: int,
) -> Seq<usize>
    decreases n,
{
    if n == 0 {
        Seq::empty()
    } else {
        let rest = positions_in_bucket(keys, (n - 1) as nat, seed, m, b);
        if bucket_spec(seed, keys[n - 1], m) == b {
            rest.push((n - 1) as usize)
        } else {
            rest
        }
    }
}

/// The elements `p` of `ps` with `keys[p] == q`, in order.
pub open spec fn keep_matching(keys: Seq<Seq<u8>>, ps: Seq<usize>, q: Seq<u8>) -> Seq<usize>
    decreases ps.len(),
{
    if ps.len() == 0 {
        Seq::empty()
    } else {
        let rest = keep_matching(keys, ps.drop_last(), q);
        if keys[ps.last() as int] == q {
            rest.push(ps.last())
        } else {
            rest
        }
    }
}

fn fnv(seed: u64, s: &[u8]) -> (h: u64)
    ensures
        h == fnv_spec(seed, s@),
{
    let mut h: u64 = FNV_OFFSET ^ seed;
    let mut i: usize = 0;
    while i < s.len()
        invariant
            i <= s@.len(),
            h == fnv_spec(seed, s@.subrange(0, i as int)),
        decreases s.len() - i,
    {
        assert(s@.subrange(0, i + 1).drop_last() == s@.subrange(0, i as int));
        h = (h ^ (s[i] as u64)).wrapping_mul(FNV_PRIME);
        i += 1;
    }
    assert(s@.subrange(0, s@.len() as int) == s@);
    h
}

fn bucket_index(seed: u64, s: &[u8], m: usize) -> (b: usize)
    requires
        m > 0,
    ensures
        b < m,
        b as int == bucket_spec(seed, s@, m as nat),
{
    let h = fnv(seed, s);
    let a = h ^ (h >> 33u64);
    let c = a.wrapping_mul(MIX);
    let d = c ^ (c >> 33u64);
    (d % (m as u64)) as usize
}

proof fn lemma_positions_in_bucket_bounds(keys: Seq<Seq<u8>>, n: nat, seed: u64, m: nat, b: int)
    requires
        n <= keys.len(),
        keys.len() <= usize::MAX,
    ensures
        forall|k: int|
            0 <= k < positions_in_bucket(keys, n, seed, m, b).len() ==> (#[trigger] positions_in_bucket(
                keys,
                n,
                seed,
                m,
                b,
            )[k] as int) < n,
    decreases n,
{
    if n > 0 {
        lemma_positions_in_bucket_bounds(keys, (n - 1) as nat, seed, m, b);
        let rest = positions_in_bucket(keys, (n - 1) as nat, seed, m, b);
        let ps = positions_in_bucket(keys, n, seed, m, b);
        assert forall|k: int| 0 <= k < ps.len() implies (#[trigger] ps[k] as int) < n by {
            if k < rest.len() {
                assert(ps[k] == rest[k]);
            } else {
                assert(ps[k] == (n - 1) as usize);
            }
        }
    }
}

/// Filtering bucket `b` by key equality recovers every position of `q`.
proof fn lemma_bucket_filter(keys: Seq<Seq<u8>>, n: nat, seed: u64, m: nat, q: Seq<u8>)
    requires
        n <= keys.len(),
        keys.len() <= usize::MAX,
    ensures
        keep_matching(keys, positions_in_bucket(keys, n, seed, m, bucket_spec(seed, q, m)), q)
            == positions_of(keys, n, q),
    decreases n,
{
    let b = bucket_spec(seed, q, m);
    if n > 0 {
        lemma_bucket_filter(keys, (n - 1) as nat, seed, m, q);
        let rest = positions_in_bucket(keys, (n - 1) as nat, seed, m, b);
        if bucket_spec(seed, keys[n - 1], m) == b {
            let ps = rest.push((n - 1) as usize);
            assert(ps.drop_last() == rest);
            assert(ps.last() == (n - 1) as usize);
            assert(((n - 1) as usize) as int == n - 1);
        }
    }
}

/// Hash index from byte keys to the ascending list of positions holding them.
///
/// Bucket `b` holds exactly the positions whose key hashes to `b`, in
/// ascending order, so filtering one bucket by key equality yields every
/// position of a key and nothing else, independent of the hash function.
pub struct KeyTable {
    seed: u64,
    buckets: Vec<Vec<usize>>,
}

impl KeyTable {
    /// `self` indexes exactly the keys `keys`.
    pub closed spec fn indexes(&self, keys: Seq<Seq<u8>>) -> bool {
        &&& self.buckets@.len() > 0
        &&& forall|b: int|
            0 <= b < self.buckets@.len() ==> (#[trigger] self.buckets@[b])@ == positions_in_bucket(
                keys,
                keys.len(),
                self.seed,
                self.buckets@.len() as nat,
                b,
            )
    }

    /// The number of buckets.
    pub closed spec fn bucket_count(&self) -> nat {
        self.buckets@.len() as nat
    }

    /// Index `keys` in one bucket per key (one bucket for no keys); `seed`
    /// only affects how keys spread across buckets.
    pub fn build(seed: u64, keys: &[Vec<u8>]) -> (t: Self)
        ensures
            t.indexes(views(keys@)),
            t.bucket_count() == if keys@.len() == 0 {
                1
            } else {
                keys@.len()
            },
    {
        let ghost ks = views(keys@);
        let n = keys.len();
        let m: usize = if n == 0 {
            1
        } else {
            n
        };
        let mut buckets: Vec<Vec<usize>> = Vec::new();
        let mut c: usize = 0;
        while c < m
            invariant
                c <= m,
                buckets@.len() == c,
                forall|d: int| 0 <= d < c ==> (#[trigger] buckets@[d])@ == Seq::<usize>::empty(),
            decreases m - c,
        {
            buckets.push(Vec::new());
            c += 1;
        }
        let mut i: usize = 0;
        while i < n
            invariant
                n == keys@.len(),
                ks == views(keys@),
                m > 0,
                i <= n,
                buckets@.len() == m,
                forall|d: int|
                    0 <= d < m ==> (#[trigger] buckets@[d])@ == positions_in_bucket(
                        ks,
                        i as nat,
                        seed,
                        m as nat,
                        d,
                    ),
            decreases n - i,
        {
            let b = bucket_index(seed, keys[i].as_slice(), m);
            assert(ks[i as int] == keys@[i as int]@);
            let ghost before = buckets@;
            buckets[b].push(i);
            assert forall|d: int| 0 <= d < m implies (#[trigger] buckets@[d])@ == positions_in_bucket(
                ks,
                (i + 1) as nat,
                seed,
                m as nat,
                d,
            ) by {
                assert(((i + 1 - 1) as nat) == i as nat);
                if d != b {
                    assert(buckets@[d] == before[d]);
                }
            }
            i += 1;
        }
        KeyTable { seed, buckets }
    }

    /// Whether some position holds `q`.
    ///
    /// Crate-private: its `requires` ties the table to the very keys it was
    /// built from, which unverified callers could not be held to.
    pub(crate) fn contains_key(&self, keys: &[Vec<u8>], q: &[u8]) -> (r: bool)
        requires
            self.indexes(views(keys@)),
        ensures
            r == exists|i: int| 0 <= i < keys@.len() && #[trigger] views(keys@)[i] == q@,
    {
        let ghost ks = views(keys@);
        let _n = keys.len();
        let m = self.buckets.len();
        let b = bucket_index(self.seed, q, m);
        let bucket = &self.buckets[b];
        let ghost ps = positions_in_bucket(ks, ks.len(), self.seed, m as nat, b as int);
        proof {
            lemma_positions_in_bucket_bounds(ks, ks.len(), self.seed, m as nat, b as int);
            lemma_bucket_filter(ks, ks.len(), self.seed, m as nat, q@);
        }
        let mut k: usize = 0;
        while k < bucket.len()
            invariant
                bucket@ == ps,
                ks == views(keys@),
                k <= ps.len(),
                forall|j: int| 0 <= j < ps.len() ==> (#[trigger] ps[j] as int) < keys@.len(),
                keep_matching(ks, ps.subrange(0, k as int), q@) == Seq::<usize>::empty(),
            decreases bucket.len() - k,
        {
            let p = bucket[k];
            assert(ps.subrange(0, k + 1).drop_last() == ps.subrange(0, k as int));
            let found = keys[p].as_slice() == q;
            if found {
                assert(keys@[p as int]@ =~= q@);
                assert(ks[p as int] == q@);
                return true;
            }
            k += 1;
        }
        assert(ps.subrange(0, ps.len() as int) == ps);
        proof {
            lemma_positions_of_empty(ks, ks.len(), q@);
        }
        false
    }

    /// Every position holding `q`, ascending.
    ///
    /// Crate-private for the same reason as `KeyTable::contains_key`.
    pub(crate) fn positions(&self, keys: &[Vec<u8>], q: &[u8]) -> (r: Vec<usize>)
        requires
            self.indexes(views(keys@)),
        ensures
            r@ == positions_of(views(keys@), keys@.len() as nat, q@),
    {
        let ghost ks = views(keys@);
        let _n = keys.len();
        let m = self.buckets.len();
        let b = bucket_index(self.seed, q, m);
        let bucket = &self.buckets[b];
        let ghost ps = positions_in_bucket(ks, ks.len(), self.seed, m as nat, b as int);
        proof {
            lemma_positions_in_bucket_bounds(ks, ks.len(), self.seed, m as nat, b as int);
            lemma_bucket_filter(ks, ks.len(), self.seed, m as nat, q@);
        }
        let mut out: Vec<usize> = Vec::new();
        let mut k: usize = 0;
        while k < bucket.len()
            invariant
                bucket@ == ps,
                ks == views(keys@),
                k <= ps.len(),
                forall|j: int| 0 <= j < ps.len() ==> (#[trigger] ps[j] as int) < keys@.len(),
                out@ == keep_matching(ks, ps.subrange(0, k as int), q@),
            decreases bucket.len() - k,
        {
            let p = bucket[k];
            assert(ps.subrange(0, k + 1).drop_last() == ps.subrange(0, k as int));
            assert(ks[p as int] == keys@[p as int]@);
            if keys[p].as_slice() == q {
                assert(keys@[p as int]@ =~= q@);
                out.push(p);
            } else {
                assert(keys@[p as int]@ != q@);
            }
            k += 1;
        }
        assert(ps.subrange(0, ps.len() as int) == ps);
        out
    }
}

/// `positions_of` is empty exactly when no position holds `q`.
proof fn lemma_positions_of_empty(keys: Seq<Seq<u8>>, n: nat, q: Seq<u8>)
    requires
        n <= keys.len(),
    ensures
        (positions_of(keys, n, q).len() == 0) == !(exists|i: int| 0 <= i < n && keys[i] == q),
    decreases n,
{
    if n > 0 {
        lemma_positions_of_empty(keys, (n - 1) as nat, q);
        if keys[n - 1] != q {
            if exists|i: int| 0 <= i < n && keys[i] == q {
                let i = choose|i: int| 0 <= i < n && keys[i] == q;
                assert(i < n - 1);
            }
        }
    }
}

/// Every element of `positions_of(keys, n, q)` is a position below `n` holding
/// `q`, the list is strictly ascending, and every such position is listed.
pub proof fn lemma_positions_of_exact(keys: Seq<Seq<u8>>, n: nat, q: Seq<u8>)
    requires
        n <= keys.len(),
        keys.len() <= usize::MAX,
    ensures
        forall|j: int|
            0 <= j < positions_of(keys, n, q).len() ==> {
                let p = #[trigger] positions_of(keys, n, q)[j];
                &&& (p as int) < n
                &&& keys[p as int] == q
            },
        forall|a: int, b: int|
            0 <= a < b < positions_of(keys, n, q).len() ==> (positions_of(keys, n, q)[a] as int)
                < (positions_of(keys, n, q)[b] as int),
        forall|i: int|
            0 <= i < n && keys[i] == q ==> exists|j: int|
                0 <= j < positions_of(keys, n, q).len() && (#[trigger] positions_of(keys, n, q)[j])
                    as int == i,
    decreases n,
{
    if n > 0 {
        lemma_positions_of_exact(keys, (n - 1) as nat, q);
        let rest = positions_of(keys, (n - 1) as nat, q);
        let ps = positions_of(keys, n, q);
        if keys[n - 1] == q {
            assert(ps == rest.push((n - 1) as usize));
            assert forall|i: int| 0 <= i < n && keys[i] == q implies exists|j: int|
                0 <= j < ps.len() && (#[trigger] ps[j]) as int == i by {
                if i == n - 1 {
                    assert(ps[rest.len() as int] as int == i);
                } else {
                    let j = choose|j: int| 0 <= j < rest.len() && (#[trigger] rest[j]) as int == i;
                    assert(ps[j] == rest[j]);
                }
            }
        } else {
            assert forall|i: int| 0 <= i < n && keys[i] == q implies exists|j: int|
                0 <= j < ps.len() && (#[trigger] ps[j]) as int == i by {
                assert(i < n - 1);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Trust boundary: oracles implemented by production
// ---------------------------------------------------------------------------
/// The case-folding key function, implemented by production.
///
/// Trusted: `fold_key` is a deterministic function of the path bytes alone
/// (the receiver-free [`FoldOracle::fold`] makes that structural: one type,
/// one function), so index entries and queries are folded identically.
/// Nothing here trusts the fold to be *correct* Unicode case folding: exact
/// index spellings are protected without consulting it.
pub trait FoldOracle {
    /// The fold key of `path`.
    spec fn fold(path: Seq<u8>) -> Seq<u8>;

    /// Compute the fold key of `path`.
    fn fold_key(&self, path: &[u8]) -> (key: Vec<u8>)
        ensures
            key@ == Self::fold(path@),
    ;
}

/// Opaque per-object filesystem identity (device and inode, or the Windows
/// volume serial number and file index).
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub struct FileId {
    /// Device (Unix) or volume serial number (Windows).
    pub dev: u64,
    /// Inode (Unix) or file index (Windows).
    pub ino: u64,
}

/// Filesystem identity probes, implemented by production.
///
/// Trusted: `query_identity` reports the identity the filesystem gives the
/// repository path (or `None` when it does not resolve), and the answers are
/// stable for the duration of one kernel call. `probes` is a ghost log of the
/// paths asked about, which lets the kernel state exactly which filesystem
/// queries it issues and in what order.
pub trait IdentityOracle {
    /// The identity the filesystem reports for `path`.
    spec fn identity(&self, path: Seq<u8>) -> Option<FileId>;

    /// Every path probed so far, in order.
    spec fn probes(&self) -> Seq<Seq<u8>>;

    /// Probe the filesystem identity of `path`.
    fn query_identity(&mut self, path: &[u8]) -> (id: Option<FileId>)
        ensures
            id == old(self).identity(path@),
            final(self).probes() == old(self).probes().push(path@),
            forall|p: Seq<u8>| #[trigger] final(self).identity(p) == old(self).identity(p),
    ;
}

/// Whether `a` names a resolved object and `b` names the same one.
pub open spec fn same_object(a: Option<FileId>, b: Option<FileId>) -> bool {
    a is Some && a == b
}

fn same_identity(a: Option<FileId>, b: Option<FileId>) -> (r: bool)
    ensures
        r == same_object(a, b),
{
    match (a, b) {
        (Some(x), Some(y)) => x == y,
        (Some(_), None) => false,
        (None, _) => false,
    }
}

// ---------------------------------------------------------------------------
// The tracked-path decision, declaratively
// ---------------------------------------------------------------------------
/// Fold keys of every entry.
pub open spec fn fold_keys<F: FoldOracle>(entries: Seq<Seq<u8>>) -> Seq<Seq<u8>> {
    entries.map_values(|e: Seq<u8>| F::fold(e))
}

/// Some index entry is byte-for-byte the query.
pub open spec fn exact_match(entries: Seq<Seq<u8>>, q: Seq<u8>) -> bool {
    exists|i: int| 0 <= i < entries.len() && entries[i] == q
}

/// Some index entry folds onto the query.
pub open spec fn fold_collision<F: FoldOracle>(entries: Seq<Seq<u8>>, q: Seq<u8>) -> bool {
    exists|i: int| 0 <= i < entries.len() && F::fold(#[trigger] entries[i]) == F::fold(q)
}

/// The query resolves, and some index entry that folds onto it resolves to
/// the same filesystem object.
pub open spec fn identity_collision<F: FoldOracle, O: IdentityOracle>(
    entries: Seq<Seq<u8>>,
    q: Seq<u8>,
    o: O,
) -> bool {
    exists|i: int|
        0 <= i < entries.len() && F::fold(#[trigger] entries[i]) == F::fold(q) && same_object(
            o.identity(q),
            o.identity(entries[i]),
        )
}

/// The naive linear-scan meaning of "the index tracks `q`".
///
/// An exact spelling is always tracked. Otherwise the query must fold onto
/// an index entry, and that collision alone suffices on a case-folding
/// checkout (`protect`); on a case-sensitive checkout the filesystem must
/// also resolve the query and a colliding entry to the same object.
pub open spec fn is_tracked<F: FoldOracle, O: IdentityOracle>(
    entries: Seq<Seq<u8>>,
    protect: bool,
    q: Seq<u8>,
    o: O,
) -> bool {
    exact_match(entries, q) || (fold_collision::<F>(entries, q) && (protect
        || identity_collision::<F, O>(entries, q, o)))
}

/// The index entries that fold onto `q`, in index order.
pub open spec fn colliding_entries<F: FoldOracle>(entries: Seq<Seq<u8>>, q: Seq<u8>) -> Seq<
    Seq<u8>,
> {
    positions_of(fold_keys::<F>(entries), entries.len(), F::fold(q)).map_values(
        |p: usize| entries[p as int],
    )
}

/// Probe `paths` in order, stopping after the first one that resolves to `id`.
pub open spec fn probe_until_match<O: IdentityOracle>(
    o: O,
    paths: Seq<Seq<u8>>,
    id: Option<FileId>,
) -> Seq<Seq<u8>>
    decreases paths.len(),
{
    if paths.len() == 0 {
        Seq::empty()
    } else if same_object(id, o.identity(paths[0])) {
        seq![paths[0]]
    } else {
        seq![paths[0]] + probe_until_match(o, paths.drop_first(), id)
    }
}

/// The filesystem probes a tracked-path query issues, in order: none unless
/// the query has no exact spelling, folds onto an index entry, and the
/// checkout is case-sensitive; then the query itself, and (if it resolves)
/// the colliding entries in index order up to the first match.
pub open spec fn probe_trace<F: FoldOracle, O: IdentityOracle>(
    entries: Seq<Seq<u8>>,
    protect: bool,
    q: Seq<u8>,
    o: O,
) -> Seq<Seq<u8>> {
    if exact_match(entries, q) || !fold_collision::<F>(entries, q) || protect {
        Seq::empty()
    } else if o.identity(q) is None {
        seq![q]
    } else {
        seq![q] + probe_until_match(o, colliding_entries::<F>(entries, q), o.identity(q))
    }
}

/// The positions `i < n` whose query is tracked, ascending.
pub open spec fn tracked_positions<F: FoldOracle, O: IdentityOracle>(
    entries: Seq<Seq<u8>>,
    protect: bool,
    queries: Seq<Seq<u8>>,
    n: nat,
    o: O,
) -> Seq<usize>
    decreases n,
{
    if n == 0 {
        Seq::empty()
    } else {
        let rest = tracked_positions::<F, O>(entries, protect, queries, (n - 1) as nat, o);
        if is_tracked::<F, O>(entries, protect, queries[n - 1], o) {
            rest.push((n - 1) as usize)
        } else {
            rest
        }
    }
}

/// The filesystem probes a batch of the first `n` queries issues, in order:
/// each query's `probe_trace`, in query order.
pub open spec fn select_trace<F: FoldOracle, O: IdentityOracle>(
    entries: Seq<Seq<u8>>,
    protect: bool,
    queries: Seq<Seq<u8>>,
    n: nat,
    o: O,
) -> Seq<Seq<u8>>
    decreases n,
{
    if n == 0 {
        Seq::empty()
    } else {
        select_trace::<F, O>(entries, protect, queries, (n - 1) as nat, o) + probe_trace::<F, O>(
            entries,
            protect,
            queries[n - 1],
            o,
        )
    }
}

/// The byte strings of a slice of byte slices.
pub open spec fn query_views(qs: Seq<&[u8]>) -> Seq<Seq<u8>> {
    qs.map_values(|q: &[u8]| q@)
}

// ---------------------------------------------------------------------------
// Theorems about the specification
// ---------------------------------------------------------------------------
/// An exact index spelling is tracked whatever the fold function, the
/// checkout's case policy, or the filesystem say, and is decided without a
/// single filesystem probe.
pub proof fn theorem_exact_spelling_always_tracked<F: FoldOracle, O: IdentityOracle>(
    entries: Seq<Seq<u8>>,
    protect: bool,
    q: Seq<u8>,
    o: O,
)
    requires
        exact_match(entries, q),
    ensures
        is_tracked::<F, O>(entries, protect, q, o),
        probe_trace::<F, O>(entries, protect, q, o) == Seq::<Seq<u8>>::empty(),
{
}

/// On a case-folding checkout a folded collision alone protects the query,
/// and the filesystem is never consulted.
pub proof fn theorem_folding_checkout_protects_without_probing<
    F: FoldOracle,
    O: IdentityOracle,
>(entries: Seq<Seq<u8>>, q: Seq<u8>, o: O)
    ensures
        is_tracked::<F, O>(entries, true, q, o) == (exact_match(entries, q) || fold_collision::<F>(
            entries,
            q,
        )),
        probe_trace::<F, O>(entries, true, q, o) == Seq::<Seq<u8>>::empty(),
{
}

/// On a case-sensitive checkout a non-exact spelling is tracked only with
/// filesystem evidence: the query resolves, and an index entry that folds
/// onto it resolves to the very same object. Distinct differently-cased files
/// are therefore never protected by their name alone.
pub proof fn theorem_case_sensitive_requires_identity_evidence<
    F: FoldOracle,
    O: IdentityOracle,
>(entries: Seq<Seq<u8>>, q: Seq<u8>, o: O)
    requires
        is_tracked::<F, O>(entries, false, q, o),
        !exact_match(entries, q),
    ensures
        o.identity(q) is Some,
        exists|i: int|
            0 <= i < entries.len() && F::fold(#[trigger] entries[i]) == F::fold(q) && o.identity(
                entries[i],
            ) == o.identity(q),
{
}

/// Folded-name protection only ever adds protection.
pub proof fn theorem_protection_is_monotone<F: FoldOracle, O: IdentityOracle>(
    entries: Seq<Seq<u8>>,
    q: Seq<u8>,
    o: O,
)
    ensures
        is_tracked::<F, O>(entries, false, q, o) ==> is_tracked::<F, O>(entries, true, q, o),
{
}

/// Every filesystem probe a query issues is the query itself or an index
/// entry that folds onto it; the query is probed first.
pub proof fn theorem_probes_stay_in_the_collision<F: FoldOracle, O: IdentityOracle>(
    entries: Seq<Seq<u8>>,
    protect: bool,
    q: Seq<u8>,
    o: O,
)
    requires
        entries.len() <= usize::MAX,
    ensures
        probe_trace::<F, O>(entries, protect, q, o).len() > 0 ==> probe_trace::<F, O>(
            entries,
            protect,
            q,
            o,
        )[0] == q,
        forall|k: int|
            #![trigger probe_trace::<F, O>(entries, protect, q, o)[k]]
            0 < k < probe_trace::<F, O>(entries, protect, q, o).len() ==> exists|i: int|
                0 <= i < entries.len() && #[trigger] entries[i] == probe_trace::<F, O>(
                    entries,
                    protect,
                    q,
                    o,
                )[k] && F::fold(entries[i]) == F::fold(q),
{
    let t = probe_trace::<F, O>(entries, protect, q, o);
    if !(exact_match(entries, q) || !fold_collision::<F>(entries, q) || protect) && o.identity(
        q,
    ) is Some {
        let keys = fold_keys::<F>(entries);
        let ps = positions_of(keys, entries.len(), F::fold(q));
        let cs = colliding_entries::<F>(entries, q);
        lemma_positions_of_exact(keys, entries.len(), F::fold(q));
        lemma_probe_until_match_members(o, cs, o.identity(q));
        assert forall|k: int| #![trigger t[k]] 0 < k < t.len() implies exists|i: int|
            0 <= i < entries.len() && #[trigger] entries[i] == t[k] && F::fold(entries[i])
                == F::fold(q) by {
            let pm = probe_until_match(o, cs, o.identity(q));
            assert(t[k] == pm[k - 1]);
            let j = choose|j: int| 0 <= j < cs.len() && cs[j] == pm[k - 1];
            let p = ps[j];
            assert(cs[j] == entries[p as int]);
            assert(keys[p as int] == F::fold(entries[p as int]));
        }
    }
}

proof fn lemma_probe_until_match_members<O: IdentityOracle>(
    o: O,
    paths: Seq<Seq<u8>>,
    id: Option<FileId>,
)
    ensures
        probe_until_match(o, paths, id).len() <= paths.len(),
        forall|k: int|
            0 <= k < probe_until_match(o, paths, id).len() ==> exists|j: int|
                0 <= j < paths.len() && paths[j] == #[trigger] probe_until_match(o, paths, id)[k],
    decreases paths.len(),
{
    if paths.len() > 0 && !same_object(id, o.identity(paths[0])) {
        let rest = paths.drop_first();
        lemma_probe_until_match_members(o, rest, id);
        let pm = probe_until_match(o, paths, id);
        assert forall|k: int| 0 <= k < pm.len() implies exists|j: int|
            0 <= j < paths.len() && paths[j] == #[trigger] pm[k] by {
            if k == 0 {
                assert(paths[0] == pm[0]);
            } else {
                let tail = probe_until_match(o, rest, id);
                assert(pm[k] == tail[k - 1]);
                let j = choose|j: int| 0 <= j < rest.len() && rest[j] == tail[k - 1];
                assert(paths[j + 1] == rest[j]);
            }
        }
    } else if paths.len() > 0 {
        let pm = probe_until_match(o, paths, id);
        assert(pm[0] == paths[0]);
    }
}

// ---------------------------------------------------------------------------
// The executable lookup
// ---------------------------------------------------------------------------
/// Precomputed tracked-name lookup over one index's entries.
///
/// Per query: one hash probe for the exact spelling, one fold and hash probe
/// for the folded bucket, and filesystem probes only for index entries that
/// fold onto the query. With one bucket per key, a bucket's worth of work per
/// query is the expected cost; it is not a proved bound, because the hash
/// does not stop crafted names from sharing a bucket.
pub struct TrackedLookup<F: FoldOracle> {
    fold: F,
    entries: Vec<Vec<u8>>,
    keys: Vec<Vec<u8>>,
    exact: KeyTable,
    folded: KeyTable,
    protect: bool,
}

impl<F: FoldOracle> TrackedLookup<F> {
    #[verifier::type_invariant]
    closed spec fn inv(self) -> bool {
        &&& self.keys@.len() == self.entries@.len()
        &&& views(self.keys@) == fold_keys::<F>(views(self.entries@))
        &&& self.exact.indexes(views(self.entries@))
        &&& self.folded.indexes(views(self.keys@))
    }

    /// The index entries, in index order.
    pub closed spec fn entries_view(&self) -> Seq<Seq<u8>> {
        views(self.entries@)
    }

    /// Whether a folded collision alone proves trackedness.
    pub closed spec fn protects_folded_names(&self) -> bool {
        self.protect
    }

    /// Build the lookup for `entries` (index order, duplicates allowed).
    ///
    /// `protect_folded_names` is whether the checkout folds case. `seed`
    /// only spreads keys across hash buckets; no result depends on it.
    pub fn new(fold: F, entries: Vec<Vec<u8>>, protect_folded_names: bool, seed: u64) -> (r: Self)
        ensures
            r.entries_view() == views(entries@),
            r.protects_folded_names() == protect_folded_names,
    {
        let mut keys: Vec<Vec<u8>> = Vec::new();
        let mut i: usize = 0;
        while i < entries.len()
            invariant
                i <= entries@.len(),
                keys@.len() == i,
                forall|j: int| 0 <= j < i ==> (#[trigger] keys@[j])@ == F::fold(entries@[j]@),
            decreases entries.len() - i,
        {
            let key = fold.fold_key(entries[i].as_slice());
            keys.push(key);
            i += 1;
        }
        assert(views(keys@) =~= fold_keys::<F>(views(entries@)));
        let exact = KeyTable::build(seed, entries.as_slice());
        let folded = KeyTable::build(seed, keys.as_slice());
        TrackedLookup { fold, entries, keys, exact, folded, protect: protect_folded_names }
    }

    /// Whether the index tracks `query`, probing the filesystem exactly as
    /// `probe_trace` describes.
    pub fn contains<O: IdentityOracle>(&self, oracle: &mut O, query: &[u8]) -> (r: bool)
        ensures
            r == is_tracked::<F, O>(
                self.entries_view(),
                self.protects_folded_names(),
                query@,
                *old(oracle),
            ),
            final(oracle).probes() == old(oracle).probes() + probe_trace::<F, O>(
                self.entries_view(),
                self.protects_folded_names(),
                query@,
                *old(oracle),
            ),
            forall|p: Seq<u8>| #[trigger] final(oracle).identity(p) == old(oracle).identity(p),
    {
        proof {
            use_type_invariant(self);
        }
        let ghost es = views(self.entries@);
        let ghost ks = views(self.keys@);
        let ghost o0 = *oracle;
        let ghost base = oracle.probes();
        let _n = self.entries.len();
        if self.exact.contains_key(self.entries.as_slice(), query) {
            return true;
        }
        let qk = self.fold.fold_key(query);
        let colliding = self.folded.positions(self.keys.as_slice(), qk.as_slice());
        let ghost qf = F::fold(query@);
        proof {
            lemma_positions_of_exact(ks, ks.len(), qf);
            lemma_positions_of_empty(ks, ks.len(), qf);
            assert forall|i: int| 0 <= i < es.len() implies #[trigger] ks[i] == F::fold(es[i]) by {
                assert(ks[i] == fold_keys::<F>(es)[i]);
            }
            assert(!exact_match(es, query@));
            if fold_collision::<F>(es, query@) {
                let i = choose|i: int| 0 <= i < es.len() && F::fold(#[trigger] es[i]) == qf;
                assert(ks[i] == qf);
                assert(colliding@.len() > 0);
            }
            if colliding@.len() > 0 {
                let p = colliding@[0];
                assert(ks[p as int] == qf);
                assert(fold_collision::<F>(es, query@));
            }
        }
        if colliding.is_empty() {
            return false;
        }
        if self.protect {
            return true;
        }
        let query_id = oracle.query_identity(query);
        if query_id.is_none() {
            return false;
        }
        let ghost cs = colliding_entries::<F>(es, query@);
        assert(cs.len() == colliding@.len());
        assert(cs.subrange(0, cs.len() as int) == cs);
        assert(cs.subrange(0, 0) + cs == cs);
        let mut k: usize = 0;
        while k < colliding.len()
            invariant
                k <= colliding@.len(),
                cs.len() == colliding@.len(),
                cs == colliding_entries::<F>(es, query@),
                colliding@ == positions_of(ks, ks.len(), qf),
                qf == F::fold(query@),
                es == views(self.entries@),
                ks == fold_keys::<F>(es),
                es.len() == ks.len(),
                !exact_match(es, query@),
                fold_collision::<F>(es, query@),
                !self.protects_folded_names(),
                forall|i: int| 0 <= i < es.len() ==> #[trigger] ks[i] == F::fold(es[i]),
                forall|j: int|
                    0 <= j < colliding@.len() ==> ((#[trigger] colliding@[j]) as int) < es.len()
                        && ks[colliding@[j] as int] == qf,
                forall|j: int| 0 <= j < colliding@.len() ==> #[trigger] cs[j] == es[colliding@[j] as int],
                o0 == *old(oracle),
                base == old(oracle).probes(),
                es == self.entries_view(),
                query_id == o0.identity(query@),
                query_id is Some,
                forall|p: Seq<u8>| #[trigger] oracle.identity(p) == o0.identity(p),
                oracle.probes() == base.push(query@) + cs.subrange(0, k as int),
                probe_until_match(o0, cs, query_id) == cs.subrange(0, k as int) + probe_until_match(
                    o0,
                    cs.subrange(k as int, cs.len() as int),
                    query_id,
                ),
                forall|j: int| 0 <= j < k ==> !same_object(query_id, #[trigger] o0.identity(cs[j])),
            decreases colliding.len() - k,
        {
            let p = colliding[k];
            let ghost probes_before = oracle.probes();
            let id = oracle.query_identity(self.entries[p].as_slice());
            let ghost rest = cs.subrange(k as int, cs.len() as int);
            assert(rest[0] == cs[k as int]);
            assert(rest.drop_first() == cs.subrange(k + 1, cs.len() as int));
            assert(cs.subrange(0, k + 1) == cs.subrange(0, k as int).push(cs[k as int]));
            assert(cs[k as int] == es[p as int]);
            assert(self.entries@[p as int]@ == es[p as int]);
            assert(oracle.probes() == probes_before.push(es[p as int]));
            assert(oracle.probes() =~= base.push(query@) + cs.subrange(0, k + 1));
            if same_identity(query_id, id) {
                proof {
                    assert(ks[p as int] == qf);
                    assert(F::fold(es[p as int]) == qf);
                    assert(same_object(o0.identity(query@), o0.identity(es[p as int])));
                    assert(identity_collision::<F, O>(es, query@, o0));
                    assert(is_tracked::<F, O>(es, false, query@, o0));
                    assert(probe_until_match(o0, rest, query_id) == seq![cs[k as int]]);
                    assert(probe_until_match(o0, cs, query_id) =~= cs.subrange(0, k + 1));
                    assert(probe_trace::<F, O>(es, false, query@, o0) =~= seq![query@]
                        + cs.subrange(0, k + 1));
                    assert(oracle.probes() =~= base + probe_trace::<F, O>(es, false, query@, o0));
                }
                return true;
            }
            proof {
                assert(probe_until_match(o0, rest, query_id) == seq![cs[k as int]]
                    + probe_until_match(o0, cs.subrange(k + 1, cs.len() as int), query_id));
                assert(probe_until_match(o0, cs, query_id) =~= cs.subrange(0, k + 1)
                    + probe_until_match(o0, cs.subrange(k + 1, cs.len() as int), query_id));
            }
            k += 1;
        }
        proof {
            assert(cs.subrange(k as int, cs.len() as int) == Seq::<Seq<u8>>::empty());
            assert(cs.subrange(0, k as int) == cs);
            if identity_collision::<F, O>(es, query@, o0) {
                let i = choose|i: int|
                    0 <= i < es.len() && F::fold(#[trigger] es[i]) == F::fold(query@) && same_object(
                        o0.identity(query@),
                        o0.identity(es[i]),
                    );
                assert(ks[i] == F::fold(query@));
                let j = choose|j: int|
                    0 <= j < colliding@.len() && (#[trigger] colliding@[j]) as int == i;
                assert(cs[j] == es[i]);
            }
        }
        false
    }

    /// The positions of the tracked queries, ascending, probing the
    /// filesystem exactly as `select_trace` describes: each query's
    /// `probe_trace`, in query order, and nothing else.
    pub fn select<O: IdentityOracle>(&self, oracle: &mut O, queries: &[&[u8]]) -> (r: Vec<usize>)
        ensures
            r@ == tracked_positions::<F, O>(
                self.entries_view(),
                self.protects_folded_names(),
                query_views(queries@),
                queries@.len() as nat,
                *old(oracle),
            ),
            final(oracle).probes() == old(oracle).probes() + select_trace::<F, O>(
                self.entries_view(),
                self.protects_folded_names(),
                query_views(queries@),
                queries@.len() as nat,
                *old(oracle),
            ),
            forall|p: Seq<u8>| #[trigger] final(oracle).identity(p) == old(oracle).identity(p),
            forall|j: int| 0 <= j < r@.len() ==> (#[trigger] r@[j] as int) < queries@.len(),
            forall|a: int, b: int| 0 <= a < b < r@.len() ==> r@[a] < r@[b],
            forall|i: int|
                0 <= i < queries@.len() ==> (is_tracked::<F, O>(
                    self.entries_view(),
                    self.protects_folded_names(),
                    #[trigger] queries@[i]@,
                    *old(oracle),
                ) <==> exists|j: int| 0 <= j < r@.len() && r@[j] as int == i),
    {
        let ghost o0 = *oracle;
        let ghost base = oracle.probes();
        let ghost qs = query_views(queries@);
        let _n = queries.len();
        proof {
            lemma_tracked_positions_exact::<F, O>(
                self.entries_view(),
                self.protects_folded_names(),
                qs,
                qs.len(),
                o0,
            );
            assert forall|i: int| 0 <= i < queries@.len() implies #[trigger] queries@[i]@ == qs[i] by {}
        }
        let mut out: Vec<usize> = Vec::new();
        let mut i: usize = 0;
        while i < queries.len()
            invariant
                i <= queries@.len(),
                qs == query_views(queries@),
                forall|p: Seq<u8>| #[trigger] oracle.identity(p) == o0.identity(p),
                out@ == tracked_positions::<F, O>(
                    self.entries_view(),
                    self.protects_folded_names(),
                    qs,
                    i as nat,
                    o0,
                ),
                o0 == *old(oracle),
                base == old(oracle).probes(),
                oracle.probes() == base + select_trace::<F, O>(
                    self.entries_view(),
                    self.protects_folded_names(),
                    qs,
                    i as nat,
                    o0,
                ),
            decreases queries.len() - i,
        {
            let ghost before = *oracle;
            let hit = self.contains(oracle, queries[i]);
            proof {
                let es = self.entries_view();
                let protect = self.protects_folded_names();
                assert(qs[i as int] == queries@[i as int]@);
                lemma_is_tracked_depends_on_identity::<F, O>(es, protect, qs[i as int], before, o0);
                lemma_probe_trace_depends_on_identity::<F, O>(
                    es,
                    protect,
                    qs[i as int],
                    before,
                    o0,
                );
                let prev = select_trace::<F, O>(es, protect, qs, i as nat, o0);
                let step = probe_trace::<F, O>(es, protect, qs[i as int], o0);
                assert(select_trace::<F, O>(es, protect, qs, (i + 1) as nat, o0) == prev + step);
                assert(oracle.probes() =~= base + (prev + step));
            }
            if hit {
                out.push(i);
            }
            i += 1;
        }
        out
    }
}

/// `select` is exactly a filter: its result lists, in ascending order, the
/// positions of precisely the tracked queries.
pub proof fn lemma_tracked_positions_exact<F: FoldOracle, O: IdentityOracle>(
    entries: Seq<Seq<u8>>,
    protect: bool,
    queries: Seq<Seq<u8>>,
    n: nat,
    o: O,
)
    requires
        n <= queries.len(),
        queries.len() <= usize::MAX,
    ensures
        forall|j: int|
            0 <= j < tracked_positions::<F, O>(entries, protect, queries, n, o).len() ==> (
            #[trigger] tracked_positions::<F, O>(entries, protect, queries, n, o)[j] as int) < n,
        forall|a: int, b: int|
            0 <= a < b < tracked_positions::<F, O>(entries, protect, queries, n, o).len()
                ==> tracked_positions::<F, O>(entries, protect, queries, n, o)[a]
                < tracked_positions::<F, O>(entries, protect, queries, n, o)[b],
        forall|i: int|
            0 <= i < n ==> (is_tracked::<F, O>(entries, protect, #[trigger] queries[i], o)
                <==> exists|j: int|
                0 <= j < tracked_positions::<F, O>(entries, protect, queries, n, o).len()
                    && tracked_positions::<F, O>(entries, protect, queries, n, o)[j] as int == i),
    decreases n,
{
    if n > 0 {
        let m = (n - 1) as nat;
        lemma_tracked_positions_exact::<F, O>(entries, protect, queries, m, o);
        let rest = tracked_positions::<F, O>(entries, protect, queries, m, o);
        let ps = tracked_positions::<F, O>(entries, protect, queries, n, o);
        if is_tracked::<F, O>(entries, protect, queries[n - 1], o) {
            assert(ps == rest.push((n - 1) as usize));
            assert forall|j: int| 0 <= j < ps.len() implies (#[trigger] ps[j] as int) < n by {
                if j < rest.len() {
                    assert(ps[j] == rest[j]);
                }
            }
            assert forall|i: int| 0 <= i < n implies (is_tracked::<F, O>(
                entries,
                protect,
                #[trigger] queries[i],
                o,
            ) <==> exists|j: int| 0 <= j < ps.len() && ps[j] as int == i) by {
                if i == n - 1 {
                    assert(ps[rest.len() as int] as int == i);
                } else {
                    if is_tracked::<F, O>(entries, protect, queries[i], o) {
                        let j = choose|j: int| 0 <= j < rest.len() && rest[j] as int == i;
                        assert(ps[j] == rest[j]);
                    }
                    if exists|j: int| 0 <= j < ps.len() && ps[j] as int == i {
                        let j = choose|j: int| 0 <= j < ps.len() && ps[j] as int == i;
                        if j < rest.len() {
                            assert(ps[j] == rest[j]);
                        }
                    }
                }
            }
        } else {
            assert forall|i: int| 0 <= i < n implies (is_tracked::<F, O>(
                entries,
                protect,
                #[trigger] queries[i],
                o,
            ) <==> exists|j: int| 0 <= j < ps.len() && ps[j] as int == i) by {
                if exists|j: int| 0 <= j < ps.len() && ps[j] as int == i {
                    let j = choose|j: int| 0 <= j < ps.len() && ps[j] as int == i;
                    assert((rest[j] as int) < m);
                }
            }
        }
    }
}

/// Two oracles that agree on every identity agree on every tracked answer.
proof fn lemma_is_tracked_depends_on_identity<F: FoldOracle, O: IdentityOracle>(
    entries: Seq<Seq<u8>>,
    protect: bool,
    q: Seq<u8>,
    a: O,
    b: O,
)
    requires
        forall|p: Seq<u8>| #[trigger] a.identity(p) == b.identity(p),
    ensures
        is_tracked::<F, O>(entries, protect, q, a) == is_tracked::<F, O>(entries, protect, q, b),
{
    if identity_collision::<F, O>(entries, q, a) {
        let i = choose|i: int|
            0 <= i < entries.len() && F::fold(#[trigger] entries[i]) == F::fold(q) && same_object(
                a.identity(q),
                a.identity(entries[i]),
            );
        assert(a.identity(entries[i]) == b.identity(entries[i]));
    }
    if identity_collision::<F, O>(entries, q, b) {
        let i = choose|i: int|
            0 <= i < entries.len() && F::fold(#[trigger] entries[i]) == F::fold(q) && same_object(
                b.identity(q),
                b.identity(entries[i]),
            );
        assert(a.identity(entries[i]) == b.identity(entries[i]));
    }
}

/// Two oracles that agree on every identity probe the same paths in the same
/// order.
proof fn lemma_probe_trace_depends_on_identity<F: FoldOracle, O: IdentityOracle>(
    entries: Seq<Seq<u8>>,
    protect: bool,
    q: Seq<u8>,
    a: O,
    b: O,
)
    requires
        forall|p: Seq<u8>| #[trigger] a.identity(p) == b.identity(p),
    ensures
        probe_trace::<F, O>(entries, protect, q, a) == probe_trace::<F, O>(entries, protect, q, b),
{
    assert(a.identity(q) == b.identity(q));
    lemma_probe_until_match_depends_on_identity(
        a,
        b,
        colliding_entries::<F>(entries, q),
        a.identity(q),
    );
}

proof fn lemma_probe_until_match_depends_on_identity<O: IdentityOracle>(
    a: O,
    b: O,
    paths: Seq<Seq<u8>>,
    id: Option<FileId>,
)
    requires
        forall|p: Seq<u8>| #[trigger] a.identity(p) == b.identity(p),
    ensures
        probe_until_match(a, paths, id) == probe_until_match(b, paths, id),
    decreases paths.len(),
{
    if paths.len() > 0 {
        assert(a.identity(paths[0]) == b.identity(paths[0]));
        lemma_probe_until_match_depends_on_identity(a, b, paths.drop_first(), id);
    }
}

/// On a case-folding checkout a whole batch of queries is decided without a
/// single filesystem probe.
pub proof fn theorem_select_on_folding_checkout_never_probes<F: FoldOracle, O: IdentityOracle>(
    entries: Seq<Seq<u8>>,
    queries: Seq<Seq<u8>>,
    n: nat,
    o: O,
)
    ensures
        select_trace::<F, O>(entries, true, queries, n, o) == Seq::<Seq<u8>>::empty(),
    decreases n,
{
    if n > 0 {
        theorem_select_on_folding_checkout_never_probes::<F, O>(entries, queries, (n - 1) as nat, o);
        assert(select_trace::<F, O>(entries, true, queries, n, o) =~= Seq::<Seq<u8>>::empty());
    }
}

/// `p` is the query `q` itself or an index entry that folds onto it.
pub open spec fn probe_target<F: FoldOracle>(entries: Seq<Seq<u8>>, q: Seq<u8>, p: Seq<u8>) -> bool {
    p == q || exists|e: int|
        0 <= e < entries.len() && #[trigger] entries[e] == p && F::fold(entries[e]) == F::fold(q)
}

/// `p` is one of the first `n` queries or an index entry that folds onto
/// one of them.
pub open spec fn in_queried_collisions<F: FoldOracle>(
    entries: Seq<Seq<u8>>,
    queries: Seq<Seq<u8>>,
    n: nat,
    p: Seq<u8>,
) -> bool {
    exists|i: int| 0 <= i < n && #[trigger] probe_target::<F>(entries, queries[i], p)
}

/// Every path a batch probes is one of its queries or an index entry that
/// folds onto one of them: nothing outside the queried collisions is ever
/// `stat`ed.
pub proof fn theorem_select_probes_stay_in_the_collisions<F: FoldOracle, O: IdentityOracle>(
    entries: Seq<Seq<u8>>,
    protect: bool,
    queries: Seq<Seq<u8>>,
    n: nat,
    o: O,
)
    requires
        n <= queries.len(),
        entries.len() <= usize::MAX,
    ensures
        forall|k: int|
            0 <= k < select_trace::<F, O>(entries, protect, queries, n, o).len()
                ==> in_queried_collisions::<F>(
                entries,
                queries,
                n,
                #[trigger] select_trace::<F, O>(entries, protect, queries, n, o)[k],
            ),
    decreases n,
{
    if n > 0 {
        let m = (n - 1) as nat;
        theorem_select_probes_stay_in_the_collisions::<F, O>(entries, protect, queries, m, o);
        theorem_probes_stay_in_the_collision::<F, O>(entries, protect, queries[m as int], o);
        let prev = select_trace::<F, O>(entries, protect, queries, m, o);
        let step = probe_trace::<F, O>(entries, protect, queries[m as int], o);
        let t = select_trace::<F, O>(entries, protect, queries, n, o);
        assert(t == prev + step);
        assert forall|k: int| 0 <= k < t.len() implies in_queried_collisions::<F>(
            entries,
            queries,
            n,
            #[trigger] t[k],
        ) by {
            if k < prev.len() {
                assert(t[k] == prev[k]);
                assert(in_queried_collisions::<F>(entries, queries, m, prev[k]));
                let i = choose|i: int|
                    0 <= i < m && #[trigger] probe_target::<F>(entries, queries[i], prev[k]);
                assert(probe_target::<F>(entries, queries[i], t[k]));
            } else {
                let j = k - prev.len();
                assert(t[k] == step[j]);
                if j > 0 {
                    let e = choose|e: int|
                        0 <= e < entries.len() && #[trigger] entries[e] == step[j] && F::fold(
                            entries[e],
                        ) == F::fold(queries[m as int]);
                    assert(entries[e] == t[k]);
                }
                assert(probe_target::<F>(entries, queries[m as int], t[k]));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Fold keys
// ---------------------------------------------------------------------------
/// First byte of the fold key of a path that is valid UTF-8.
pub const UTF8_KEY_TAG: u8 = 0;

/// First byte of the fold key of a path that is not valid UTF-8.
pub const NON_UTF8_KEY_TAG: u8 = 1;

/// ASCII lowercase of one byte; every other byte is unchanged.
pub open spec fn ascii_lower(b: u8) -> u8 {
    if b'A' <= b <= b'Z' {
        (b + 32) as u8
    } else {
        b
    }
}

/// Bytewise ASCII lowercase.
pub open spec fn ascii_lower_seq(s: Seq<u8>) -> Seq<u8> {
    s.map_values(|b: u8| ascii_lower(b))
}

/// The fold key of a UTF-8 path whose Unicode fold (computed by production)
/// is `folded`.
pub fn utf8_fold_key(folded: &[u8]) -> (key: Vec<u8>)
    ensures
        key@ == seq![UTF8_KEY_TAG] + folded@,
{
    let mut key: Vec<u8> = Vec::new();
    key.push(UTF8_KEY_TAG);
    key.extend_from_slice(folded);
    assert(key@ =~= seq![UTF8_KEY_TAG] + folded@);
    key
}

#[allow(clippy::manual_is_ascii_check)]
fn ascii_lower_byte(b: u8) -> (r: u8)
    ensures
        r == ascii_lower(b),
{
    if (b'A'..=b'Z').contains(&b) {
        b + 32
    } else {
        b
    }
}

/// The fold key of a path that is not valid UTF-8: its bytes, ASCII-lowercased.
pub fn non_utf8_fold_key(raw: &[u8]) -> (key: Vec<u8>)
    ensures
        key@ == seq![NON_UTF8_KEY_TAG] + ascii_lower_seq(raw@),
{
    let mut key: Vec<u8> = Vec::new();
    key.push(NON_UTF8_KEY_TAG);
    let mut i: usize = 0;
    while i < raw.len()
        invariant
            i <= raw@.len(),
            key@ == seq![NON_UTF8_KEY_TAG] + ascii_lower_seq(raw@.subrange(0, i as int)),
        decreases raw.len() - i,
    {
        let b = ascii_lower_byte(raw[i]);
        key.push(b);
        assert(ascii_lower_seq(raw@.subrange(0, i + 1)) =~= ascii_lower_seq(
            raw@.subrange(0, i as int),
        ).push(b));
        assert(key@ =~= seq![NON_UTF8_KEY_TAG] + ascii_lower_seq(raw@.subrange(0, i + 1)));
        i += 1;
    }
    assert(raw@.subrange(0, raw@.len() as int) == raw@);
    key
}

/// A UTF-8 path's key and a non-UTF-8 path's key are never equal, so the two
/// kinds of path can never fold onto each other.
pub proof fn theorem_fold_key_kinds_never_collide(folded: Seq<u8>, raw: Seq<u8>)
    ensures
        seq![UTF8_KEY_TAG] + folded != seq![NON_UTF8_KEY_TAG] + ascii_lower_seq(raw),
{
    assert((seq![UTF8_KEY_TAG] + folded)[0] == UTF8_KEY_TAG);
    assert((seq![NON_UTF8_KEY_TAG] + ascii_lower_seq(raw))[0] == NON_UTF8_KEY_TAG);
}

/// Equal UTF-8 keys come from equal folds, so the tag adds no collisions.
pub proof fn theorem_utf8_fold_key_is_injective(a: Seq<u8>, b: Seq<u8>)
    requires
        seq![UTF8_KEY_TAG] + a == seq![UTF8_KEY_TAG] + b,
    ensures
        a == b,
{
    assert(a =~= (seq![UTF8_KEY_TAG] + a).drop_first());
    assert(b =~= (seq![UTF8_KEY_TAG] + b).drop_first());
}

/// The non-UTF-8 fold preserves length, is idempotent, and changes nothing
/// but ASCII uppercase letters.
pub proof fn theorem_ascii_fold_shape(raw: Seq<u8>)
    ensures
        ascii_lower_seq(raw).len() == raw.len(),
        ascii_lower_seq(ascii_lower_seq(raw)) == ascii_lower_seq(raw),
        forall|i: int|
            0 <= i < raw.len() && !(b'A' <= raw[i] <= b'Z') ==> #[trigger] ascii_lower_seq(raw)[i]
                == raw[i],
{
    assert(ascii_lower_seq(ascii_lower_seq(raw)) =~= ascii_lower_seq(raw));
}

// ---------------------------------------------------------------------------
// Repository path equivalence
// ---------------------------------------------------------------------------
/// Two repository paths are equivalent: byte-identical, or (when the
/// checkout folds case) equal under the fold.
pub open spec fn equivalent<F: FoldOracle>(a: Seq<u8>, b: Seq<u8>, ignore_case: bool) -> bool {
    a == b || (ignore_case && F::fold(a) == F::fold(b))
}

/// Decide `equivalent`, folding only when the bytes differ and the
/// checkout folds case.
pub fn paths_equivalent<F: FoldOracle>(fold: &F, a: &[u8], b: &[u8], ignore_case: bool) -> (r:
    bool)
    ensures
        r == equivalent::<F>(a@, b@, ignore_case),
{
    if a == b {
        assert(a@ =~= b@);
        return true;
    }
    if !ignore_case {
        return false;
    }
    let ka = fold.fold_key(a);
    let kb = fold.fold_key(b);
    let same = ka.as_slice() == kb.as_slice();
    if same {
        assert(ka@ =~= kb@);
    }
    same
}

/// For each case policy, `equivalent` is an equivalence relation that
/// contains byte equality, and folding only ever merges more paths.
pub proof fn theorem_equivalence_relation<F: FoldOracle>(
    a: Seq<u8>,
    b: Seq<u8>,
    c: Seq<u8>,
    ignore_case: bool,
)
    ensures
        equivalent::<F>(a, a, ignore_case),
        equivalent::<F>(a, b, ignore_case) == equivalent::<F>(b, a, ignore_case),
        equivalent::<F>(a, b, ignore_case) && equivalent::<F>(b, c, ignore_case) ==> equivalent::<
            F,
        >(a, c, ignore_case),
        equivalent::<F>(a, b, false) == (a == b),
        equivalent::<F>(a, b, false) ==> equivalent::<F>(a, b, true),
{
}

// ---------------------------------------------------------------------------
// Filesystem alias decision
// ---------------------------------------------------------------------------
/// Whether the probed path `alias` (with identity `alias_identity`, already
/// `None` when it names an entry exactly or does not resolve) is a filesystem
/// alias of `canonical`.
pub open spec fn resolves_to<O: IdentityOracle>(
    alias_identity: Option<FileId>,
    alias: Seq<u8>,
    canonical: Seq<u8>,
    o: O,
) -> bool {
    alias_identity is Some && canonical != alias && o.identity(canonical) == alias_identity
}

/// Decide `resolves_to`; the filesystem is probed (once, for `canonical`)
/// only when the alias resolved and the two spellings differ.
pub fn alias_resolves_to<O: IdentityOracle>(
    oracle: &mut O,
    alias_identity: Option<FileId>,
    alias: &[u8],
    canonical: &[u8],
) -> (r: bool)
    ensures
        r == resolves_to(alias_identity, alias@, canonical@, *old(oracle)),
        final(oracle).probes() == if alias_identity is Some && canonical@ != alias@ {
            old(oracle).probes().push(canonical@)
        } else {
            old(oracle).probes()
        },
        forall|p: Seq<u8>| #[trigger] final(oracle).identity(p) == old(oracle).identity(p),
        r ==> canonical@ != alias@,
{
    if alias_identity.is_none() {
        return false;
    }
    if canonical == alias {
        assert(canonical@ =~= alias@);
        return false;
    }
    let id = oracle.query_identity(canonical);
    same_identity(id, alias_identity)
}

} // verus!
