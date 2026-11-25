//! Verified kernel: eligibility grouping into maximal copyable directories,
//! and the order of path lists.
//!
//! # What it decides
//!
//! `eligibility_groups::compute` walks the source tree and reports the
//! outermost directories whose every entry is eligible, plus the eligible
//! files outside them. Three decisions in it are made here:
//!
//! * [`dir_copyable`]: whether one directory is fully copyable, from what
//!   the walk found in it ([`ChildKind`]).
//! * [`outermost`]: which copyable directories are reported, namely those
//!   with no copyable `'/'`-ancestor.
//! * [`uncovered`]: which eligible paths remain outside every reported
//!   directory.
//!
//! [`sort_unique`] orders and deduplicates the path lists the planner works
//! from (`planner::plan` and `EligibilityGroups::from_files`).
//!
//! Paths are UTF-8 repository-relative paths compared as byte strings; `'/'`
//! is ASCII, so byte-level ancestry is exact, and byte-lexicographic order
//! (`lex_cmp`) is `String`'s order.
//!
//! # Theorems
//!
//! * [`dir_copyable`]: a directory is copyable exactly when it has at least
//!   one entry and every entry is an eligible regular file or a copyable
//!   directory (`copyable_spec`). Applied bottom-up by the walk, this makes a
//!   directory copyable exactly when its subtree is non-empty and contains
//!   only eligible regular files and non-empty directories, with no symlink,
//!   special file, git boundary, or unnormalizable entry anywhere.
//! * [`outermost`]: `keep[i]` holds exactly when no `dirs[j]` is a
//!   `'/'`-ancestor of `dirs[i]` (`is_dir_ancestor`).
//! * `lemma_dir_ancestor_trans` and `lemma_common_descendant`: ancestry over
//!   `'/'`-separated byte paths is transitive, and two directories with a
//!   common descendant are equal or one contains the other.
//! * `theorem_outermost_antichain`: no reported directory contains another.
//! * `theorem_outermost_disjoint`: no path lies under two distinct reported
//!   directories, so reported directories cover disjoint sets of files.
//! * `theorem_outermost_covers`: every copyable directory is a reported
//!   directory or lies under one, and that reported directory is unique.
//! * [`uncovered`]: `r[i]` holds exactly when `eligible[i]` is not among the
//!   covered paths. `theorem_partition`: when the covered paths are eligible,
//!   the eligible paths are exactly the covered ones plus the remaining ones,
//!   and no path is both. Since the planner turns reported directories back
//!   into their files, directory promotion can neither add nor drop a file
//!   from the copy plan.
//! * [`sort_unique`]: afterwards the keys are strictly increasing (no path
//!   appears twice, so each destination is planned at most once), the set of
//!   keys is unchanged, and every element is one of the originals.
//! * `lemma_lex_equal`, `lemma_lex_flip`, `lemma_lex_trans`: `lex_cmp` is a
//!   total order whose only ties are equal strings; [`cmp_bytes`] computes it.
//!
//! # Trust assumptions
//!
//! * The walk (production) reports each entry's kind faithfully and applies
//!   [`dir_copyable`] bottom-up; a copyable directory's recorded files are
//!   the eligible files under it, so they lie under it and are eligible.
//! * Production passes the copyable directories' keys and the covered and
//!   eligible paths as the UTF-8 bytes of their repository-relative paths.
//!   No ordering is assumed: the kernel checks sortedness itself before it
//!   uses binary search and otherwise falls back to a linear scan.
//! * [`PathKey::key_bytes`] (production: `RepoRelPath`'s UTF-8 bytes) returns
//!   the same bytes every time it is called on the same value.
//! * `std`'s `sort_by` meets vstd's specification (a permutation, sorted by
//!   the comparator) given the comparator is a total order, which is proven.

#[allow(unused_imports)]
use vstd::prelude::*;

use core::cmp::Ordering;
#[cfg(verus_keep_ghost)]
use vstd::std_specs::slice::{cmp_fn_total_on, compare_le, sorted_by_le};

verus! {

// ---------------------------------------------------------------------------
// Copyable directories.
// ---------------------------------------------------------------------------

/// What one directory entry turned out to be during the eligibility walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildKind {
    /// A git boundary directory: a nested repository or a gitlink.
    GitBoundary,
    /// A directory, with the walk's verdict for it.
    Directory {
        /// Whether that directory is fully copyable.
        copyable: bool,
    },
    /// A non-directory entry whose path is not a valid repository path.
    Unnormalizable,
    /// A symlink.
    Symlink,
    /// A regular file.
    RegularFile {
        /// Whether the file is eligible.
        eligible: bool,
    },
    /// Anything else: a socket, device, or FIFO.
    Other,
}

/// An entry a copyable directory may contain.
pub open spec fn child_ok(c: ChildKind) -> bool {
    c == ChildKind::Directory { copyable: true } || c == ChildKind::RegularFile { eligible: true }
}

/// A directory is copyable when it has at least one entry and every entry
/// is an eligible regular file or a copyable directory.
pub open spec fn copyable_spec(children: Seq<ChildKind>) -> bool {
    children.len() > 0 && forall|i: int| 0 <= i < children.len() ==> child_ok(#[trigger] children[i])
}

/// Whether a directory with these entries is fully copyable.
pub fn dir_copyable(children: &[ChildKind]) -> (r: bool)
    ensures
        r == copyable_spec(children@),
{
    if children.is_empty() {
        return false;
    }
    let mut i: usize = 0;
    while i < children.len()
        invariant
            i <= children@.len(),
            forall|j: int| 0 <= j < i ==> child_ok(#[trigger] children@[j]),
        decreases children.len() - i,
    {
        match children[i] {
            ChildKind::Directory { copyable: true } | ChildKind::RegularFile { eligible: true } => {},
            _ => {
                assert(!child_ok(children@[i as int]));
                return false;
            },
        }
        i += 1;
    }
    true
}

// ---------------------------------------------------------------------------
// Byte-string order.
// ---------------------------------------------------------------------------

/// Lexicographic order of byte strings (the order of Rust's `str` and
/// `String`).
pub open spec fn lex_cmp(a: Seq<u8>, b: Seq<u8>) -> Ordering
    decreases a.len(),
{
    if a.len() == 0 {
        if b.len() == 0 {
            Ordering::Equal
        } else {
            Ordering::Less
        }
    } else if b.len() == 0 {
        Ordering::Greater
    } else if a[0] < b[0] {
        Ordering::Less
    } else if a[0] > b[0] {
        Ordering::Greater
    } else {
        lex_cmp(a.drop_first(), b.drop_first())
    }
}

/// The reverse of an ordering.
pub open spec fn flip(o: Ordering) -> Ordering {
    match o {
        Ordering::Less => Ordering::Greater,
        Ordering::Equal => Ordering::Equal,
        Ordering::Greater => Ordering::Less,
    }
}

/// Only equal byte strings compare equal.
pub proof fn lemma_lex_equal(a: Seq<u8>, b: Seq<u8>)
    ensures
        (lex_cmp(a, b) == Ordering::Equal) == (a == b),
    decreases a.len(),
{
    if a.len() > 0 && b.len() > 0 && a[0] == b[0] {
        lemma_lex_equal(a.drop_first(), b.drop_first());
        if a.drop_first() == b.drop_first() {
            assert(a =~= b) by {
                assert forall|i: int| 0 <= i < a.len() implies a[i] == b[i] by {
                    if i > 0 {
                        assert(a[i] == a.drop_first()[i - 1]);
                        assert(b[i] == b.drop_first()[i - 1]);
                    }
                }
            }
        }
        if a == b {
            assert(a.drop_first() == b.drop_first());
        }
    } else if a.len() > 0 && b.len() > 0 {
        assert(a[0] != b[0]);
    } else if a.len() == 0 && b.len() == 0 {
        assert(a =~= b);
    }
}

/// Comparing the other way round flips the answer.
pub proof fn lemma_lex_flip(a: Seq<u8>, b: Seq<u8>)
    ensures
        lex_cmp(b, a) == flip(lex_cmp(a, b)),
    decreases a.len(),
{
    if a.len() > 0 && b.len() > 0 && a[0] == b[0] {
        lemma_lex_flip(a.drop_first(), b.drop_first());
    }
}

/// The order is transitive, strictly so when either step is strict.
pub proof fn lemma_lex_trans(a: Seq<u8>, b: Seq<u8>, c: Seq<u8>)
    requires
        lex_cmp(a, b) != Ordering::Greater,
        lex_cmp(b, c) != Ordering::Greater,
    ensures
        lex_cmp(a, c) != Ordering::Greater,
        lex_cmp(a, b) == Ordering::Less || lex_cmp(b, c) == Ordering::Less ==> lex_cmp(a, c)
            == Ordering::Less,
    decreases a.len(),
{
    if a.len() > 0 && b.len() > 0 && c.len() > 0 && a[0] == b[0] && b[0] == c[0] {
        lemma_lex_trans(a.drop_first(), b.drop_first(), c.drop_first());
    }
}

/// Compare two byte strings.
pub fn cmp_bytes(a: &[u8], b: &[u8]) -> (r: Ordering)
    ensures
        r == lex_cmp(a@, b@),
{
    let mut i: usize = 0;
    assert(a@.subrange(0, a@.len() as int) == a@);
    assert(b@.subrange(0, b@.len() as int) == b@);
    while i < a.len() && i < b.len()
        invariant
            0 <= i <= a@.len(),
            i <= b@.len(),
            lex_cmp(a@, b@) == lex_cmp(
                a@.subrange(i as int, a@.len() as int),
                b@.subrange(i as int, b@.len() as int),
            ),
        decreases a.len() - i,
    {
        let ghost sa = a@.subrange(i as int, a@.len() as int);
        let ghost sb = b@.subrange(i as int, b@.len() as int);
        assert(sa[0] == a@[i as int]);
        assert(sb[0] == b@[i as int]);
        if a[i] < b[i] {
            return Ordering::Less;
        } else if a[i] > b[i] {
            return Ordering::Greater;
        }
        assert(sa.drop_first() == a@.subrange(i + 1, a@.len() as int));
        assert(sb.drop_first() == b@.subrange(i + 1, b@.len() as int));
        i += 1;
    }
    if i < a.len() {
        Ordering::Greater
    } else if i < b.len() {
        Ordering::Less
    } else {
        Ordering::Equal
    }
}

// ---------------------------------------------------------------------------
// Membership in a set of paths.
// ---------------------------------------------------------------------------

/// `t` is one of the paths in `set`.
pub open spec fn in_set(set: Seq<&[u8]>, t: Seq<u8>) -> bool {
    exists|j: int| 0 <= j < set.len() && (#[trigger] set[j])@ == t
}

/// `set` is strictly increasing.
pub open spec fn sorted_strict(set: Seq<&[u8]>) -> bool {
    forall|a: int, b: int| 0 <= a < b < set.len() ==> lex_cmp((#[trigger] set[a])@, (#[trigger] set[b])@)
        == Ordering::Less
}

/// Whether `set` is strictly increasing (checked, so callers need not assume
/// anything about the order they pass).
pub fn is_strictly_sorted(set: &[&[u8]]) -> (r: bool)
    ensures
        r ==> sorted_strict(set@),
{
    let mut i: usize = 1;
    while i < set.len()
        invariant
            1 <= i,
            forall|a: int, b: int| 0 <= a < b < i && b < set@.len() ==> lex_cmp(
                (#[trigger] set@[a])@,
                (#[trigger] set@[b])@,
            ) == Ordering::Less,
        decreases set.len() - i,
    {
        if !matches!(cmp_bytes(set[i - 1], set[i]), Ordering::Less) {
            return false;
        }
        proof {
            assert forall|a: int| 0 <= a < i implies lex_cmp(
                (#[trigger] set@[a])@,
                set@[i as int]@,
            ) == Ordering::Less by {
                if a < i - 1 {
                    lemma_lex_trans(set@[a]@, set@[i - 1]@, set@[i as int]@);
                }
            }
        }
        i += 1;
    }
    true
}

/// Binary search in a strictly increasing set.
fn member_sorted(set: &[&[u8]], t: &[u8]) -> (r: bool)
    requires
        sorted_strict(set@),
    ensures
        r == in_set(set@, t@),
{
    let mut lo: usize = 0;
    let mut hi: usize = set.len();
    while lo < hi
        invariant
            0 <= lo <= hi <= set@.len(),
            sorted_strict(set@),
            forall|i: int| 0 <= i < lo ==> lex_cmp((#[trigger] set@[i])@, t@) == Ordering::Less,
            forall|i: int| hi <= i < set@.len() ==> lex_cmp((#[trigger] set@[i])@, t@)
                == Ordering::Greater,
        decreases hi - lo,
    {
        let mid = lo + (hi - lo) / 2;
        let o = cmp_bytes(set[mid], t);
        if matches!(o, Ordering::Equal) {
            proof {
                lemma_lex_equal(set@[mid as int]@, t@);
            }
            return true;
        } else if matches!(o, Ordering::Less) {
            proof {
                assert forall|i: int| 0 <= i < mid + 1 implies lex_cmp((#[trigger] set@[i])@, t@)
                    == Ordering::Less by {
                    if i < mid {
                        lemma_lex_trans(set@[i]@, set@[mid as int]@, t@);
                    }
                }
            }
            lo = mid + 1;
        } else {
            proof {
                lemma_lex_flip(set@[mid as int]@, t@);
                assert forall|i: int| mid <= i < set@.len() implies lex_cmp(
                    (#[trigger] set@[i])@,
                    t@,
                ) == Ordering::Greater by {
                    if i > mid {
                        lemma_lex_trans(t@, set@[mid as int]@, set@[i]@);
                        lemma_lex_flip(t@, set@[i]@);
                    }
                }
            }
            hi = mid;
        }
    }
    proof {
        assert forall|j: int| 0 <= j < set@.len() implies (#[trigger] set@[j])@ != t@ by {
            lemma_lex_equal(set@[j]@, t@);
        }
    }
    false
}

/// Linear search, for a set that is not known to be sorted.
fn member_linear(set: &[&[u8]], t: &[u8]) -> (r: bool)
    ensures
        r == in_set(set@, t@),
{
    let mut j: usize = 0;
    while j < set.len()
        invariant
            j <= set@.len(),
            forall|k: int| 0 <= k < j ==> (#[trigger] set@[k])@ != t@,
        decreases set.len() - j,
    {
        let o = cmp_bytes(set[j], t);
        proof {
            lemma_lex_equal(set@[j as int]@, t@);
        }
        if matches!(o, Ordering::Equal) {
            return true;
        }
        j += 1;
    }
    false
}

/// Membership, by binary search when `sorted` says `set` is strictly
/// increasing.
fn member(set: &[&[u8]], sorted: bool, t: &[u8]) -> (r: bool)
    requires
        sorted ==> sorted_strict(set@),
    ensures
        r == in_set(set@, t@),
{
    if sorted {
        member_sorted(set, t)
    } else {
        member_linear(set, t)
    }
}

// ---------------------------------------------------------------------------
// Sorting and deduplicating path lists.
// ---------------------------------------------------------------------------

/// A value identified by a path: production implements it for
/// `RepoRelPath` by returning the path's UTF-8 bytes.
pub trait PathKey {
    /// The path bytes that identify the value.
    spec fn key(&self) -> Seq<u8>;

    /// The path bytes that identify the value.
    fn key_bytes(&self) -> (r: &[u8])
        ensures
            r@ == self.key(),
    ;
}

/// The keys of `s` are strictly increasing: sorted, with no key twice.
pub open spec fn keys_strictly_increasing<T: PathKey>(s: Seq<T>) -> bool {
    forall|i: int, j: int| 0 <= i < j < s.len() ==> lex_cmp((#[trigger] s[i]).key(), (#[trigger] s[j]).key())
        == Ordering::Less
}

/// Some element of `s` has key `k`.
pub open spec fn has_key<T: PathKey>(s: Seq<T>, k: Seq<u8>) -> bool {
    exists|i: int| 0 <= i < s.len() && (#[trigger] s[i]).key() == k
}

/// `a` and `b` have the same set of keys.
pub open spec fn same_keys<T: PathKey>(a: Seq<T>, b: Seq<T>) -> bool {
    forall|k: Seq<u8>| #![trigger has_key(a, k)] #![trigger has_key(b, k)] has_key(a, k) <==> has_key(b, k)
}

/// Sort `v` by key and keep one element per key.
///
/// Afterwards the keys are strictly increasing (so no path appears twice),
/// the set of keys is unchanged, and every element is one of the originals.
pub fn sort_unique<T: PathKey>(v: &mut Vec<T>)
    ensures
        keys_strictly_increasing(final(v)@),
        same_keys(old(v)@, final(v)@),
        forall|j: int| 0 <= j < final(v)@.len() ==> old(v)@.contains(#[trigger] final(v)@[j]),
{
    let cmp = |a: &T, b: &T| -> (o: Ordering)
        ensures
            o == lex_cmp(a.key(), b.key()),
        { cmp_bytes(a.key_bytes(), b.key_bytes()) };
    let ghost original = v@;
    proof {
        let s = v@;
        assert forall|x: &T, y: &T, o: Ordering| #[trigger] call_ensures(cmp, (x, y), o) implies o
            == lex_cmp(x.key(), y.key()) by {}
        assert forall|i: int, j: int| #![trigger s[i], s[j]] 0 <= i < s.len() && 0 <= j < s.len() implies lex_cmp(
            s[j].key(),
            s[i].key(),
        ) == flip(lex_cmp(s[i].key(), s[j].key())) by {
            lemma_lex_flip(s[i].key(), s[j].key());
        }
        assert forall|i: int, j: int, k: int| #![trigger s[i], s[j], s[k]]
            0 <= i < s.len() && 0 <= j < s.len() && 0 <= k < s.len() && lex_cmp(s[i].key(), s[j].key())
                != Ordering::Greater && lex_cmp(s[j].key(), s[k].key()) != Ordering::Greater implies {
            &&& lex_cmp(s[i].key(), s[k].key()) != Ordering::Greater
            &&& (lex_cmp(s[i].key(), s[j].key()) == Ordering::Less || lex_cmp(s[j].key(), s[k].key())
                == Ordering::Less ==> lex_cmp(s[i].key(), s[k].key()) == Ordering::Less)
        } by {
            lemma_lex_trans(s[i].key(), s[j].key(), s[k].key());
        }
        assert(cmp_fn_total_on(cmp, s));
    }
    v.as_mut_slice().sort_by(cmp);
    let ghost sorted = v@;
    proof {
        original.to_multiset_ensures();
        sorted.to_multiset_ensures();
        assert forall|a: int, b: int| 0 <= a < b < sorted.len() implies compare_le(cmp, sorted[a], sorted[b]) by {
            assert(sorted_by_le(sorted, |x: T, y: T| compare_le(cmp, x, y)));
        }
    }
    // The sort only promises that `cmp` returned "not greater" on the pairs
    // it was asked about; one call per adjacent pair turns that into the key
    // order.
    let mut i: usize = 1;
    while i < v.len()
        invariant
            v@ == sorted,
            sorted.to_multiset() == original.to_multiset(),
            1 <= i,
            forall|x: &T, y: &T| #[trigger] call_requires(cmp, (x, y)),
            forall|x: &T, y: &T, o: Ordering| #[trigger] call_ensures(cmp, (x, y), o) ==> o == lex_cmp(
                x.key(),
                y.key(),
            ),
            forall|a: int, b: int| 0 <= a < b < sorted.len() ==> compare_le(cmp, sorted[a], sorted[b]),
            forall|a: int| 0 < a < i && a < sorted.len() ==> lex_cmp(
                (#[trigger] sorted[a - 1]).key(),
                sorted[a].key(),
            ) != Ordering::Greater,
        decreases v.len() - i,
    {
        let _o = cmp(&v[i - 1], &v[i]);
        proof {
            assert(compare_le(cmp, sorted[i - 1], sorted[i as int]));
        }
        i += 1;
    }
    let n = v.len();
    if n == 0 {
        proof {
            original.to_multiset_ensures();
            sorted.to_multiset_ensures();
        }
        return;
    }
    proof {
        assert(sorted.subrange(0, 1)[0] == v@[0]);
        assert(v@.subrange(0, 1)[0] == sorted.subrange(0, 1)[0]);
        assert(v@.subrange(0, 1) =~= sorted.subrange(0, 1));
    }
    // Keep the first element of each run of equal keys, moving it down to
    // the end of the kept prefix.
    let mut w: usize = 1;
    let mut r: usize = 1;
    while r < n
        invariant
            v@.len() == n,
            n == sorted.len(),
            sorted.to_multiset() == original.to_multiset(),
            1 <= w <= r <= n,
            forall|a: int| 0 < a < n ==> lex_cmp((#[trigger] sorted[a - 1]).key(), sorted[a].key())
                != Ordering::Greater,
            forall|k: int| r <= k < n ==> #[trigger] v@[k] == sorted[k],
            forall|a: int, b: int| 0 <= a < b < w ==> lex_cmp((#[trigger] v@[a]).key(), (#[trigger] v@[b]).key())
                == Ordering::Less,
            v@[w - 1].key() == sorted[r - 1].key(),
            forall|a: int| 0 <= a < w ==> sorted.subrange(0, r as int).contains(#[trigger] v@[a]),
            same_keys(sorted.subrange(0, r as int), v@.subrange(0, w as int)),
        decreases n - r,
    {
        let ghost before = v@;
        let o = cmp_bytes(v[w - 1].key_bytes(), v[r].key_bytes());
        assert(lex_cmp(sorted[r - 1].key(), sorted[r as int].key()) != Ordering::Greater);
        if matches!(o, Ordering::Less) {
            v.as_mut_slice().swap(w, r);
            proof {
                assert(v@[w as int] == sorted[r as int]);
                assert forall|a: int| 0 <= a < w implies lex_cmp(
                    (#[trigger] v@[a]).key(),
                    v@[w as int].key(),
                ) == Ordering::Less by {
                    assert(v@[a] == before[a]);
                    if a < w - 1 {
                        lemma_lex_trans(before[a].key(), before[w - 1].key(), sorted[r as int].key());
                    }
                }
                assert(sorted.subrange(0, r + 1) == sorted.subrange(0, r as int).push(sorted[r as int]));
                assert(v@.subrange(0, w + 1) == before.subrange(0, w as int).push(sorted[r as int]));
                assert forall|a: int| 0 <= a < w + 1 implies sorted.subrange(0, r + 1).contains(
                    #[trigger] v@[a],
                ) by {
                    if a < w {
                        assert(v@[a] == before[a]);
                        assert(sorted.subrange(0, r as int).contains(before[a]));
                    } else {
                        assert(sorted.subrange(0, r + 1)[r as int] == v@[a]);
                    }
                }
                assert forall|k: Seq<u8>|
                    #![trigger has_key(sorted.subrange(0, r + 1), k)]
                    #![trigger has_key(v@.subrange(0, w + 1), k)]
                    has_key(sorted.subrange(0, r + 1), k) <==> has_key(v@.subrange(0, w + 1), k) by {
                    if has_key(sorted.subrange(0, r + 1), k) && k != sorted[r as int].key() {
                        let j = choose|j: int| 0 <= j < r + 1 && (#[trigger] sorted.subrange(0, r + 1)[j]).key() == k;
                        assert(sorted.subrange(0, r as int)[j].key() == k);
                        assert(has_key(sorted.subrange(0, r as int), k));
                        let m = choose|m: int| 0 <= m < w && (#[trigger] before.subrange(0, w as int)[m]).key() == k;
                        assert(v@.subrange(0, w + 1)[m].key() == k);
                    }
                    if has_key(sorted.subrange(0, r + 1), k) && k == sorted[r as int].key() {
                        assert(v@.subrange(0, w + 1)[w as int].key() == k);
                    }
                    if has_key(v@.subrange(0, w + 1), k) && k != sorted[r as int].key() {
                        let m = choose|m: int| 0 <= m < w + 1 && (#[trigger] v@.subrange(0, w + 1)[m]).key() == k;
                        assert(before.subrange(0, w as int)[m].key() == k);
                        assert(has_key(before.subrange(0, w as int), k));
                        let j = choose|j: int| 0 <= j < r && (#[trigger] sorted.subrange(0, r as int)[j]).key() == k;
                        assert(sorted.subrange(0, r + 1)[j].key() == k);
                    }
                    if has_key(v@.subrange(0, w + 1), k) && k == sorted[r as int].key() {
                        assert(sorted.subrange(0, r + 1)[r as int].key() == k);
                    }
                }
            }
            w += 1;
        } else {
            proof {
                // Equal keys: the previous kept key is `sorted[r - 1]`'s,
                // which is not after `sorted[r]`'s, and `o` is not "less".
                lemma_lex_equal(before[w - 1].key(), sorted[r as int].key());
                assert(sorted.subrange(0, r + 1) == sorted.subrange(0, r as int).push(sorted[r as int]));
                assert forall|a: int| 0 <= a < w implies sorted.subrange(0, r + 1).contains(
                    #[trigger] v@[a],
                ) by {
                    assert(sorted.subrange(0, r as int).contains(v@[a]));
                    let j = choose|j: int| 0 <= j < r && sorted.subrange(0, r as int)[j] == v@[a];
                    assert(sorted.subrange(0, r + 1)[j] == v@[a]);
                }
                assert forall|k: Seq<u8>|
                    #![trigger has_key(sorted.subrange(0, r + 1), k)]
                    #![trigger has_key(v@.subrange(0, w as int), k)]
                    has_key(sorted.subrange(0, r + 1), k) <==> has_key(v@.subrange(0, w as int), k) by {
                    if has_key(sorted.subrange(0, r + 1), k) && k != sorted[r as int].key() {
                        let j = choose|j: int| 0 <= j < r + 1 && (#[trigger] sorted.subrange(0, r + 1)[j]).key() == k;
                        assert(sorted.subrange(0, r as int)[j].key() == k);
                        assert(has_key(sorted.subrange(0, r as int), k));
                    }
                    if has_key(sorted.subrange(0, r + 1), k) && k == sorted[r as int].key() {
                        assert(v@.subrange(0, w as int)[w - 1].key() == k);
                    }
                    if has_key(v@.subrange(0, w as int), k) {
                        let j = choose|j: int| 0 <= j < r && (#[trigger] sorted.subrange(0, r as int)[j]).key() == k;
                        assert(sorted.subrange(0, r + 1)[j].key() == k);
                    }
                }
            }
        }
        r += 1;
    }
    let ghost kept = v@;
    v.truncate(w);
    proof {
        original.to_multiset_ensures();
        sorted.to_multiset_ensures();
        assert(sorted.subrange(0, n as int) == sorted);
        assert(v@ == kept.subrange(0, w as int));
        assert forall|k: Seq<u8>| #![trigger has_key(original, k)] #![trigger has_key(v@, k)]
            has_key(original, k) <==> has_key(v@, k) by {
            if has_key(original, k) {
                let i = choose|i: int| 0 <= i < original.len() && (#[trigger] original[i]).key() == k;
                assert(original.contains(original[i]));
                assert(original.to_multiset().count(original[i]) > 0);
                assert(sorted.to_multiset().count(original[i]) > 0);
                assert(sorted.contains(original[i]));
                let j = choose|j: int| 0 <= j < sorted.len() && sorted[j] == original[i];
                assert(sorted[j].key() == k);
                assert(has_key(sorted, k));
            }
            if has_key(v@, k) {
                assert(has_key(sorted, k));
                let j = choose|j: int| 0 <= j < sorted.len() && (#[trigger] sorted[j]).key() == k;
                assert(sorted.contains(sorted[j]));
                assert(sorted.to_multiset().count(sorted[j]) > 0);
                assert(original.to_multiset().count(sorted[j]) > 0);
                assert(original.contains(sorted[j]));
                let i = choose|i: int| 0 <= i < original.len() && original[i] == sorted[j];
                assert(original[i].key() == k);
            }
        }
        assert forall|j: int| 0 <= j < v@.len() implies original.contains(#[trigger] v@[j]) by {
            assert(v@[j] == kept[j]);
            assert(sorted.subrange(0, n as int).contains(kept[j]));
            assert(sorted.contains(kept[j]));
            assert(sorted.to_multiset().count(kept[j]) > 0);
            assert(original.to_multiset().count(kept[j]) > 0);
        }
    }
}

// ---------------------------------------------------------------------------
// Directory ancestry and the outermost directories.
// ---------------------------------------------------------------------------

/// `a` is a proper `'/'`-ancestor of `d`: `a` followed by `'/'` is a prefix
/// of `d`.
pub open spec fn is_dir_ancestor(a: Seq<u8>, d: Seq<u8>) -> bool {
    &&& a.len() < d.len()
    &&& d[a.len() as int] == 0x2Fu8
    &&& d.subrange(0, a.len() as int) == a
}

/// Some path in `dirs` is a `'/'`-ancestor of `d`.
pub open spec fn has_ancestor_in(dirs: Seq<&[u8]>, d: Seq<u8>) -> bool {
    exists|j: int| 0 <= j < dirs.len() && is_dir_ancestor((#[trigger] dirs[j])@, d)
}

/// Whether some path in `dirs` is a `'/'`-ancestor of `d`.
fn has_ancestor(dirs: &[&[u8]], sorted: bool, d: &[u8]) -> (r: bool)
    requires
        sorted ==> sorted_strict(dirs@),
    ensures
        r == has_ancestor_in(dirs@, d@),
{
    let mut k: usize = 0;
    while k < d.len()
        invariant
            k <= d@.len(),
            sorted ==> sorted_strict(dirs@),
            forall|j: int| 0 <= j < dirs@.len() && (#[trigger] dirs@[j])@.len() < k ==> !is_dir_ancestor(
                dirs@[j]@,
                d@,
            ),
        decreases d.len() - k,
    {
        if d[k] == 0x2Fu8 {
            let prefix = &d[..k];
            assert(prefix@ == d@.subrange(0, k as int));
            if member(dirs, sorted, prefix) {
                proof {
                    let j = choose|j: int| 0 <= j < dirs@.len() && (#[trigger] dirs@[j])@ == prefix@;
                    assert(is_dir_ancestor(dirs@[j]@, d@));
                }
                return true;
            }
            proof {
                assert forall|j: int| 0 <= j < dirs@.len() && (#[trigger] dirs@[j])@.len() < k + 1 implies !is_dir_ancestor(
                    dirs@[j]@,
                    d@,
                ) by {
                    if dirs@[j]@.len() == k && is_dir_ancestor(dirs@[j]@, d@) {
                        assert(dirs@[j]@ == prefix@);
                    }
                }
            }
        }
        k += 1;
    }
    false
}

/// For each path in `dirs`, whether no other path in `dirs` is a
/// `'/'`-ancestor of it: the outermost directories.
pub fn outermost(dirs: &[&[u8]]) -> (keep: Vec<bool>)
    ensures
        keep@.len() == dirs@.len(),
        forall|i: int| 0 <= i < dirs@.len() ==> #[trigger] keep@[i] == !has_ancestor_in(dirs@, dirs@[i]@),
{
    let sorted = is_strictly_sorted(dirs);
    let mut keep: Vec<bool> = Vec::new();
    let mut i: usize = 0;
    while i < dirs.len()
        invariant
            i <= dirs@.len(),
            sorted ==> sorted_strict(dirs@),
            keep@.len() == i,
            forall|j: int| 0 <= j < i ==> #[trigger] keep@[j] == !has_ancestor_in(dirs@, dirs@[j]@),
        decreases dirs.len() - i,
    {
        let has = has_ancestor(dirs, sorted, dirs[i]);
        keep.push(!has);
        i += 1;
    }
    keep
}

/// For each eligible path, whether it is outside the covered paths: the
/// paths that remain after the reported directories' files are taken out.
pub fn uncovered(eligible: &[&[u8]], covered: &[&[u8]]) -> (r: Vec<bool>)
    ensures
        r@.len() == eligible@.len(),
        forall|i: int| 0 <= i < eligible@.len() ==> #[trigger] r@[i] == !in_set(covered@, eligible@[i]@),
{
    let sorted = is_strictly_sorted(covered);
    let mut r: Vec<bool> = Vec::new();
    let mut i: usize = 0;
    while i < eligible.len()
        invariant
            i <= eligible@.len(),
            sorted ==> sorted_strict(covered@),
            r@.len() == i,
            forall|j: int| 0 <= j < i ==> #[trigger] r@[j] == !in_set(covered@, eligible@[j]@),
        decreases eligible.len() - i,
    {
        let is_covered = member(covered, sorted, eligible[i]);
        r.push(!is_covered);
        i += 1;
    }
    r
}

// ---------------------------------------------------------------------------
// Theorems.
// ---------------------------------------------------------------------------

/// Ancestry is transitive.
pub proof fn lemma_dir_ancestor_trans(a: Seq<u8>, b: Seq<u8>, c: Seq<u8>)
    requires
        is_dir_ancestor(a, b),
        is_dir_ancestor(b, c),
    ensures
        is_dir_ancestor(a, c),
{
    assert(c[a.len() as int] == b[a.len() as int]);
    assert(c.subrange(0, a.len() as int) =~= b.subrange(0, a.len() as int));
}

/// Two directories with a common descendant are equal, or one contains the
/// other.
pub proof fn lemma_common_descendant(a: Seq<u8>, b: Seq<u8>, f: Seq<u8>)
    requires
        is_dir_ancestor(a, f),
        is_dir_ancestor(b, f),
    ensures
        a == b || is_dir_ancestor(a, b) || is_dir_ancestor(b, a),
{
    if a.len() < b.len() {
        assert(b[a.len() as int] == f[a.len() as int]);
        assert(b.subrange(0, a.len() as int) =~= f.subrange(0, a.len() as int));
    } else if b.len() < a.len() {
        assert(a[b.len() as int] == f[b.len() as int]);
        assert(a.subrange(0, b.len() as int) =~= f.subrange(0, b.len() as int));
    } else {
        assert(a =~= f.subrange(0, a.len() as int));
    }
}

/// No reported directory contains another.
pub proof fn theorem_outermost_antichain(dirs: Seq<&[u8]>, keep: Seq<bool>, i: int, j: int)
    requires
        keep.len() == dirs.len(),
        forall|k: int| 0 <= k < dirs.len() ==> #[trigger] keep[k] == !has_ancestor_in(dirs, dirs[k]@),
        0 <= i < dirs.len(),
        0 <= j < dirs.len(),
        keep[i],
        keep[j],
    ensures
        !is_dir_ancestor(dirs[i]@, dirs[j]@),
{
    if is_dir_ancestor(dirs[i]@, dirs[j]@) {
        assert(has_ancestor_in(dirs, dirs[j]@));
    }
}

/// No path lies under two distinct reported directories.
pub proof fn theorem_outermost_disjoint(
    dirs: Seq<&[u8]>,
    keep: Seq<bool>,
    i: int,
    j: int,
    f: Seq<u8>,
)
    requires
        keep.len() == dirs.len(),
        forall|k: int| 0 <= k < dirs.len() ==> #[trigger] keep[k] == !has_ancestor_in(dirs, dirs[k]@),
        0 <= i < dirs.len(),
        0 <= j < dirs.len(),
        keep[i],
        keep[j],
        dirs[i]@ != dirs[j]@,
    ensures
        !(is_dir_ancestor(dirs[i]@, f) && is_dir_ancestor(dirs[j]@, f)),
{
    if is_dir_ancestor(dirs[i]@, f) && is_dir_ancestor(dirs[j]@, f) {
        lemma_common_descendant(dirs[i]@, dirs[j]@, f);
        theorem_outermost_antichain(dirs, keep, i, j);
        theorem_outermost_antichain(dirs, keep, j, i);
    }
}

/// Every copyable directory is reported or lies under a reported
/// directory, and that reported directory is unique.
pub proof fn theorem_outermost_covers(dirs: Seq<&[u8]>, keep: Seq<bool>, i: int)
    requires
        keep.len() == dirs.len(),
        forall|k: int| 0 <= k < dirs.len() ==> #[trigger] keep[k] == !has_ancestor_in(dirs, dirs[k]@),
        0 <= i < dirs.len(),
    ensures
        exists|j: int|
            0 <= j < dirs.len() && #[trigger] keep[j] && (dirs[j]@ == dirs[i]@ || is_dir_ancestor(
                dirs[j]@,
                dirs[i]@,
            )),
        forall|j1: int, j2: int|
            0 <= j1 < dirs.len() && 0 <= j2 < dirs.len() && #[trigger] keep[j1] && #[trigger] keep[j2]
                && (dirs[j1]@ == dirs[i]@ || is_dir_ancestor(dirs[j1]@, dirs[i]@)) && (dirs[j2]@
                == dirs[i]@ || is_dir_ancestor(dirs[j2]@, dirs[i]@)) ==> dirs[j1]@ == dirs[j2]@,
    decreases dirs[i]@.len(),
{
    if !keep[i] {
        let j = choose|j: int| 0 <= j < dirs.len() && is_dir_ancestor((#[trigger] dirs[j])@, dirs[i]@);
        theorem_outermost_covers(dirs, keep, j);
        let m = choose|m: int|
            0 <= m < dirs.len() && #[trigger] keep[m] && (dirs[m]@ == dirs[j]@ || is_dir_ancestor(
                dirs[m]@,
                dirs[j]@,
            ));
        if dirs[m]@ != dirs[j]@ {
            lemma_dir_ancestor_trans(dirs[m]@, dirs[j]@, dirs[i]@);
        }
        assert(keep[m] && is_dir_ancestor(dirs[m]@, dirs[i]@));
    }
    assert forall|j1: int, j2: int|
        0 <= j1 < dirs.len() && 0 <= j2 < dirs.len() && #[trigger] keep[j1] && #[trigger] keep[j2]
            && (dirs[j1]@ == dirs[i]@ || is_dir_ancestor(dirs[j1]@, dirs[i]@)) && (dirs[j2]@
            == dirs[i]@ || is_dir_ancestor(dirs[j2]@, dirs[i]@)) implies dirs[j1]@ == dirs[j2]@ by {
        if dirs[j1]@ != dirs[j2]@ {
            if dirs[j1]@ == dirs[i]@ {
                theorem_outermost_antichain(dirs, keep, j2, j1);
            } else if dirs[j2]@ == dirs[i]@ {
                theorem_outermost_antichain(dirs, keep, j1, j2);
            } else {
                lemma_common_descendant(dirs[j1]@, dirs[j2]@, dirs[i]@);
                theorem_outermost_antichain(dirs, keep, j1, j2);
                theorem_outermost_antichain(dirs, keep, j2, j1);
            }
        }
    }
}

/// `p` is one of the eligible paths flagged as remaining.
pub open spec fn in_remaining(eligible: Seq<&[u8]>, r: Seq<bool>, p: Seq<u8>) -> bool {
    exists|i: int| 0 <= i < eligible.len() && #[trigger] r[i] && eligible[i]@ == p
}

/// The partition: given covered paths that are all eligible and the flags
/// [`uncovered`] returns, the eligible paths are exactly the covered ones
/// plus the remaining ones, and no path is both.
pub proof fn theorem_partition(eligible: Seq<&[u8]>, covered: Seq<&[u8]>, r: Seq<bool>)
    requires
        r.len() == eligible.len(),
        forall|i: int| 0 <= i < eligible.len() ==> #[trigger] r[i] == !in_set(covered, eligible[i]@),
        forall|j: int| 0 <= j < covered.len() ==> in_set(eligible, (#[trigger] covered[j])@),
    ensures
        forall|p: Seq<u8>|
            #[trigger] in_set(eligible, p) <==> (in_set(covered, p) || in_remaining(eligible, r, p)),
        forall|p: Seq<u8>| !(#[trigger] in_set(covered, p) && in_remaining(eligible, r, p)),
{
    assert forall|p: Seq<u8>|
        #[trigger] in_set(eligible, p) <==> (in_set(covered, p) || in_remaining(eligible, r, p)) by {
        if in_set(eligible, p) && !in_set(covered, p) {
            let i = choose|i: int| 0 <= i < eligible.len() && (#[trigger] eligible[i])@ == p;
            assert(r[i]);
        }
        if in_set(covered, p) {
            let j = choose|j: int| 0 <= j < covered.len() && (#[trigger] covered[j])@ == p;
            assert(in_set(eligible, covered[j]@));
        }
    }
    assert forall|p: Seq<u8>| !(#[trigger] in_set(covered, p) && in_remaining(eligible, r, p)) by {
        if in_remaining(eligible, r, p) {
            let i = choose|i: int| 0 <= i < eligible.len() && #[trigger] r[i] && eligible[i]@ == p;
            assert(!in_set(covered, eligible[i]@));
        }
    }
}

} // verus!
