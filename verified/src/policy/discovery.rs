//! Project-config discovery boundary: which directories between the source
//! root and the invocation directory are searched for `.waft.toml`.
//!
//! # What it decides
//!
//! Production `discover_project_configs_in_repo` always searches the source
//! root, and, when the invocation directory is under it, lowers the
//! remaining path components to [`ComponentKind`] and lets
//! [`walk_toward_invocation`] drive the walk through the [`DiscoveryWalk`]
//! oracle: `enter(i)` descends into name component `i` and reports whether
//! that directory is a nested checkout (`.git` exists) or a registered
//! gitlink; `search(i)` records it as searched.
//!
//! # Theorems
//!
//! - `walk_toward_invocation`: the calls made are exactly
//!   `walk_outcome(reachable_names(kinds, 0), m)` for some `m`: reachable
//!   names are entered in order, each non-boundary is searched right after
//!   it is entered, the first boundary ends the walk without being searched,
//!   and with no boundary every reachable name is searched.
//! - `theorem_reachable_names`: the reachable names are in-bounds `Normal`
//!   components, strictly increasing, with no `..`/root/prefix component at
//!   or before them, and every name with no such component before it is
//!   reachable.
//!
//! Together: no directory outside the walk from the root toward the
//! invocation directory, and nothing at or below a nested checkout or
//! gitlink, is ever searched; each directory is probed at most once, in
//! order, and probing stops at the first boundary (the same filesystem
//! queries, in the same order, as before).
//!
//! # Trust
//!
//! The oracle records each call in its ghost log; its boundary answers are
//! arbitrary. Production's `Path::strip_prefix`/`components` lowering, the
//! `.git` existence probe, `RepoRelPath::normalize` and
//! `git::repo_paths_equivalent` (Unicode case folding), the final
//! `.waft.toml` join and the symlink-metadata filter stay in production.
//!
//! Quirk (current behavior): a `..`, root or prefix component ends the walk
//! but keeps the directories already searched, so `a/../b` searches the
//! root and `a` (callers pass normalized paths, so this is not reached in
//! practice).

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// A component of the invocation directory relative to the source root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentKind {
    /// `.`: skipped.
    CurDir,
    /// A directory name: descended into.
    Normal,
    /// `..`, a root, or a prefix: ends the search.
    Stop,
}

/// One call the walk makes to production (ghost model of the call log).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkEvent {
    /// `enter(i)` answered whether component `i`'s directory is a boundary.
    Enter(usize, bool),
    /// `search(i)` recorded component `i`'s directory as searched.
    Search(usize),
}

/// Production side of the walk.
pub trait DiscoveryWalk {
    /// Ghost model: the calls made so far, in order.
    spec fn events(&self) -> Seq<WalkEvent>;

    /// Descend into name component `i` and report whether the directory
    /// reached is a boundary (a nested checkout or a registered gitlink).
    /// Trusted: the call is recorded in the model.
    fn enter(&mut self, i: usize) -> (boundary: bool)
        ensures
            final(self).events() == old(self).events().push(WalkEvent::Enter(i, boundary)),
    ;

    /// Record the current directory (reached by component `i`) as searched.
    /// Trusted: the call is recorded in the model.
    fn search(&mut self, i: usize)
        ensures
            final(self).events() == old(self).events().push(WalkEvent::Search(i)),
    ;
}

/// The name components the walk may descend into, from index `from` on:
/// every name, in order, up to the first `..`/root/prefix component.
pub open spec fn reachable_names(kinds: Seq<ComponentKind>, from: int) -> Seq<usize>
    decreases kinds.len() - from,
{
    if from < 0 || from >= kinds.len() {
        Seq::empty()
    } else {
        match kinds[from] {
            ComponentKind::CurDir => reachable_names(kinds, from + 1),
            ComponentKind::Stop => Seq::empty(),
            ComponentKind::Normal => seq![from as usize] + reachable_names(kinds, from + 1),
        }
    }
}

/// The calls for the first `m` reachable names, none a boundary: each is
/// entered and then searched.
pub open spec fn walk_log(ns: Seq<usize>, m: int) -> Seq<WalkEvent>
    decreases m,
{
    if m <= 0 {
        Seq::empty()
    } else {
        walk_log(ns, m - 1) + seq![WalkEvent::Enter(ns[m - 1], false), WalkEvent::Search(ns[m - 1])]
    }
}

/// The full call log when the first `m` reachable names are not boundaries
/// and name `m` (if any) is: that boundary is entered and nothing follows.
pub open spec fn walk_outcome(ns: Seq<usize>, m: int) -> Seq<WalkEvent> {
    if m < ns.len() {
        walk_log(ns, m).push(WalkEvent::Enter(ns[m], true))
    } else {
        walk_log(ns, m)
    }
}

/// Walks from the source root toward the invocation directory.
///
/// Proven: the calls made are exactly `walk_outcome(reachable_names(kinds,
/// 0), m)` for some `m`: the reachable names are entered in order, each
/// non-boundary is searched right after it is entered, the first boundary
/// ends the walk unsearched, and when no boundary is met every reachable
/// name is searched. With `theorem_reachable_names` this gives: only names
/// before the first `..`/root/prefix component are entered or searched,
/// in increasing index order, at most once each, and nothing at or below a
/// boundary is searched.
pub fn walk_toward_invocation<W: DiscoveryWalk>(kinds: &[ComponentKind], walk: &mut W)
    ensures
        exists|m: int|
            0 <= m <= reachable_names(kinds@, 0).len() && final(walk).events() == old(
                walk,
            ).events() + #[trigger] walk_outcome(reachable_names(kinds@, 0), m),
{
    let ghost ev0 = walk.events();
    let ghost ns = reachable_names(kinds@, 0);
    let ghost mut m: int = 0;
    let mut i: usize = 0;
    proof {
        assert(ev0 + walk_log(ns, 0) =~= ev0);
        assert(ns.skip(0) =~= ns);
    }
    while i < kinds.len()
        invariant
            i <= kinds@.len(),
            ev0 == old(walk).events(),
            ns == reachable_names(kinds@, 0),
            0 <= m <= ns.len(),
            ns.skip(m) == reachable_names(kinds@, i as int),
            walk.events() == ev0 + walk_log(ns, m),
        decreases kinds.len() - i,
    {
        match kinds[i] {
            ComponentKind::CurDir => {},
            ComponentKind::Stop => {
                proof {
                    assert(ns.skip(m).len() == 0);
                    assert(walk_outcome(ns, m) == walk_log(ns, m));
                    assert(walk.events() == ev0 + walk_outcome(reachable_names(kinds@, 0), m));
                }
                return;
            },
            ComponentKind::Normal => {
                proof {
                    assert(ns.skip(m) == seq![i] + reachable_names(kinds@, i + 1));
                    assert(ns.skip(m)[0] == i);
                    assert(ns[m] == i);
                }
                let boundary = walk.enter(i);
                if boundary {
                    proof {
                        assert(walk.events() =~= ev0 + walk_outcome(ns, m));
                        assert(walk.events() == ev0 + walk_outcome(reachable_names(kinds@, 0), m));
                    }
                    return;
                }
                walk.search(i);
                proof {
                    assert(walk_log(ns, m + 1) == walk_log(ns, m) + seq![
                        WalkEvent::Enter(ns[m], false),
                        WalkEvent::Search(ns[m]),
                    ]);
                    assert(walk.events() =~= ev0 + walk_log(ns, m + 1));
                    assert(ns.skip(m + 1) =~= ns.skip(m).drop_first());
                    assert(ns.skip(m).drop_first() =~= reachable_names(kinds@, i + 1));
                    m = m + 1;
                }
            },
        }
        i += 1;
    }
    proof {
        assert(ns.skip(m).len() == 0);
        assert(walk_outcome(ns, m) == walk_log(ns, m));
        assert(walk.events() == ev0 + walk_outcome(reachable_names(kinds@, 0), m));
    }
}

/// Each reachable name is an in-bounds name with no `..`/root/prefix
/// component from `from` up to and including it.
pub proof fn lemma_reachable_names_sound(kinds: Seq<ComponentKind>, from: int, j: int, k: int)
    requires
        0 <= from,
        kinds.len() <= usize::MAX,
        0 <= j < reachable_names(kinds, from).len(),
    ensures
        from <= reachable_names(kinds, from)[j] < kinds.len(),
        kinds[reachable_names(kinds, from)[j] as int] is Normal,
        from <= k <= reachable_names(kinds, from)[j] ==> !(kinds[k] is Stop),
    decreases kinds.len() - from,
{
    let ns = reachable_names(kinds, from);
    let rest = reachable_names(kinds, from + 1);
    match kinds[from] {
        ComponentKind::CurDir => {
            lemma_reachable_names_sound(kinds, from + 1, j, k);
        },
        ComponentKind::Stop => {},
        ComponentKind::Normal => {
            if j > 0 {
                assert(ns[j] == rest[j - 1]);
                lemma_reachable_names_sound(kinds, from + 1, j - 1, k);
            }
        },
    }
}

/// Reachable names are strictly increasing.
pub proof fn lemma_reachable_names_increasing(kinds: Seq<ComponentKind>, from: int, a: int, b: int)
    requires
        0 <= from,
        kinds.len() <= usize::MAX,
        0 <= a < b < reachable_names(kinds, from).len(),
    ensures
        reachable_names(kinds, from)[a] < reachable_names(kinds, from)[b],
    decreases kinds.len() - from,
{
    let ns = reachable_names(kinds, from);
    let rest = reachable_names(kinds, from + 1);
    match kinds[from] {
        ComponentKind::CurDir => {
            lemma_reachable_names_increasing(kinds, from + 1, a, b);
        },
        ComponentKind::Stop => {},
        ComponentKind::Normal => {
            assert(ns[b] == rest[b - 1]);
            if a > 0 {
                assert(ns[a] == rest[a - 1]);
                lemma_reachable_names_increasing(kinds, from + 1, a - 1, b - 1);
            } else {
                lemma_reachable_names_sound(kinds, from + 1, b - 1, from + 1);
            }
        },
    }
}

/// Every name with no `..`/root/prefix component from `from` up to it is
/// reachable.
pub proof fn lemma_reachable_names_complete(kinds: Seq<ComponentKind>, from: int, i: int)
    requires
        0 <= from <= i < kinds.len(),
        kinds.len() <= usize::MAX,
        kinds[i] is Normal,
        forall|k: int| from <= k < i ==> !(#[trigger] kinds[k] is Stop),
    ensures
        reachable_names(kinds, from).contains(i as usize),
    decreases kinds.len() - from,
{
    let ns = reachable_names(kinds, from);
    let rest = reachable_names(kinds, from + 1);
    if i == from {
        assert(ns[0] == from as usize);
    } else {
        assert(!(kinds[from] is Stop));
        lemma_reachable_names_complete(kinds, from + 1, i);
        let j = choose|j: int| 0 <= j < rest.len() && rest[j] == i as usize;
        match kinds[from] {
            ComponentKind::CurDir => {
                assert(ns[j] == i as usize);
            },
            ComponentKind::Stop => {},
            ComponentKind::Normal => {
                assert(ns[j + 1] == i as usize);
            },
        }
    }
}

/// The reachable names (from the start) are in bounds, are names, come
/// strictly before any `..`/root/prefix component, are strictly increasing,
/// and include every name that has no such component before it.
pub proof fn theorem_reachable_names(kinds: Seq<ComponentKind>)
    requires
        kinds.len() <= usize::MAX,
    ensures
        ({
            let ns = reachable_names(kinds, 0);
            &&& forall|j: int|
                0 <= j < ns.len() ==> #[trigger] ns[j] < kinds.len() && kinds[ns[j] as int] is Normal
            &&& forall|j: int, k: int|
                #![trigger ns[j], kinds[k]]
                0 <= j < ns.len() && 0 <= k <= ns[j] ==> !(kinds[k] is Stop)
            &&& forall|a: int, b: int| #![trigger ns[a], ns[b]] 0 <= a < b < ns.len() ==> ns[a] < ns[b]
            &&& forall|i: int|
                0 <= i < kinds.len() && #[trigger] kinds[i] is Normal && (forall|k: int|
                    0 <= k < i ==> !(#[trigger] kinds[k] is Stop)) ==> ns.contains(i as usize)
        }),
{
    let ns = reachable_names(kinds, 0);
    assert forall|j: int| 0 <= j < ns.len() implies #[trigger] ns[j] < kinds.len() && kinds[ns[j] as int] is Normal by {
        lemma_reachable_names_sound(kinds, 0, j, 0);
    }
    assert forall|j: int, k: int|
        #![trigger ns[j], kinds[k]]
        0 <= j < ns.len() && 0 <= k <= ns[j] implies !(kinds[k] is Stop) by {
        lemma_reachable_names_sound(kinds, 0, j, k);
    }
    assert forall|a: int, b: int| #![trigger ns[a], ns[b]] 0 <= a < b < ns.len() implies ns[a] < ns[b] by {
        lemma_reachable_names_increasing(kinds, 0, a, b);
    }
    assert forall|i: int|
        0 <= i < kinds.len() && #[trigger] kinds[i] is Normal && (forall|k: int|
            0 <= k < i ==> !(#[trigger] kinds[k] is Stop)) implies ns.contains(i as usize) by {
        lemma_reachable_names_complete(kinds, 0, i);
    }
}

} // verus!
