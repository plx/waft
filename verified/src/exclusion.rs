//! Verified kernel: the policy exclusion verdict and its case policy.
//!
//! # What it decides
//!
//! [`path_excluded`] decides whether the exclusion filter
//! (`policy_filter::filter_paths_with_case`) drops a candidate. The
//! exclusion matcher holds the built-in patterns (for `tooling-v1`) followed
//! by the user's `extra_excludes`, and the candidate is asked about level by
//! level: the path itself (as a directory only if it is one on disk), then
//! each parent directory, ending with the empty path (the source root). The
//! first level with a verdict decides; the candidate is dropped iff that
//! verdict is a selecting (non-negated) pattern. This is exactly the walk of
//! `ignore`'s `Gitignore::matched_path_or_any_parents`, now performed here so
//! that its consequences can be proved. (When `ignore` could strip part of a
//! level, which only a library caller can cause, for example with a relative
//! source root, production keeps that call's walk and hands the kernel its
//! verdict as a single level; see the trust assumptions.)
//!
//! [`exclusion_folds_case`] decides whether exclusion matching folds case.
//!
//! # Theorems
//!
//! - `path_excluded` returns `spec_excluded`: the first level with a
//!   verdict exists and is a selection. Its query log shows it asks the
//!   levels in order, path first, and stops at the first verdict.
//! - `theorem_excluded_without_negation`: when no level matches a negated
//!   pattern, a candidate is excluded iff *some* level is selected.
//! - `theorem_excluded_directory_excludes_subtree`: if a directory is
//!   excluded, so is every path below it unless a negated pattern matches the
//!   path or a directory between the two.
//! - `lemma_quirk_negated_extra_reincludes_guarded_path` (current
//!   behavior, not fixed): with the built-in `.claude/` and an extra
//!   `!.claude/settings.json`, the directory `.claude` is excluded but
//!   `.claude/settings.json` is kept. Extras are added after the built-ins,
//!   so a negated extra that matches a path overrides the guard; an extra
//!   `!*` disables every `tooling-v1` guard, `.git/` included.
//! - [`exclusion_folds_case`] folds case exactly when `core.ignoreCase`
//!   is set or the platform may alias case, so it is never less
//!   conservative than either.
//!
//! # Trust assumptions
//!
//! - [`LevelMatchOracle`]: production answers level `l` with
//!   `Gitignore::matched(level_l, is_dir_l)` on the compiled exclusion
//!   matcher (glob semantics, last-match-wins within the pattern list and
//!   ASCII-only case folding stay in `ignore`/`globset`). The levels are
//!   `Path::parent` steps of the relative candidate, exactly the walk inside
//!   `matched_path_or_any_parents`. Asking `matched` per level is equivalent
//!   only because `ignore`'s prefix strip is the identity on every level of a
//!   relative candidate below an absolute matcher root. The CLI always passes
//!   a canonical, absolute source root. For any other matcher root (the
//!   public `filter_paths` functions accept a relative one, which `ignore`
//!   strips byte-wise from the whole candidate), and for a candidate with a
//!   root or a leading `.` component, production reports a single level
//!   answered by `matched_path_or_any_parents` itself. The verdict is then
//!   that call's, as before this kernel existed, and the theorems below speak
//!   about that one level only. Production checks the condition, and its
//!   tests compare both cases with `matched_path_or_any_parents`.
//! - Whether the path is a directory comes from the filesystem (`is_dir`).
//! - Whether the platform may alias differently-cased names is a
//!   compile-time platform fact supplied by production.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

use crate::rules::precedence::MatchKind;

/// The exclusion matcher's per-level answers, implemented by production.
///
/// Trusted: `query_level(l)` returns the compiled exclusion matcher's
/// verdict for level `l` of the candidate (0 is the path itself, then each
/// parent, the last being the empty path). When `ignore`'s prefix strip
/// could change a level (the matcher root is not absolute, or the candidate
/// has a root or starts with a `.` component), there is a single level 0,
/// holding the matcher's own parent-walk verdict for the candidate. The
/// answers are a fixed function of `l`, and `queries` is a ghost log of the
/// levels asked, in order.
pub trait LevelMatchOracle {
    /// Number of levels (the path, its parents, and the empty path).
    spec fn spec_levels(&self) -> usize;

    /// The matcher's verdict for level `l`.
    spec fn level_match(&self, l: usize) -> Option<MatchKind>;

    /// Levels asked so far, in order.
    spec fn queries(&self) -> Seq<usize>;

    /// The number of levels.
    fn levels(&self) -> (k: usize)
        ensures
            k == self.spec_levels(),
    ;

    /// Ask the matcher about level `l`.
    fn query_level(&mut self, l: usize) -> (r: Option<MatchKind>)
        requires
            l < old(self).spec_levels(),
        ensures
            r == old(self).level_match(l),
            final(self).queries() == old(self).queries().push(l),
            final(self).spec_levels() == old(self).spec_levels(),
            forall|h: usize| #[trigger] final(self).level_match(h) == old(self).level_match(h),
    ;
}

/// `l` is the first of the `k` levels (path first) with a verdict.
pub open spec fn is_first_level_match(lm: spec_fn(usize) -> Option<MatchKind>, k: usize, l: usize) -> bool {
    &&& l < k
    &&& lm(l) is Some
    &&& forall|h: usize| h < l ==> #[trigger] lm(h) is None
}

/// A candidate is excluded iff the first level with a verdict is selected.
///
/// Current behavior, kept: because the path's own level is asked first, a
/// negated `extra_excludes` pattern matching the path keeps it even inside a
/// directory a built-in pattern excludes
/// (`lemma_quirk_negated_extra_reincludes_guarded_path`).
pub open spec fn spec_excluded(lm: spec_fn(usize) -> Option<MatchKind>, k: usize) -> bool {
    exists|l: usize| is_first_level_match(lm, k, l) && lm(l) == Some(MatchKind::Ignore)
}

/// The levels asked: up to and including the first with a verdict, or all.
pub open spec fn level_trace(lm: spec_fn(usize) -> Option<MatchKind>, k: usize) -> Seq<usize> {
    let asked = if exists|l: usize| is_first_level_match(lm, k, l) {
        (choose|l: usize| is_first_level_match(lm, k, l)) + 1
    } else {
        k as int
    };
    Seq::new(asked as nat, |h: int| h as usize)
}

/// The level answers of an oracle, as a plain function.
pub open spec fn lm_of<O: LevelMatchOracle>(o: &O) -> spec_fn(usize) -> Option<MatchKind> {
    |l: usize| o.level_match(l)
}

/// The first level with a verdict is unique.
pub proof fn lemma_first_level_match_unique(lm: spec_fn(usize) -> Option<MatchKind>, k: usize, l1: usize, l2: usize)
    requires
        is_first_level_match(lm, k, l1),
        is_first_level_match(lm, k, l2),
    ensures
        l1 == l2,
{
    if l1 < l2 {
        assert(lm(l1) is None);
    } else if l2 < l1 {
        assert(lm(l2) is None);
    }
}

/// Some level has a verdict, so a first one exists.
pub proof fn lemma_first_level_match_exists(lm: spec_fn(usize) -> Option<MatchKind>, k: usize, l: usize)
    requires
        l < k,
        lm(l) is Some,
    ensures
        exists|f: usize| is_first_level_match(lm, k, f),
    decreases l,
{
    if exists|h: usize| h < l && #[trigger] lm(h) is Some {
        let h = choose|h: usize| h < l && #[trigger] lm(h) is Some;
        lemma_first_level_match_exists(lm, k, h);
    } else {
        assert(is_first_level_match(lm, k, l));
    }
}

/// Decide whether the exclusion filter drops a candidate: walk from the
/// path up through its parents and let the first verdict decide.
pub fn path_excluded<O: LevelMatchOracle>(o: &mut O) -> (r: bool)
    ensures
        r == spec_excluded(lm_of(old(o)), old(o).spec_levels()),
        final(o).queries() == old(o).queries() + level_trace(lm_of(old(o)), old(o).spec_levels()),
{
    let ghost o0 = *o;
    let ghost lm = lm_of(&o0);
    let k = o.levels();
    let mut l: usize = 0;
    assert(o.queries() =~= o0.queries() + Seq::new(0, |h: int| h as usize));
    while l < k
        invariant
            l <= k,
            o0 == *old(o),
            lm == lm_of(&o0),
            k == o0.spec_levels(),
            o.spec_levels() == k,
            forall|h: usize| #[trigger] o.level_match(h) == lm(h),
            forall|h: usize| h < l ==> #[trigger] lm(h) is None,
            o.queries() == o0.queries() + Seq::new(l as nat, |h: int| h as usize),
        decreases k - l,
    {
        let verdict = o.query_level(l);
        proof {
            assert(Seq::new((l + 1) as nat, |h: int| h as usize) =~= Seq::new(l as nat, |h: int| h as usize).push(
                l,
            ));
            if verdict is Some {
                assert(is_first_level_match(lm, k, l));
                let f = choose|f: usize| is_first_level_match(lm, k, f);
                lemma_first_level_match_unique(lm, k, l, f);
                assert(o.queries() =~= o0.queries() + level_trace(lm, k));
            }
        }
        match verdict {
            Some(MatchKind::Ignore) => return true,
            Some(MatchKind::Whitelist) => return false,
            None => {},
        }
        l += 1;
    }
    proof {
        if exists|f: usize| is_first_level_match(lm, k, f) {
            let f = choose|f: usize| is_first_level_match(lm, k, f);
            assert(lm(f) is None);
        }
        assert(o.queries() =~= o0.queries() + level_trace(lm, k));
    }
    false
}

/// Without negations the walk order is irrelevant: when no level matches
/// a negated pattern (true for the built-in set and any extras without
/// `!`), a candidate is excluded iff some level is selected.
pub proof fn theorem_excluded_without_negation(lm: spec_fn(usize) -> Option<MatchKind>, k: usize)
    requires
        forall|h: usize| h < k ==> #[trigger] lm(h) != Some(MatchKind::Whitelist),
    ensures
        spec_excluded(lm, k) <==> exists|l: usize| l < k && #[trigger] lm(l) == Some(MatchKind::Ignore),
{
    if exists|l: usize| l < k && #[trigger] lm(l) == Some(MatchKind::Ignore) {
        let l = choose|l: usize| l < k && #[trigger] lm(l) == Some(MatchKind::Ignore);
        lemma_first_level_match_exists(lm, k, l);
        let f = choose|f: usize| is_first_level_match(lm, k, f);
        assert(lm(f) != Some(MatchKind::Whitelist));
        assert(is_first_level_match(lm, k, f) && lm(f) == Some(MatchKind::Ignore));
    }
    if spec_excluded(lm, k) {
        let f = choose|f: usize| is_first_level_match(lm, k, f) && lm(f) == Some(MatchKind::Ignore);
        assert(f < k && lm(f) == Some(MatchKind::Ignore));
    }
}

/// Excluding a directory excludes its subtree: if directory `D` is
/// excluded and path `P` lies `d` levels below it (so `P`'s levels from
/// `d` on are `D`'s levels), then `P` is excluded too, unless a negated
/// pattern matches `P` or a directory strictly between `P` and `D`.
pub proof fn theorem_excluded_directory_excludes_subtree(
    lm_p: spec_fn(usize) -> Option<MatchKind>,
    k_p: usize,
    lm_d: spec_fn(usize) -> Option<MatchKind>,
    k_d: usize,
    d: usize,
)
    requires
        d + k_d == k_p,
        forall|h: usize| h < k_d ==> #[trigger] lm_p((d + h) as usize) == lm_d(h),
        forall|h: usize| h < d ==> #[trigger] lm_p(h) != Some(MatchKind::Whitelist),
        spec_excluded(lm_d, k_d),
    ensures
        spec_excluded(lm_p, k_p),
{
    let f = choose|f: usize| is_first_level_match(lm_d, k_d, f) && lm_d(f) == Some(MatchKind::Ignore);
    assert(lm_p((d + f) as usize) == Some(MatchKind::Ignore));
    lemma_first_level_match_exists(lm_p, k_p, (d + f) as usize);
    let g = choose|g: usize| is_first_level_match(lm_p, k_p, g);
    if g >= d {
        let h = (g - d) as usize;
        assert(lm_p((d + h) as usize) == lm_d(h));
        if h < f {
            assert(lm_d(h) is None);
        }
        if f < h {
            assert(lm_p((d + f) as usize) is None);
        }
        assert(h == f);
        assert(is_first_level_match(lm_p, k_p, g) && lm_p(g) == Some(MatchKind::Ignore));
    } else {
        assert(lm_p(g) != Some(MatchKind::Whitelist));
        assert(is_first_level_match(lm_p, k_p, g) && lm_p(g) == Some(MatchKind::Ignore));
    }
}

/// Levels of `.claude/settings.json` (a file) under the built-in `.claude/`
/// plus an extra `!.claude/settings.json`: the path itself matches the
/// negated extra, `.claude` matches the built-in, the empty path nothing.
pub open spec fn quirk_settings_levels() -> spec_fn(usize) -> Option<MatchKind> {
    |l: usize|
        if l == 0 {
            Some(MatchKind::Whitelist)
        } else if l == 1 {
            Some(MatchKind::Ignore)
        } else {
            None
        }
}

/// Levels of the directory `.claude` under the same patterns.
pub open spec fn quirk_claude_dir_levels() -> spec_fn(usize) -> Option<MatchKind> {
    |l: usize|
        if l == 0 {
            Some(MatchKind::Ignore)
        } else {
            None
        }
}

/// Current behavior, deliberately not fixed: a negated extra exclude
/// re-includes a path inside a built-in-guarded directory. The directory
/// `.claude` is excluded, `.claude/settings.json` sits one level below it,
/// and it is kept, because the path's own negated match is found before the
/// walk reaches `.claude`. (So the "unless a negation matches" clause of
/// `theorem_excluded_directory_excludes_subtree` cannot be dropped.)
pub proof fn lemma_quirk_negated_extra_reincludes_guarded_path()
    ensures
        spec_excluded(quirk_claude_dir_levels(), 2),
        forall|h: usize| h < 2 ==> #[trigger] quirk_settings_levels()((1 + h) as usize) == quirk_claude_dir_levels()(h),
        !spec_excluded(quirk_settings_levels(), 3),
{
    let dir = quirk_claude_dir_levels();
    let file = quirk_settings_levels();
    assert(is_first_level_match(dir, 2, 0));
    if spec_excluded(file, 3) {
        let f = choose|f: usize| is_first_level_match(file, 3, f) && file(f) == Some(MatchKind::Ignore);
        assert(is_first_level_match(file, 3, 0));
        lemma_first_level_match_unique(file, 3, 0, f);
    }
}

/// Whether exclusion matching folds case: `core.ignoreCase` is only a
/// lower bound, because failing to exclude a file can copy a secret while
/// a spurious exclusion only withholds a file.
pub fn exclusion_folds_case(core_ignore_case: bool, platform_may_alias: bool) -> (r: bool)
    ensures
        r == (core_ignore_case || platform_may_alias),
{
    core_ignore_case || platform_may_alias
}

} // verus!
