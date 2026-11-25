//! Verified kernel: map a path in any worktree of the family to the source
//! worktree.
//!
//! # What it decides
//!
//! `waft info` arguments and the directory project-config discovery starts
//! from are given relative to whichever worktree the user is in. Production
//! (`Cli::map_worktree_path_to_source`) re-roots such a path at the source
//! worktree by stripping the *deepest* known worktree that is a
//! component-wise prefix of it. [`deepest_worktree_prefix`] makes that
//! choice over component lists: every path is lowered to its sequence of
//! components, and each component to an injective byte key, so component
//! equality is byte-sequence equality.
//!
//! When several worktrees of the same depth match (production's
//! `Iterator::max_by_key` keeps the last of them), the kernel also returns
//! the last one, although the choice cannot matter: see
//! `lemma_equal_depth_prefixes_are_equal`.
//!
//! # Theorems
//!
//! - [`deepest_worktree_prefix`]'s postcondition: `None` exactly when no
//!   worktree is a component prefix of the path; otherwise the returned
//!   worktree is a component prefix of the path, no matching worktree is
//!   deeper, and no later matching worktree is as deep (the
//!   `max_by_key` tie-break), i.e. `is_chosen`.
//! - `lemma_equal_depth_prefixes_are_equal`: two component prefixes of the
//!   same path with the same depth are the same component sequence.
//! - `theorem_choice_is_order_independent`: for any two worktree lists
//!   with the same members (any reordering, or with duplicates), the deepest
//!   matches are the same component sequence, so the remainder that is
//!   re-rooted at the source worktree is the same. The mapping does not
//!   depend on the order `git worktree list` reports worktrees in.
//! - `theorem_deepest_containing_worktree_wins`: if some worktree `w`
//!   contains the path and no worktree is a strictly deeper prefix, the
//!   mapping strips exactly `w` (for example, a path inside the source
//!   worktree maps to itself when no nested worktree contains it).
//!
//! # Trust assumptions
//!
//! - Production lowers `Path::components()` faithfully: one key per
//!   component, and two keys are equal exactly when the components are equal
//!   (`Component` equality, which compares parsed prefixes and the bytes of
//!   normal components).
//! - `Path::strip_prefix(base)` succeeds exactly when `base`'s components
//!   are a prefix of the path's components (the documented semantics), and
//!   production re-runs it on the chosen worktree to obtain the remainder
//!   spelled exactly as std returns it.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// A component list as byte sequences.
pub open spec fn comps(c: Seq<Vec<u8>>) -> Seq<Seq<u8>> {
    c.map_values(|x: Vec<u8>| x@)
}

/// Each worktree's component list as byte sequences.
pub open spec fn worktree_comps(w: Seq<Vec<Vec<u8>>>) -> Seq<Seq<Seq<u8>>> {
    w.map_values(|x: Vec<Vec<u8>>| comps(x@))
}

/// `base` is a component-wise prefix of `path` (`path.strip_prefix(base)`
/// succeeds).
pub open spec fn is_component_prefix(base: Seq<Seq<u8>>, path: Seq<Seq<u8>>) -> bool {
    base.len() <= path.len() && path.subrange(0, base.len() as int) == base
}

/// Worktree `j` contains `path`.
pub open spec fn matches_at(path: Seq<Seq<u8>>, wts: Seq<Seq<Seq<u8>>>, j: int) -> bool {
    0 <= j < wts.len() && is_component_prefix(wts[j], path)
}

/// Worktree `i` contains `path` and no worktree containing it is deeper.
pub open spec fn is_deepest(path: Seq<Seq<u8>>, wts: Seq<Seq<Seq<u8>>>, i: int) -> bool {
    &&& matches_at(path, wts, i)
    &&& forall|j: int| #[trigger] matches_at(path, wts, j) ==> wts[j].len() <= wts[i].len()
}

/// Worktree `i` is the deepest one containing `path`, and the last such one
/// (`Iterator::max_by_key` keeps the last maximum).
pub open spec fn is_chosen(path: Seq<Seq<u8>>, wts: Seq<Seq<Seq<u8>>>, i: int) -> bool {
    &&& is_deepest(path, wts, i)
    &&& forall|j: int| i < j && #[trigger] matches_at(path, wts, j) ==> wts[j].len() < wts[i].len()
}

/// Whether `base` is a component prefix of `path`.
fn component_prefix(base: &[Vec<u8>], path: &[Vec<u8>]) -> (r: bool)
    ensures
        r == is_component_prefix(comps(base@), comps(path@)),
{
    if base.len() > path.len() {
        return false;
    }
    let mut k: usize = 0;
    while k < base.len()
        invariant
            base@.len() <= path@.len(),
            k <= base@.len(),
            forall|m: int| 0 <= m < k ==> (#[trigger] base@[m])@ == path@[m]@,
        decreases base.len() - k,
    {
        if base[k].as_slice() != path[k].as_slice() {
            assert(comps(path@).subrange(0, base@.len() as int)[k as int] != comps(base@)[k as int]);
            return false;
        }
        assert(base@[k as int]@ =~= path@[k as int]@);
        k += 1;
    }
    assert(comps(path@).subrange(0, base@.len() as int) =~= comps(base@));
    true
}

/// The index of the deepest worktree whose components prefix `path`'s
/// (the last one among equally deep matches), or `None` when no worktree
/// contains `path`.
pub fn deepest_worktree_prefix(path: &[Vec<u8>], worktrees: &[Vec<Vec<u8>>]) -> (r: Option<
    usize,
>)
    ensures
        r is None <==> forall|j: int| !#[trigger] matches_at(comps(path@), worktree_comps(worktrees@), j),
        r matches Some(i) ==> is_chosen(comps(path@), worktree_comps(worktrees@), i as int),
{
    let ghost p = comps(path@);
    let ghost w = worktree_comps(worktrees@);
    let mut best: Option<usize> = None;
    let mut best_len: usize = 0;
    let mut i: usize = 0;
    while i < worktrees.len()
        invariant
            i <= worktrees@.len(),
            p == comps(path@),
            w == worktree_comps(worktrees@),
            best is None ==> forall|j: int| 0 <= j < i ==> !#[trigger] matches_at(p, w, j),
            best matches Some(b) ==> {
                &&& b < i
                &&& matches_at(p, w, b as int)
                &&& best_len == w[b as int].len()
                &&& forall|j: int|
                    0 <= j < i && #[trigger] matches_at(p, w, j) ==> w[j].len() <= best_len
                &&& forall|j: int|
                    b < j < i && #[trigger] matches_at(p, w, j) ==> w[j].len() < best_len
            },
        decreases worktrees.len() - i,
    {
        let candidate: &[Vec<u8>] = worktrees[i].as_slice();
        assert(w[i as int] == comps(worktrees@[i as int]@));
        if component_prefix(candidate, path) {
            let depth = candidate.len();
            match best {
                Some(_) => {
                    if depth >= best_len {
                        best = Some(i);
                        best_len = depth;
                    }
                },
                None => {
                    best = Some(i);
                    best_len = depth;
                },
            }
        }
        i += 1;
    }
    best
}

/// Two component prefixes of the same path with the same depth are equal.
pub proof fn lemma_equal_depth_prefixes_are_equal(
    path: Seq<Seq<u8>>,
    a: Seq<Seq<u8>>,
    b: Seq<Seq<u8>>,
)
    requires
        is_component_prefix(a, path),
        is_component_prefix(b, path),
        a.len() == b.len(),
    ensures
        a == b,
{
}

/// The chosen worktree's components, and therefore the remainder re-rooted
/// at the source worktree, do not depend on the order (or multiplicity) of
/// the worktree list: any deepest match in one list equals any deepest match
/// in another list with the same members.
pub proof fn theorem_choice_is_order_independent(
    path: Seq<Seq<u8>>,
    a: Seq<Seq<Seq<u8>>>,
    b: Seq<Seq<Seq<u8>>>,
    i: int,
    j: int,
)
    requires
        forall|x: Seq<Seq<u8>>| a.contains(x) <==> b.contains(x),
        is_deepest(path, a, i),
        is_deepest(path, b, j),
    ensures
        a[i] == b[j],
        path.subrange(a[i].len() as int, path.len() as int) == path.subrange(
            b[j].len() as int,
            path.len() as int,
        ),
{
    assert(a.contains(a[i]));
    assert(b.contains(b[j]));
    let jb = choose|k: int| 0 <= k < b.len() && b[k] == a[i];
    let ia = choose|k: int| 0 <= k < a.len() && a[k] == b[j];
    assert(matches_at(path, b, jb));
    assert(matches_at(path, a, ia));
    lemma_equal_depth_prefixes_are_equal(path, a[i], b[j]);
}

/// If worktree `k` contains the path and no worktree is a strictly deeper
/// prefix, the chosen worktree is `k`'s component sequence.
pub proof fn theorem_deepest_containing_worktree_wins(
    path: Seq<Seq<u8>>,
    wts: Seq<Seq<Seq<u8>>>,
    k: int,
    i: int,
)
    requires
        matches_at(path, wts, k),
        forall|j: int| #[trigger] matches_at(path, wts, j) ==> wts[j].len() <= wts[k].len(),
        is_chosen(path, wts, i),
    ensures
        wts[i] == wts[k],
        path == wts[k] + path.subrange(wts[k].len() as int, path.len() as int),
{
    lemma_equal_depth_prefixes_are_equal(path, wts[i], wts[k]);
    assert(path =~= wts[k] + path.subrange(wts[k].len() as int, path.len() as int));
}

} // verus!
