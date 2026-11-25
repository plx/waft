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
//! choice over component lists in the path kernel's model
//! ([`crate::path::model::Comp`]): production lowers every path with the
//! same conversion `RepoRelPath::normalize` uses (`kernel_path_components`
//! in `src/path.rs`), and "worktree `w` contains the path" is the path
//! kernel's own component prefix, `containment::is_component_prefix` of the
//! [`CompView`] views, decided by the shared
//! [`crate::path::containment::component_prefix`] with
//! [`crate::path::model::comp_eq`] (proven to be view equality).
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
//! - `theorem_match_agrees_with_containment`: a worktree contains the path
//!   with only names after it exactly when the repository-containment
//!   decision `spec_containment(path, worktree)` is not `Outside`, and an
//!   `Inside(n)` decision strips the same `n` components the mapping does.
//!   The mapping alone also accepts a remainder with `..` or `.`, because
//!   `strip_prefix` does not normalize it.
//!
//! # Trust assumptions
//!
//! - Production lowers `Path::components()` faithfully, one `Comp` per
//!   component, with the conversion the path kernel already trusts (see
//!   `crate::path`'s trust assumptions): kernel component equality is
//!   `std::path::Component` equality.
//! - `Path::strip_prefix(base)` succeeds exactly when `base`'s components
//!   are a prefix of the path's components (the documented semantics), and
//!   production re-runs it on the chosen worktree to obtain the remainder
//!   spelled exactly as std returns it.

#[allow(unused_imports)]
use vstd::prelude::*;

#[allow(unused_imports)]
use crate::path::containment::*;
#[allow(unused_imports)]
use crate::path::model::*;
#[cfg(verus_keep_ghost)]
use crate::path::normalize::all_normal;

verus! {

/// Each worktree's component list as views.
pub open spec fn worktree_views<'a>(w: Seq<Vec<Comp<'a>>>) -> Seq<Seq<CompView>> {
    w.map_values(|x: Vec<Comp<'a>>| views(x@))
}

/// Worktree `j` contains `path`.
pub open spec fn matches_at(path: Seq<CompView>, wts: Seq<Seq<CompView>>, j: int) -> bool {
    0 <= j < wts.len() && is_component_prefix(wts[j], path)
}

/// Worktree `i` contains `path` and no worktree containing it is deeper.
pub open spec fn is_deepest(path: Seq<CompView>, wts: Seq<Seq<CompView>>, i: int) -> bool {
    &&& matches_at(path, wts, i)
    &&& forall|j: int| #[trigger] matches_at(path, wts, j) ==> wts[j].len() <= wts[i].len()
}

/// Worktree `i` is the deepest one containing `path`, and the last such one
/// (`Iterator::max_by_key` keeps the last maximum).
pub open spec fn is_chosen(path: Seq<CompView>, wts: Seq<Seq<CompView>>, i: int) -> bool {
    &&& is_deepest(path, wts, i)
    &&& forall|j: int| i < j && #[trigger] matches_at(path, wts, j) ==> wts[j].len() < wts[i].len()
}

/// The index of the deepest worktree whose components prefix `path`'s
/// (the last one among equally deep matches), or `None` when no worktree
/// contains `path`.
pub fn deepest_worktree_prefix(path: &[Comp], worktrees: &[Vec<Comp>]) -> (r: Option<usize>)
    ensures
        r is None <==> forall|j: int|
            !#[trigger] matches_at(views(path@), worktree_views(worktrees@), j),
        r matches Some(i) ==> is_chosen(views(path@), worktree_views(worktrees@), i as int),
{
    let ghost p = views(path@);
    let ghost w = worktree_views(worktrees@);
    let mut best: Option<usize> = None;
    let mut best_len: usize = 0;
    let mut i: usize = 0;
    while i < worktrees.len()
        invariant
            i <= worktrees@.len(),
            p == views(path@),
            w == worktree_views(worktrees@),
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
        let candidate: &[Comp] = worktrees[i].as_slice();
        assert(w[i as int] == views(worktrees@[i as int]@));
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
    path: Seq<CompView>,
    a: Seq<CompView>,
    b: Seq<CompView>,
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
    path: Seq<CompView>,
    a: Seq<Seq<CompView>>,
    b: Seq<Seq<CompView>>,
    i: int,
    j: int,
)
    requires
        forall|x: Seq<CompView>| a.contains(x) <==> b.contains(x),
        is_deepest(path, a, i),
        is_deepest(path, b, j),
    ensures
        a[i] == b[j],
        path.skip(a[i].len() as int) == path.skip(b[j].len() as int),
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
    path: Seq<CompView>,
    wts: Seq<Seq<CompView>>,
    k: int,
    i: int,
)
    requires
        matches_at(path, wts, k),
        forall|j: int| #[trigger] matches_at(path, wts, j) ==> wts[j].len() <= wts[k].len(),
        is_chosen(path, wts, i),
    ensures
        wts[i] == wts[k],
        path == wts[k] + path.skip(wts[k].len() as int),
{
    lemma_equal_depth_prefixes_are_equal(path, wts[i], wts[k]);
    assert(path =~= wts[k] + path.skip(wts[k].len() as int));
}

/// The mapping and the repository-containment decision share one notion of
/// "under": worktree `j` contains the path with only names after it exactly
/// when `spec_containment(path, wts[j])` (the decision
/// `classify_containment` makes) is not `Outside`, and an `Inside(n)`
/// decision strips the same `n` components the mapping strips. The mapping
/// additionally accepts a remainder that is not all names (for example one
/// starting with `..`), which `strip_prefix` returns unnormalized.
pub proof fn theorem_match_agrees_with_containment(
    path: Seq<CompView>,
    wts: Seq<Seq<CompView>>,
    j: int,
)
    requires
        0 <= j < wts.len(),
    ensures
        !(spec_containment(path, wts[j]) is Outside) <==> (matches_at(path, wts, j) && all_normal(
            path.skip(wts[j].len() as int),
        )),
        spec_containment(path, wts[j]) is RepoRoot <==> (matches_at(path, wts, j) && path.len()
            == wts[j].len()),
        spec_containment(path, wts[j]) matches Containment::Inside(n) ==> n == wts[j].len() as usize
            && matches_at(path, wts, j) && path.len() > wts[j].len(),
{
    theorem_classification_is_exact(path, wts[j]);
    if matches_at(path, wts, j) && path.len() == wts[j].len() {
        assert(path.skip(wts[j].len() as int).len() == 0);
    }
}

} // verus!
