//! Component-wise containment of a normalized path under a normalized root.
//!
//! [`classify_containment`] replaces `Path::strip_prefix`, the empty-remainder
//! check, and the "only names may remain" loop of `RepoRelPath::normalize`.
//! Its specification `spec_containment` says:
//!
//! - **Outside** when the root is not a component-wise prefix of the path,
//!   or when something other than a name remains after it (a remaining `..`
//!   climbs out of the root, or a root directory or prefix follows a
//!   repository root that is empty or only a prefix). For
//!   `Path::components`-shaped paths this second case is possible exactly
//!   when the normalized root has no name at all: `/`, `C:\`, `""`, `.`, or
//!   a root that normalizes to only `..`s after those, such as `..`, `/..`
//!   or `/repo/../..`. Production used to hit `unreachable!` here;
//! - **RepoRoot** when the path is the root itself;
//! - **Inside(n)** otherwise: the first `n` components are the root and the
//!   rest, which is non-empty, consists of names only.
//!
//! Proven here:
//!
//! - `theorem_inside_is_name_extension`: an `Inside(n)` decision means the
//!   path is exactly the root followed by a non-empty list of names (no `.`,
//!   no `..`, no root, no prefix).
//! - `theorem_classification_is_exact`: `RepoRoot` exactly when the two
//!   sequences are equal, and `Inside` exactly when the root is a proper
//!   component prefix followed by names only.
//! - `theorem_normalized_containment`: for normalized inputs, the
//!   normalized root followed by the result is the normalized path.
//! - `theorem_round_trip`: joining a result back onto the root and
//!   classifying again gives the same result.
//! - `theorem_escape_needs_nameless_root`: if the normalized root contains
//!   a name, a path under it never has a non-name remainder, so the
//!   escape branch is unreachable for ordinary repository roots.
//! - `theorem_nameless_root_escapes`: conversely, every root whose
//!   normalized form has no name reaches the escape branch (with the path
//!   `root/..`). So the branch is reachable exactly for nameless roots.
//! - `theorem_escape_is_leading`: whenever a non-name remains, it is the
//!   very first remaining component, so production's former loop would have
//!   panicked before converting any name to UTF-8. Reporting "outside the
//!   repository root" instead is therefore the only behaviour change.
//! - `example_escape_above_filesystem_root`: the reproduction of the former
//!   panic (`/../x` against the root `/`) is classified Outside.

#[allow(unused_imports)]
use vstd::prelude::*;

#[allow(unused_imports)]
use super::model::*;
#[allow(unused_imports)]
use super::normalize::*;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

/// Where a path lies relative to a repository root.
#[derive(Debug)]
pub enum Containment {
    /// Not under the root (or climbing out of it).
    Outside,
    /// The root itself.
    RepoRoot,
    /// Strictly under the root: the components from this index on are the
    /// repository-relative names.
    Inside(usize),
}

/// `root` is a component-wise prefix of `abs` (`Path::strip_prefix`).
pub open spec fn is_component_prefix(root: Seq<CompView>, abs: Seq<CompView>) -> bool {
    root.len() <= abs.len() && abs.take(root.len() as int) == root
}

/// The containment decision for (already normalized) `abs` under `root`.
pub open spec fn spec_containment(abs: Seq<CompView>, root: Seq<CompView>) -> Containment {
    if !is_component_prefix(root, abs) {
        Containment::Outside
    } else if abs.len() == root.len() {
        Containment::RepoRoot
    } else if all_normal(abs.skip(root.len() as int)) {
        Containment::Inside(root.len() as usize)
    } else {
        Containment::Outside
    }
}

/// The decision `RepoRelPath::normalize` makes for raw component lists: both
/// are normalized, then compared.
pub open spec fn spec_repo_relative(abs: Seq<CompView>, root: Seq<CompView>) -> Containment {
    spec_containment(spec_normalize(abs), spec_normalize(root))
}

/// Classify `abs` against `root` component by component.
pub fn classify_containment(abs: &[Comp], root: &[Comp]) -> (r: Containment)
    ensures
        r == spec_containment(views(abs@), views(root@)),
        r matches Containment::Inside(n) ==> n == root@.len(),
{
    let ghost a = views(abs@);
    let ghost rt = views(root@);
    if root.len() > abs.len() {
        return Containment::Outside;
    }
    let mut j: usize = 0;
    while j < root.len()
        invariant
            a == views(abs@),
            rt == views(root@),
            j <= root@.len() <= abs@.len(),
            forall|k: int| 0 <= k < j ==> a[k] == rt[k],
        decreases root.len() - j,
    {
        if !comp_eq(&abs[j], &root[j]) {
            assert(a.take(rt.len() as int)[j as int] != rt[j as int]);
            return Containment::Outside;
        }
        j += 1;
    }
    assert(a.take(rt.len() as int) =~= rt);
    if abs.len() == root.len() {
        return Containment::RepoRoot;
    }
    let start = root.len();
    let mut k: usize = start;
    while k < abs.len()
        invariant
            a == views(abs@),
            rt == views(root@),
            start == rt.len(),
            start < abs@.len(),
            a.take(start as int) == rt,
            start <= k <= abs@.len(),
            forall|m: int| start <= m < k ==> #[trigger] a[m] is Normal,
        decreases abs.len() - k,
    {
        if !is_normal(&abs[k]) {
            assert(a[k as int] == abs@[k as int]@);
            assert(a.skip(start as int)[k - start] == a[k as int]);
            assert(!all_normal(a.skip(start as int)));
            return Containment::Outside;
        }
        k += 1;
    }
    assert(all_normal(a.skip(start as int))) by {
        assert forall|m: int| 0 <= m < a.skip(start as int).len() implies #[trigger] a.skip(
            start as int,
        )[m] is Normal by {
            assert(a.skip(start as int)[m] == a[start + m]);
        }
    }
    Containment::Inside(start)
}

// ---------------------------------------------------------------------------
// Theorems about the decision
// ---------------------------------------------------------------------------

/// An `Inside` decision means: at least one component remains after the
/// root, the root followed by that remainder is the path, and every
/// remaining component is a name (no `.`, `..`, root or prefix).
/// (`classify_containment` reports the remainder's start, `root.len()`.)
pub proof fn theorem_inside_is_name_extension(abs: Seq<CompView>, root: Seq<CompView>)
    ensures
        spec_containment(abs, root) is Inside ==> {
            let rel = abs.skip(root.len() as int);
            &&& rel.len() > 0
            &&& root + rel == abs
            &&& all_normal(rel)
            &&& forall|i: int|
                0 <= i < rel.len() ==> !(#[trigger] rel[i] is CurDir) && !(rel[i] is ParentDir)
                    && !(rel[i] is RootDir) && !(rel[i] is Prefix)
        },
{
    if spec_containment(abs, root) is Inside {
        let n = root.len();
        assert(abs.take(n as int) + abs.skip(n as int) =~= abs);
    }
}

/// The decision is exact: `RepoRoot` iff the sequences are equal, and
/// `Inside` iff the root is a proper component prefix followed only by
/// names. In particular the decision is `Inside` only with a non-empty
/// remainder.
pub proof fn theorem_classification_is_exact(abs: Seq<CompView>, root: Seq<CompView>)
    ensures
        (spec_containment(abs, root) is RepoRoot) <==> abs == root,
        (spec_containment(abs, root) is Inside) <==> (is_component_prefix(root, abs) && abs.len()
            > root.len() && all_normal(abs.skip(root.len() as int))),
        (spec_containment(abs, root) is Outside) <==> (!is_component_prefix(root, abs) || (
        abs.len() > root.len() && !all_normal(abs.skip(root.len() as int)))),
{
    if abs == root {
        assert(abs.take(root.len() as int) =~= root);
    }
    if is_component_prefix(root, abs) && abs.len() == root.len() {
        assert(abs =~= abs.take(root.len() as int));
    }
}

/// For raw component lists, a successful decision splits the *normalized*
/// path into the normalized root followed by non-empty names.
pub proof fn theorem_normalized_containment(abs: Seq<CompView>, root: Seq<CompView>)
    ensures
        spec_repo_relative(abs, root) is Inside ==> {
            let rel = spec_normalize(abs).skip(spec_normalize(root).len() as int);
            &&& spec_normalize(root) + rel == spec_normalize(abs)
            &&& rel.len() > 0
            &&& all_normal(rel)
        },
{
    theorem_inside_is_name_extension(spec_normalize(abs), spec_normalize(root));
}

/// Joining non-empty names `rel` onto any root and classifying the result
/// against that root yields `Inside`, and the remainder is exactly `rel`.
/// With `theorem_normalized_containment` this is the component-level form
/// of `normalize(p.to_path(root), root) == Ok(p)`.
pub proof fn theorem_round_trip(root: Seq<CompView>, rel: Seq<CompView>)
    requires
        rel.len() > 0,
        all_normal(rel),
    ensures
        spec_repo_relative(root + rel, root) == Containment::Inside(
            spec_normalize(root).len() as usize,
        ),
        spec_normalize(root + rel).skip(spec_normalize(root).len() as int) == rel,
{
    let nr = spec_normalize(root);
    theorem_normalize_prefix_first(root, rel);
    theorem_normalize_is_normal_form(root);
    theorem_append_names(nr, rel);
    let na = spec_normalize(root + rel);
    assert(na == nr + rel);
    assert(na.take(nr.len() as int) =~= nr);
    assert(na.skip(nr.len() as int) =~= rel);
}

/// If the normalized root contains a name and is a component prefix of the
/// normalized path, everything after it is a name. So the "non-name
/// remains" branch (formerly `unreachable!`) can only be taken for a root
/// whose normalized form has no name: a bare filesystem root, drive or
/// prefix, an empty or `.` root, or a root that normalizes to only `..`s
/// after those (`..`, `/..`, `/repo/../..`). `theorem_nameless_root_escapes`
/// shows that every such root does reach the branch.
/// Requires that a prefix component can only come first in the path's
/// component list, which `Path::components` guarantees.
pub proof fn theorem_escape_needs_nameless_root(abs: Seq<CompView>, root: Seq<CompView>, k: int)
    requires
        prefix_first(abs),
        0 <= k < spec_normalize(root).len(),
        spec_normalize(root)[k] is Normal,
        is_component_prefix(spec_normalize(root), spec_normalize(abs)),
    ensures
        all_normal(spec_normalize(abs).skip(spec_normalize(root).len() as int)),
        !(spec_repo_relative(abs, root) is Outside),
{
    let na = spec_normalize(abs);
    let nr = spec_normalize(root);
    theorem_names_last(abs);
    assert(na[k] == nr[k]) by {
        assert(na.take(nr.len() as int)[k] == na[k]);
    }
    let rel = na.skip(nr.len() as int);
    assert forall|i: int| 0 <= i < rel.len() implies #[trigger] rel[i] is Normal by {
        assert(rel[i] == na[nr.len() + i]);
        assert(na[k] is Normal);
    }
}

/// The converse of `theorem_escape_needs_nameless_root`: when the normalized
/// root has no name, the root followed by `..` normalizes to the normalized
/// root followed by `..`. That starts with the normalized root and leaves
/// the non-name `..`, so the decision is Outside through the "non-name
/// remains" branch. The witness path is `Path::components`-shaped whenever
/// the root is. Together the two theorems say the formerly `unreachable!`
/// branch is reachable exactly for roots whose normalized form has no name.
pub proof fn theorem_nameless_root_escapes(root: Seq<CompView>)
    requires
        forall|k: int|
            0 <= k < spec_normalize(root).len() ==> !(#[trigger] spec_normalize(root)[k] is Normal),
    ensures
        ({
            let abs = root.push(CompView::ParentDir);
            let na = spec_normalize(abs);
            let nr = spec_normalize(root);
            &&& prefix_first(root) ==> prefix_first(abs)
            &&& na == nr.push(CompView::ParentDir)
            &&& is_component_prefix(nr, na)
            &&& na.len() > nr.len()
            &&& !(na[nr.len() as int] is Normal)
            &&& !all_normal(na.skip(nr.len() as int))
            &&& spec_repo_relative(abs, root) is Outside
        }),
{
    let abs = root.push(CompView::ParentDir);
    let nr = spec_normalize(root);
    assert(abs.drop_last() =~= root);
    assert(spec_normalize(abs) == norm_step(nr, CompView::ParentDir));
    if nr.len() > 0 {
        assert(!(nr[nr.len() - 1] is Normal));
    }
    let na = spec_normalize(abs);
    assert(na == nr.push(CompView::ParentDir));
    assert(na.take(nr.len() as int) =~= nr);
    assert(na.skip(nr.len() as int)[0] == na[nr.len() as int]);
    if prefix_first(root) {
        assert forall|i: int| 0 <= i < abs.len() && #[trigger] abs[i] is Prefix implies i == 0 by {
            if i < root.len() {
                assert(abs[i] == root[i]);
            }
        }
    }
}

/// When the path's components are names-last (true of every normalized
/// `Path::components` list, by `theorem_names_last`) and a non-name
/// remains after the root, the first remaining component is that non-name.
/// Production's former loop therefore reached `unreachable!` before
/// converting any remaining name to UTF-8, so classifying the case as
/// Outside changes nothing but the panic.
pub proof fn theorem_escape_is_leading(abs: Seq<CompView>, root: Seq<CompView>)
    requires
        names_last(abs),
        is_component_prefix(root, abs),
        abs.len() > root.len(),
        !all_normal(abs.skip(root.len() as int)),
    ensures
        !(abs[root.len() as int] is Normal),
        spec_containment(abs, root) is Outside,
{
    let rel = abs.skip(root.len() as int);
    let m = choose|m: int| 0 <= m < rel.len() && !(#[trigger] rel[m] is Normal);
    assert(rel[m] == abs[root.len() + m]);
    if abs[root.len() as int] is Normal && m > 0 {
        assert(abs[root.len() + m] is Normal);
    }
}

/// The reproduction of the former panic: `RepoRelPath::normalize("/../x",
/// "/")` (or `"../x"` joined onto `/`). The normalized path is `/`, `..`,
/// `x`; it starts with the root `/`, and the remainder starts with `..`.
/// Production used to reach `unreachable!`; the kernel decides Outside.
pub proof fn example_escape_above_filesystem_root(x: Seq<u8>)
    ensures
        ({
            let abs = seq![CompView::RootDir, CompView::ParentDir, CompView::Normal(x)];
            let root = seq![CompView::RootDir];
            &&& spec_normalize(abs) == abs
            &&& spec_normalize(root) == root
            &&& spec_normalize(abs).skip(1)[0] is ParentDir
            &&& spec_repo_relative(abs, root) is Outside
        }),
{
    let abs = seq![CompView::RootDir, CompView::ParentDir, CompView::Normal(x)];
    let root = seq![CompView::RootDir];
    assert(is_normal_form(abs));
    assert(is_normal_form(root));
    lemma_normal_form_fixed_point(abs);
    lemma_normal_form_fixed_point(root);
    assert(abs.take(1) =~= root);
    assert(abs.skip(1)[0] is ParentDir);
}

} // verus!
