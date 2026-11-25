//! Logical (lexical) path normalization.
//!
//! [`logical_normalize`] resolves `.` and `..` without touching the
//! filesystem. Its specification `spec_normalize` is the classic stack
//! semantics, read left to right:
//!
//! - a prefix is kept;
//! - the root directory discards everything except prefixes, then is kept;
//! - `.` is dropped;
//! - `..` cancels a preceding name, and is otherwise kept (so `..` directly
//!   after the root is kept: `/../x` stays `/../x`, unlike POSIX, which
//!   would collapse it to `/x`; this is waft's long-standing behaviour and
//!   it fails closed, because the kept `..` makes containment fail);
//! - a name is kept.
//!
//! Proven here:
//!
//! - `logical_normalize` returns the strictly increasing indices of exactly
//!   the input components that `spec_normalize` keeps (so production can
//!   rebuild the normalized path from its own `std` components).
//! - `theorem_normalize_is_normal_form`: the result has no `.`, the root
//!   appears only after prefixes, and `..` never follows a name.
//! - `theorem_fixed_points_are_normal_forms`: normalization leaves a
//!   sequence unchanged exactly when it is in normal form.
//! - `theorem_normalize_idempotent`: normalizing twice is normalizing once.
//! - `theorem_normalize_prefix_first`: normalizing a leading part first
//!   does not change the result.
//! - `theorem_append_names`: appending plain names to a normal form is
//!   already normalized.
//! - `theorem_names_last`: for inputs shaped like `std` component lists
//!   (a prefix can only come first), once a name appears in the result only
//!   names follow.

#[allow(unused_imports)]
use vstd::prelude::*;

#[allow(unused_imports)]
use super::model::*;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

/// The elements of `s` that are prefixes, in order (`Vec::retain` with
/// `matches!(c, Component::Prefix(_))`).
pub open spec fn retain_prefixes(s: Seq<CompView>) -> Seq<CompView>
    decreases s.len(),
{
    if s.len() == 0 {
        s
    } else {
        let rest = retain_prefixes(s.drop_last());
        if s.last() is Prefix {
            rest.push(s.last())
        } else {
            rest
        }
    }
}

/// One step of the normalization stack machine: the stack `acc` after
/// reading component `c`.
pub open spec fn norm_step(acc: Seq<CompView>, c: CompView) -> Seq<CompView> {
    match c {
        CompView::Prefix(_) => acc.push(c),
        CompView::RootDir => retain_prefixes(acc).push(c),
        CompView::CurDir => acc,
        CompView::ParentDir => {
            if acc.len() > 0 && acc.last() is Normal {
                acc.drop_last()
            } else {
                acc.push(c)
            }
        },
        CompView::Normal(_) => acc.push(c),
    }
}

/// The stack after reading all of `cs`, starting from `acc`.
pub open spec fn normalize_from(acc: Seq<CompView>, cs: Seq<CompView>) -> Seq<CompView>
    decreases cs.len(),
{
    if cs.len() == 0 {
        acc
    } else {
        norm_step(normalize_from(acc, cs.drop_last()), cs.last())
    }
}

/// Logical normalization of a component sequence.
pub open spec fn spec_normalize(cs: Seq<CompView>) -> Seq<CompView> {
    normalize_from(Seq::empty(), cs)
}

/// Normal form: no `.`, the root only after prefixes, and no `..` directly
/// after a name.
pub open spec fn is_normal_form(s: Seq<CompView>) -> bool {
    &&& forall|i: int| 0 <= i < s.len() ==> !(#[trigger] s[i] is CurDir)
    &&& forall|i: int, j: int|
        #![trigger s[i], s[j]]
        0 <= j < i < s.len() && s[i] is RootDir ==> s[j] is Prefix
    &&& forall|i: int| 0 < i < s.len() && #[trigger] s[i] is ParentDir ==> !(s[i - 1] is Normal)
}

/// Every element of `s` is a name.
pub open spec fn all_normal(s: Seq<CompView>) -> bool {
    forall|i: int| 0 <= i < s.len() ==> #[trigger] s[i] is Normal
}

/// Every element of `s` is a prefix.
pub open spec fn all_prefix(s: Seq<CompView>) -> bool {
    forall|i: int| 0 <= i < s.len() ==> #[trigger] s[i] is Prefix
}

/// A prefix can only be the first element (true of every `Path::components`
/// list).
pub open spec fn prefix_first(s: Seq<CompView>) -> bool {
    forall|i: int| 0 <= i < s.len() && #[trigger] s[i] is Prefix ==> i == 0
}

/// Once a name appears, only names follow.
pub open spec fn names_last(s: Seq<CompView>) -> bool {
    forall|i: int, j: int|
        #![trigger s[i], s[j]]
        0 <= i < j < s.len() && s[i] is Normal ==> s[j] is Normal
}

// ---------------------------------------------------------------------------
// retain_prefixes
// ---------------------------------------------------------------------------

/// `retain_prefixes` keeps only prefixes and never grows.
pub proof fn lemma_retain_prefixes_all_prefix(s: Seq<CompView>)
    ensures
        all_prefix(retain_prefixes(s)),
        retain_prefixes(s).len() <= s.len(),
    decreases s.len(),
{
    if s.len() > 0 {
        lemma_retain_prefixes_all_prefix(s.drop_last());
    }
}

/// `retain_prefixes` is the identity on all-prefix sequences.
pub proof fn lemma_retain_prefixes_identity(s: Seq<CompView>)
    requires
        all_prefix(s),
    ensures
        retain_prefixes(s) == s,
    decreases s.len(),
{
    if s.len() > 0 {
        lemma_retain_prefixes_identity(s.drop_last());
        assert(s.drop_last().push(s.last()) =~= s);
    }
}

/// With at most one leading prefix, `retain_prefixes` keeps at most one.
pub proof fn lemma_retain_prefixes_prefix_first(s: Seq<CompView>)
    requires
        prefix_first(s),
    ensures
        retain_prefixes(s).len() <= 1,
    decreases s.len(),
{
    if s.len() > 0 {
        let init = s.drop_last();
        if s.last() is Prefix {
            assert(s[s.len() - 1] is Prefix);
            assert(init.len() == 0);
            assert(retain_prefixes(init).len() == 0);
        } else {
            assert(prefix_first(init)) by {
                assert forall|i: int| 0 <= i < init.len() && #[trigger] init[i] is Prefix implies i
                    == 0 by {
                    assert(s[i] == init[i]);
                }
            }
            lemma_retain_prefixes_prefix_first(init);
        }
    }
}

// ---------------------------------------------------------------------------
// Normal form
// ---------------------------------------------------------------------------

/// One step preserves normal form.
pub proof fn lemma_step_normal_form(acc: Seq<CompView>, c: CompView)
    requires
        is_normal_form(acc),
    ensures
        is_normal_form(norm_step(acc, c)),
{
    match c {
        CompView::RootDir => {
            lemma_retain_prefixes_all_prefix(acc);
        },
        _ => {},
    }
}

/// Reading any components from a normal-form stack yields a normal form.
pub proof fn lemma_normalize_from_normal_form(acc: Seq<CompView>, cs: Seq<CompView>)
    requires
        is_normal_form(acc),
    ensures
        is_normal_form(normalize_from(acc, cs)),
    decreases cs.len(),
{
    if cs.len() > 0 {
        lemma_normalize_from_normal_form(acc, cs.drop_last());
        lemma_step_normal_form(normalize_from(acc, cs.drop_last()), cs.last());
    }
}

/// The result of normalization is in normal form.
pub proof fn theorem_normalize_is_normal_form(cs: Seq<CompView>)
    ensures
        is_normal_form(spec_normalize(cs)),
{
    lemma_normalize_from_normal_form(Seq::empty(), cs);
}

/// A normal form is a fixed point of normalization.
pub proof fn lemma_normal_form_fixed_point(s: Seq<CompView>)
    requires
        is_normal_form(s),
    ensures
        spec_normalize(s) == s,
    decreases s.len(),
{
    if s.len() > 0 {
        let init = s.drop_last();
        let c = s.last();
        assert(is_normal_form(init)) by {
            assert forall|i: int, j: int|
                #![trigger init[i], init[j]]
                0 <= j < i < init.len() && init[i] is RootDir implies init[j] is Prefix by {
                assert(s[i] == init[i] && s[j] == init[j]);
            }
            assert forall|i: int|
                0 < i < init.len() && #[trigger] init[i] is ParentDir implies !(init[i
                    - 1] is Normal) by {
                assert(s[i] == init[i] && s[i - 1] == init[i - 1]);
            }
        }
        lemma_normal_form_fixed_point(init);
        assert(spec_normalize(s) == norm_step(init, c));
        assert(init.push(c) =~= s);
        assert(s[s.len() - 1] == c);
        match c {
            CompView::RootDir => {
                assert(all_prefix(init)) by {
                    assert forall|j: int| 0 <= j < init.len() implies #[trigger] init[j] is Prefix by {
                        assert(s[j] == init[j]);
                    }
                }
                lemma_retain_prefixes_identity(init);
            },
            CompView::ParentDir => {
                if init.len() > 0 {
                    assert(init.last() == s[s.len() - 2]);
                }
            },
            _ => {},
        }
    }
}

/// Normalization leaves `s` unchanged exactly when `s` is in normal form.
pub proof fn theorem_fixed_points_are_normal_forms(s: Seq<CompView>)
    ensures
        spec_normalize(s) == s <==> is_normal_form(s),
{
    theorem_normalize_is_normal_form(s);
    if is_normal_form(s) {
        lemma_normal_form_fixed_point(s);
    }
}

/// Normalization is idempotent.
pub proof fn theorem_normalize_idempotent(cs: Seq<CompView>)
    ensures
        spec_normalize(spec_normalize(cs)) == spec_normalize(cs),
{
    theorem_normalize_is_normal_form(cs);
    lemma_normal_form_fixed_point(spec_normalize(cs));
}

/// Reading `a + b` is reading `a`, then `b`.
pub proof fn lemma_normalize_from_concat(acc: Seq<CompView>, a: Seq<CompView>, b: Seq<CompView>)
    ensures
        normalize_from(acc, a + b) == normalize_from(normalize_from(acc, a), b),
    decreases b.len(),
{
    if b.len() == 0 {
        assert(a + b =~= a);
    } else {
        lemma_normalize_from_concat(acc, a, b.drop_last());
        assert((a + b).drop_last() =~= a + b.drop_last());
        assert((a + b).last() == b.last());
    }
}

/// Normalizing a leading part first does not change the result.
pub proof fn theorem_normalize_prefix_first(a: Seq<CompView>, b: Seq<CompView>)
    ensures
        spec_normalize(a + b) == spec_normalize(spec_normalize(a) + b),
{
    lemma_normalize_from_concat(Seq::empty(), a, b);
    lemma_normalize_from_concat(Seq::empty(), spec_normalize(a), b);
    theorem_normalize_idempotent(a);
}

/// Appending names to a normal form keeps it in normal form.
pub proof fn lemma_normal_form_append_names(s: Seq<CompView>, t: Seq<CompView>)
    requires
        is_normal_form(s),
        all_normal(t),
    ensures
        is_normal_form(s + t),
{
    let st = s + t;
    assert forall|i: int| 0 <= i < st.len() implies !(#[trigger] st[i] is CurDir) by {
        if i >= s.len() {
            assert(st[i] == t[i - s.len()]);
        }
    }
    assert forall|i: int, j: int|
        #![trigger st[i], st[j]]
        0 <= j < i < st.len() && st[i] is RootDir implies st[j] is Prefix by {
        if i >= s.len() {
            assert(st[i] == t[i - s.len()]);
        } else {
            assert(st[i] == s[i] && st[j] == s[j]);
        }
    }
    assert forall|i: int|
        0 < i < st.len() && #[trigger] st[i] is ParentDir implies !(st[i - 1] is Normal) by {
        if i >= s.len() {
            assert(st[i] == t[i - s.len()]);
        } else {
            assert(st[i] == s[i] && st[i - 1] == s[i - 1]);
        }
    }
}

/// Appending names to a normal form is already normalized: this is why a
/// repository path joined back onto its normalized root normalizes to
/// exactly that root followed by the path's names.
pub proof fn theorem_append_names(s: Seq<CompView>, t: Seq<CompView>)
    requires
        is_normal_form(s),
        all_normal(t),
    ensures
        spec_normalize(s + t) == s + t,
{
    lemma_normal_form_append_names(s, t);
    lemma_normal_form_fixed_point(s + t);
}

// ---------------------------------------------------------------------------
// Names come last
// ---------------------------------------------------------------------------

/// Normalization keeps a lone leading prefix leading.
pub proof fn lemma_normalize_prefix_first(cs: Seq<CompView>)
    requires
        prefix_first(cs),
    ensures
        prefix_first(spec_normalize(cs)),
    decreases cs.len(),
{
    if cs.len() > 0 {
        let init = cs.drop_last();
        let c = cs.last();
        assert(prefix_first(init)) by {
            assert forall|i: int| 0 <= i < init.len() && #[trigger] init[i] is Prefix implies i
                == 0 by {
                assert(cs[i] == init[i]);
            }
        }
        lemma_normalize_prefix_first(init);
        let n = spec_normalize(init);
        assert(spec_normalize(cs) == norm_step(n, c));
        match c {
            CompView::Prefix(_) => {
                assert(cs[cs.len() - 1] is Prefix);
                assert(init.len() == 0);
                assert(n =~= Seq::<CompView>::empty());
            },
            CompView::RootDir => {
                lemma_retain_prefixes_prefix_first(n);
                lemma_retain_prefixes_all_prefix(n);
            },
            _ => {},
        }
    }
}

/// In a normal form whose prefix (if any) comes first, everything after a
/// name is a name.
pub proof fn lemma_normal_form_names_last(s: Seq<CompView>)
    requires
        is_normal_form(s),
        prefix_first(s),
    ensures
        names_last(s),
{
    assert forall|i: int, j: int|
        #![trigger s[i], s[j]]
        0 <= i < j < s.len() && s[i] is Normal implies s[j] is Normal by {
        lemma_names_after(s, i, j);
    }
}

proof fn lemma_names_after(s: Seq<CompView>, i: int, j: int)
    requires
        is_normal_form(s),
        prefix_first(s),
        0 <= i < j < s.len(),
        s[i] is Normal,
    ensures
        s[j] is Normal,
    decreases j - i,
{
    if j - 1 > i {
        lemma_names_after(s, i, j - 1);
    }
    assert(s[j - 1] is Normal);
    // s[j] is not `.`, not the root (s[i] is a name before it), not `..`
    // (it follows a name), and not a prefix (it is not first).
    assert(!(s[j] is CurDir));
    assert(!(s[j] is RootDir)) by {
        if s[j] is RootDir {
            assert(s[i] is Prefix);
        }
    }
    assert(!(s[j] is ParentDir));
    assert(!(s[j] is Prefix));
}

/// For component lists shaped like `Path::components` output (a prefix can
/// only come first), the normalized result keeps any prefix first and has
/// nothing but names after its first name. With
/// `theorem_normalize_is_normal_form` (no `.`, the root only after
/// prefixes, no `..` after a name) the result is therefore: an optional
/// prefix, an optional root, then `..`s, then names.
pub proof fn theorem_names_last(cs: Seq<CompView>)
    requires
        prefix_first(cs),
    ensures
        names_last(spec_normalize(cs)),
        prefix_first(spec_normalize(cs)),
{
    theorem_normalize_is_normal_form(cs);
    lemma_normalize_prefix_first(cs);
    lemma_normal_form_names_last(spec_normalize(cs));
}

// ---------------------------------------------------------------------------
// Executable kernel
// ---------------------------------------------------------------------------

/// The components of `v` at positions `idx`.
pub open spec fn pick(v: Seq<CompView>, idx: Seq<usize>) -> Seq<CompView> {
    idx.map_values(|k: usize| v[k as int])
}

/// `idx` is strictly increasing and every entry is below `bound`.
pub open spec fn increasing_below(idx: Seq<usize>, bound: int) -> bool {
    &&& forall|k: int| 0 <= k < idx.len() ==> #[trigger] idx[k] < bound
    &&& forall|a: int, b: int| 0 <= a < b < idx.len() ==> #[trigger] idx[a] < #[trigger] idx[b]
}

/// Logically normalize `cs`, returning the positions of the components that
/// remain, in order.
///
/// The positions are strictly increasing, in bounds, and pick out exactly
/// `spec_normalize` of the input.
pub fn logical_normalize(cs: &[Comp]) -> (keep: Vec<usize>)
    ensures
        increasing_below(keep@, cs@.len() as int),
        pick(views(cs@), keep@) == spec_normalize(views(cs@)),
{
    let ghost v = views(cs@);
    let mut keep: Vec<usize> = Vec::new();
    let mut i: usize = 0;
    assert(pick(v, keep@) =~= spec_normalize(v.take(0)));
    while i < cs.len()
        invariant
            v == views(cs@),
            i <= cs@.len(),
            increasing_below(keep@, i as int),
            pick(v, keep@) == spec_normalize(v.take(i as int)),
        decreases cs.len() - i,
    {
        let ghost acc = pick(v, keep@);
        assert(v.take(i + 1).drop_last() =~= v.take(i as int));
        assert(v.take(i + 1).last() == v[i as int]);
        assert(spec_normalize(v.take(i + 1)) == norm_step(acc, v[i as int]));
        assert(v[i as int] == cs@[i as int]@);
        match cs[i] {
            Comp::Prefix(_) | Comp::Normal(_) => {
                keep.push(i);
                assert(pick(v, keep@) =~= acc.push(v[i as int]));
            },
            Comp::RootDir => {
                let mut kept: Vec<usize> = Vec::new();
                let mut j: usize = 0;
                assert(pick(v, kept@) =~= retain_prefixes(acc.take(0)));
                while j < keep.len()
                    invariant
                        v == views(cs@),
                        i < cs@.len(),
                        j <= keep@.len(),
                        increasing_below(keep@, i as int),
                        acc == pick(v, keep@),
                        increasing_below(kept@, i as int),
                        j == 0 ==> kept@.len() == 0,
                        j > 0 && kept@.len() > 0 ==> kept@.last() <= keep@[j - 1],
                        pick(v, kept@) == retain_prefixes(acc.take(j as int)),
                    decreases keep.len() - j,
                {
                    let k = keep[j];
                    let ghost before = pick(v, kept@);
                    assert(acc.take(j + 1).drop_last() =~= acc.take(j as int));
                    assert(acc.take(j + 1).last() == v[k as int]);
                    assert(v[k as int] == cs@[k as int]@);
                    if is_prefix(&cs[k]) {
                        kept.push(k);
                        assert(pick(v, kept@) =~= before.push(v[k as int]));
                    }
                    j += 1;
                }
                assert(acc.take(keep@.len() as int) =~= acc);
                let ghost before = pick(v, kept@);
                kept.push(i);
                assert(pick(v, kept@) =~= before.push(v[i as int]));
                keep = kept;
            },
            Comp::CurDir => {},
            Comp::ParentDir => {
                let len = keep.len();
                if len > 0 && is_normal(&cs[keep[len - 1]]) {
                    assert(acc.last() == v[keep@[len - 1] as int]);
                    assert(v[keep@[len - 1] as int] == cs@[keep@[len - 1] as int]@);
                    keep.pop();
                    assert(pick(v, keep@) =~= acc.drop_last());
                } else {
                    if len > 0 {
                        assert(acc.last() == v[keep@[len - 1] as int]);
                        assert(v[keep@[len - 1] as int] == cs@[keep@[len - 1] as int]@);
                    }
                    keep.push(i);
                    assert(pick(v, keep@) =~= acc.push(v[i as int]));
                }
            },
        }
        i += 1;
    }
    assert(v.take(cs@.len() as int) =~= v);
    keep
}

} // verus!
