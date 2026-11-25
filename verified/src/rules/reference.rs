//! Reference semantics for the precedence kernel: a per-level model of what
//! the `ignore` matchers answer, Git's own per-directory exclude semantics
//! over the same model, and the proven agreements and divergences between
//! the two.
//!
//! # The level model
//!
//! A path with `len` components has levels `0..=len`: level `k` is the
//! prefix of `k` components, so level `len` is the path itself and level 0
//! is the repository root. Rule file `i` lives in the directory at level
//! `depth(i)` (the root file has depth 0). `own(i, k)` is rule file `i`'s
//! own verdict for level `k` (its last matching line, directory-only lines
//! counting only for directories) *as its `ignore` matcher computes it*.
//! For `k <= depth(i)` that is a path outside the file's scope: `ignore`
//! strips the file's own directory to the empty path, and passes an outer
//! ancestor's absolute path through unchanged because its byte-prefix strip
//! fails. Git never consults those answers; waft does.
//!
//! `model_pm` and `model_ai` are the oracle answers this model
//! induces: `matched_path_or_any_parents` walks from the path up to the
//! rule file's own directory and returns the first match, and an ancestor
//! query asks one level. This correspondence is a trusted description of
//! `ignore` 0.4.33 (`Gitignore::matched_path_or_any_parents` and
//! `Gitignore::matched`), not something the kernel relies on: it is used only
//! to state the theorems below.
//!
//! # Results
//!
//! - `theorem_root_file_agrees_with_git`: with only the root rule file
//!   (the `claude-2026-04` engine, or the `git` engine without nested
//!   files) waft selects a path exactly when Git would, unless the root file
//!   matches the repository root itself (a bare `/` line).
//! - `theorem_without_negations_agrees_with_git`: with any number of
//!   nested rule files, waft selects a path exactly when Git would, provided
//!   no rule file has a negated verdict and none matches its own directory.
//! - Four lemmas exhibit concrete inputs on which the current `git` engine
//!   and Git disagree. They document current behavior; they are not fixed:
//!   `lemma_quirk_nested_file_reaches_outside_its_directory`,
//!   `lemma_quirk_cross_file_caveat_ignores_deeper_reinclusion`,
//!   `lemma_quirk_ancestor_negation_shadows_shallower_selection` and
//!   `lemma_quirk_bare_slash_selects_everything`.

#[allow(unused_imports)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
use super::precedence::MatchKind;

verus! {

/// The first verdict of rule file `i` walking from level `k` up to level
/// `lo` (inclusive), as `matched_path_or_any_parents` does.
pub open spec fn walk_up(own: spec_fn(usize, int) -> Option<MatchKind>, i: usize, k: int, lo: int) -> Option<
    MatchKind,
>
    decreases k - lo + 1,
{
    if k < lo {
        None
    } else {
        match own(i, k) {
            Some(v) => Some(v),
            None => walk_up(own, i, k - 1, lo),
        }
    }
}

/// The path verdicts the oracle reports under the model: rule file `i`'s
/// walk from the path (level `len`) up to its own directory.
pub open spec fn model_pm(
    own: spec_fn(usize, int) -> Option<MatchKind>,
    depth: spec_fn(usize) -> int,
    len: int,
) -> spec_fn(usize) -> Option<(MatchKind, ())> {
    |i: usize|
        match walk_up(own, i, len, depth(i)) {
            Some(v) => Some((v, ())),
            None => None,
        }
}

/// The ancestor selections the oracle reports under the model: ancestor
/// `a` (0 is the parent) is level `len - 1 - a`.
pub open spec fn model_ai(own: spec_fn(usize, int) -> Option<MatchKind>, len: int) -> spec_fn(
    usize,
    usize,
) -> Option<()> {
    |a: usize, i: usize|
        if own(i, len - 1 - a) == Some(MatchKind::Ignore) {
            Some(())
        } else {
            None
        }
}

/// The `git` engine selects the path (verdict `Included`) under the model.
pub open spec fn waft_selects(
    own: spec_fn(usize, int) -> Option<MatchKind>,
    depth: spec_fn(usize) -> int,
    n: usize,
    len: int,
) -> bool {
    super::precedence::spec_explain(model_pm(own, depth, len), model_ai(own, len), n, (len - 1) as usize) is Included
}

/// In Git, rule file `i` decides level `k`: it lives strictly above level
/// `k`, has a verdict there, and no deeper applicable file has one.
pub open spec fn is_git_decider(
    own: spec_fn(usize, int) -> Option<MatchKind>,
    depth: spec_fn(usize) -> int,
    n: usize,
    k: int,
    i: usize,
) -> bool {
    &&& i < n
    &&& depth(i) < k
    &&& own(i, k) is Some
    &&& forall|j: usize| i < j < n && depth(j) < k ==> #[trigger] own(j, k) is None
}

/// Git's combined verdict for level `k`: the deepest applicable rule file
/// with a verdict there decides.
pub open spec fn git_level_verdict(
    own: spec_fn(usize, int) -> Option<MatchKind>,
    depth: spec_fn(usize) -> int,
    n: usize,
    k: int,
) -> Option<MatchKind> {
    if exists|i: usize| is_git_decider(own, depth, n, k, i) {
        own(choose|i: usize| is_git_decider(own, depth, n, k, i), k)
    } else {
        None
    }
}

/// Git selects (excludes, in `.gitignore` terms) the path exactly when the
/// path itself or one of its ancestor directories has combined verdict
/// `Ignore`: Git never descends into an excluded directory, and a
/// re-included directory does not decide anything below it.
pub open spec fn git_selects(
    own: spec_fn(usize, int) -> Option<MatchKind>,
    depth: spec_fn(usize) -> int,
    n: usize,
    len: int,
) -> bool {
    exists|k: int| 1 <= k <= len && #[trigger] git_level_verdict(own, depth, n, k) == Some(MatchKind::Ignore)
}

// ---------------------------------------------------------------------------
// The walk
// ---------------------------------------------------------------------------

/// A walk that finds `v` found it at some level, with no verdict above it.
pub proof fn lemma_walk_up_some(own: spec_fn(usize, int) -> Option<MatchKind>, i: usize, k: int, lo: int, v: MatchKind)
    requires
        walk_up(own, i, k, lo) == Some(v),
    ensures
        exists|l: int|
            lo <= l <= k && #[trigger] own(i, l) == Some(v) && forall|h: int| l < h <= k ==> #[trigger] own(i, h) is None,
    decreases k - lo + 1,
{
    if own(i, k) is None {
        lemma_walk_up_some(own, i, k - 1, lo, v);
        let l = choose|l: int|
            lo <= l <= k - 1 && #[trigger] own(i, l) == Some(v) && forall|h: int| l < h <= k - 1 ==> #[trigger] own(
                i,
                h,
            ) is None;
        assert forall|h: int| l < h <= k implies #[trigger] own(i, h) is None by {
            if h < k {
                assert(own(i, h) is None);
            }
        }
    } else {
        assert(own(i, k) == Some(v));
    }
}

/// A walk that finds nothing saw no verdict at any level.
pub proof fn lemma_walk_up_none(own: spec_fn(usize, int) -> Option<MatchKind>, i: usize, k: int, lo: int)
    requires
        walk_up(own, i, k, lo) is None,
    ensures
        forall|l: int| lo <= l <= k ==> #[trigger] own(i, l) is None,
    decreases k - lo + 1,
{
    if k >= lo {
        lemma_walk_up_none(own, i, k - 1, lo);
    }
}

// ---------------------------------------------------------------------------
// Agreement: a lone root rule file
// ---------------------------------------------------------------------------

/// The depth function of a lone root rule file.
pub open spec fn root_depth() -> spec_fn(usize) -> int {
    |i: usize| 0int
}

/// With only the repository-root rule file, waft's verdict selects a path
/// exactly when Git would, provided the root file does not select the
/// repository root itself (which only a bare `/` line does; see
/// `lemma_quirk_bare_slash_selects_everything`). This covers the
/// `claude-2026-04` engine, which reads only the root file, and the `git`
/// engine for repositories without nested rule files.
pub proof fn theorem_root_file_agrees_with_git(own: spec_fn(usize, int) -> Option<MatchKind>, len: int)
    requires
        1 <= len <= usize::MAX,
        own(0, 0) != Some(MatchKind::Ignore),
    ensures
        waft_selects(own, root_depth(), 1, len) <==> git_selects(own, root_depth(), 1, len),
{
    let depth = root_depth();
    let pm = model_pm(own, depth, len);
    let ai = model_ai(own, len);
    let m = (len - 1) as usize;
    super::precedence::theorem_single_file_explain(pm, ai, m);
    // Git's verdict at every level k >= 1 is the root file's own verdict.
    assert forall|k: int| 1 <= k <= len implies #[trigger] git_level_verdict(own, depth, 1, k) == own(0, k) by {
        if own(0, k) is Some {
            assert(is_git_decider(own, depth, 1, k, 0));
        }
        if exists|i: usize| is_git_decider(own, depth, 1, k, i) {
            let i = choose|i: usize| is_git_decider(own, depth, 1, k, i);
            assert(i == 0);
        }
    }
    match walk_up(own, 0, len, 0) {
        None => {
            lemma_walk_up_none(own, 0, len, 0);
            if git_selects(own, depth, 1, len) {
                let k = choose|k: int|
                    1 <= k <= len && #[trigger] git_level_verdict(own, depth, 1, k) == Some(MatchKind::Ignore);
                assert(own(0, k) is None);
            }
        },
        Some(MatchKind::Ignore) => {
            lemma_walk_up_some(own, 0, len, 0, MatchKind::Ignore);
            let l = choose|l: int|
                0 <= l <= len && #[trigger] own(0, l) == Some(MatchKind::Ignore) && forall|h: int|
                    l < h <= len ==> #[trigger] own(0, h) is None;
            assert(1 <= l);
            assert(git_level_verdict(own, depth, 1, l) == Some(MatchKind::Ignore));
        },
        Some(MatchKind::Whitelist) => {
            lemma_walk_up_some(own, 0, len, 0, MatchKind::Whitelist);
            assert(own(0, len) != Some(MatchKind::Ignore));
            if git_selects(own, depth, 1, len) {
                let k = choose|k: int|
                    1 <= k <= len && #[trigger] git_level_verdict(own, depth, 1, k) == Some(MatchKind::Ignore);
                assert(k < len);
                let a = (len - 1 - k) as usize;
                assert(ai(a, 0) is Some);
                super::precedence::lemma_first_ancestor_hit_exists(ai, m, 0, a);
            }
            if waft_selects(own, depth, 1, len) {
                let a = choose|a: usize| super::precedence::is_first_ancestor_hit(ai, m, 0, a);
                let k = len - 1 - a;
                assert(own(0, k) == Some(MatchKind::Ignore));
                assert(git_level_verdict(own, depth, 1, k) == Some(MatchKind::Ignore));
            }
        },
    }
}

/// A walk over a range with a verdict finds one.
pub proof fn lemma_walk_up_finds(own: spec_fn(usize, int) -> Option<MatchKind>, i: usize, k: int, lo: int, l: int)
    requires
        lo <= l <= k,
        own(i, l) is Some,
    ensures
        walk_up(own, i, k, lo) is Some,
{
    if walk_up(own, i, k, lo) is None {
        lemma_walk_up_none(own, i, k, lo);
    }
}

/// Without negations, any number of nested rule files agrees with Git:
/// when no rule file has a negated verdict at any level and none matches
/// its own directory (no bare `/` line), the `git` engine selects a path
/// exactly when Git would. The divergences below all need a negation or a
/// bare `/`.
pub proof fn theorem_without_negations_agrees_with_git(
    own: spec_fn(usize, int) -> Option<MatchKind>,
    depth: spec_fn(usize) -> int,
    n: usize,
    len: int,
)
    requires
        1 <= len <= usize::MAX,
        forall|i: usize, k: int| i < n ==> #[trigger] own(i, k) != Some(MatchKind::Whitelist),
        forall|i: usize| i < n ==> 0 <= #[trigger] depth(i) < len,
        forall|i: usize| i < n ==> #[trigger] own(i, depth(i)) is None,
    ensures
        waft_selects(own, depth, n, len) <==> git_selects(own, depth, n, len),
{
    let pm = model_pm(own, depth, len);
    let ai = model_ai(own, len);
    let m = (len - 1) as usize;
    super::precedence::theorem_no_match_iff_no_file_matches(pm, ai, n, m);
    if waft_selects(own, depth, n, len) {
        let i = choose|i: usize| i < n && #[trigger] pm(i) is Some;
        let v = walk_up(own, i, len, depth(i))->Some_0;
        lemma_walk_up_some(own, i, len, depth(i), v);
        let l = choose|l: int|
            depth(i) <= l <= len && #[trigger] own(i, l) == Some(v) && forall|h: int| l < h <= len ==> #[trigger] own(
                i,
                h,
            ) is None;
        assert(own(i, l) != Some(MatchKind::Whitelist));
        assert(l != depth(i));
        assert(exists|d: usize| is_git_decider(own, depth, n, l, d)) by {
            lemma_git_decider_exists(own, depth, n, l, i);
        }
        let d = choose|d: usize| is_git_decider(own, depth, n, l, d);
        assert(own(d, l) != Some(MatchKind::Whitelist));
        assert(git_level_verdict(own, depth, n, l) == Some(MatchKind::Ignore));
    }
    if git_selects(own, depth, n, len) {
        let k = choose|k: int| 1 <= k <= len && #[trigger] git_level_verdict(own, depth, n, k) == Some(MatchKind::Ignore);
        let d = choose|d: usize| is_git_decider(own, depth, n, k, d);
        lemma_walk_up_finds(own, d, len, depth(d), k);
        assert(pm(d) is Some);
        let v = super::precedence::spec_explain(pm, ai, n, m);
        if v is ExcludedByNegation {
            let c = v->ExcludedByNegation_ctx;
            super::precedence::theorem_negation_needs_no_selected_ancestor(pm, ai, n, m, c, ());
            let wv = walk_up(own, c, len, depth(c))->Some_0;
            lemma_walk_up_some(own, c, len, depth(c), wv);
        }
    }
}

/// Some applicable rule file has a verdict at level `k`, so a deciding
/// (deepest such) one exists.
pub proof fn lemma_git_decider_exists(
    own: spec_fn(usize, int) -> Option<MatchKind>,
    depth: spec_fn(usize) -> int,
    n: usize,
    k: int,
    i: usize,
)
    requires
        i < n,
        depth(i) < k,
        own(i, k) is Some,
    ensures
        exists|d: usize| is_git_decider(own, depth, n, k, d),
    decreases n - i,
{
    if exists|j: usize| i < j < n && depth(j) < k && #[trigger] own(j, k) is Some {
        let j = choose|j: usize| i < j < n && depth(j) < k && #[trigger] own(j, k) is Some;
        lemma_git_decider_exists(own, depth, n, k, j);
    } else {
        assert(is_git_decider(own, depth, n, k, i));
    }
}

// ---------------------------------------------------------------------------
// Divergences from Git (current behavior, deliberately not fixed)
// ---------------------------------------------------------------------------

/// Quirk 1 table: `app/config/.worktreeinclude` containing `*` then
/// `!secret.key`, path `app/config/secret.key` (levels: 1 `app`,
/// 2 `app/config`, 3 the file). The file's own answer for the file is the
/// negation; for the outer ancestor `app` its matcher sees the unstripped
/// absolute path, whose basename `*` matches.
pub open spec fn quirk1_own() -> spec_fn(usize, int) -> Option<MatchKind> {
    |i: usize, k: int|
        if k == 3 {
            Some(MatchKind::Whitelist)
        } else if k == 1 {
            Some(MatchKind::Ignore)
        } else {
            None
        }
}

/// Quirk 1 depths: the only rule file lives at level 2 (`app/config`).
pub open spec fn quirk1_depth() -> spec_fn(usize) -> int {
    |i: usize| 2int
}

/// Quirk 1 (over-selection): a nested rule file's patterns are applied to
/// ancestor directories outside its own directory, so "everything here
/// except `secret.key`" still selects `secret.key`. Git consults
/// `app/config/.worktreeinclude` only for paths below `app/config` and does
/// not select the file; waft reports `Included` through ancestor `app`.
pub proof fn lemma_quirk_nested_file_reaches_outside_its_directory()
    ensures
        waft_selects(quirk1_own(), quirk1_depth(), 1, 3),
        !git_selects(quirk1_own(), quirk1_depth(), 1, 3),
{
    let own = quirk1_own();
    let depth = quirk1_depth();
    let pm = model_pm(own, depth, 3);
    let ai = model_ai(own, 3);
    assert(walk_up(own, 0, 3, 2) == Some(MatchKind::Whitelist));
    super::precedence::theorem_single_file_explain(pm, ai, 2);
    assert(super::precedence::is_first_ancestor_hit(ai, 2, 0, 1));
    assert forall|k: int| 1 <= k <= 3 implies #[trigger] git_level_verdict(own, depth, 1, k) != Some(
        MatchKind::Ignore,
    ) by {
        if exists|i: usize| is_git_decider(own, depth, 1, k, i) {
            let i = choose|i: usize| is_git_decider(own, depth, 1, k, i);
            assert(i == 0 && k == 3);
        }
    }
}

/// Quirk 2 table: root `.worktreeinclude` containing `a/b/`, and
/// `a/.worktreeinclude` containing `!b/` then `!b/x`; path `a/b/x`
/// (levels: 1 `a`, 2 `a/b`, 3 the file). Rule file 0 is the root file,
/// rule file 1 the nested one.
pub open spec fn quirk2_own() -> spec_fn(usize, int) -> Option<MatchKind> {
    |i: usize, k: int|
        if i == 0 && k == 2 {
            Some(MatchKind::Ignore)
        } else if i == 1 && (k == 2 || k == 3) {
            Some(MatchKind::Whitelist)
        } else {
            None
        }
}

/// Quirk 2 depths: the root file and the file in `a`.
pub open spec fn quirk2_depth() -> spec_fn(usize) -> int {
    |i: usize| i as int
}

/// Quirk 2 (over-selection): the cross-file caveat checks each file's
/// ancestor selection in isolation and ignores a deeper file that
/// re-includes the directory. Git's verdict for `a/b` is the deeper `!b/`,
/// so it selects nothing; waft reports `Included` through the root's `a/b/`.
pub proof fn lemma_quirk_cross_file_caveat_ignores_deeper_reinclusion()
    ensures
        waft_selects(quirk2_own(), quirk2_depth(), 2, 3),
        !git_selects(quirk2_own(), quirk2_depth(), 2, 3),
{
    let own = quirk2_own();
    let depth = quirk2_depth();
    let pm = model_pm(own, depth, 3);
    let ai = model_ai(own, 3);
    assert(walk_up(own, 1, 3, 1) == Some(MatchKind::Whitelist));
    assert(walk_up(own, 0, 2, 0) == Some(MatchKind::Ignore));
    assert(walk_up(own, 0, 3, 0) == Some(MatchKind::Ignore));
    assert(super::precedence::is_deepest_match(pm, 2, 1));
    let d = choose|d: usize| super::precedence::is_deepest_match(pm, 2, d);
    super::precedence::lemma_deepest_unique(pm, 2, 1, d);
    if exists|a: usize| super::precedence::is_first_ancestor_hit(ai, 2, 1, a) {
        let a = choose|a: usize| super::precedence::is_first_ancestor_hit(ai, 2, 1, a);
        assert(ai(a, 1) is None);
    }
    assert(super::precedence::is_first_cross_hit(ai, 2, 2, 0, 0));
    assert forall|k: int| 1 <= k <= 3 implies #[trigger] git_level_verdict(own, depth, 2, k) != Some(
        MatchKind::Ignore,
    ) by {
        if exists|i: usize| is_git_decider(own, depth, 2, k, i) {
            let i = choose|i: usize| is_git_decider(own, depth, 2, k, i);
            if k == 2 && i == 0 {
                assert(own(1, 2) is None);
            }
        }
    }
}

/// Quirk 3 table: root `.worktreeinclude` containing `*.env`, and
/// `sub/.worktreeinclude` containing `!x/`; path `sub/x/a.env` (levels:
/// 1 `sub`, 2 `sub/x`, 3 the file). Rule file 0 is the root file, rule
/// file 1 the nested one.
pub open spec fn quirk3_own() -> spec_fn(usize, int) -> Option<MatchKind> {
    |i: usize, k: int|
        if i == 0 && k == 3 {
            Some(MatchKind::Ignore)
        } else if i == 1 && k == 2 {
            Some(MatchKind::Whitelist)
        } else {
            None
        }
}

/// Quirk 3 (under-selection): `matched_path_or_any_parents` reports the
/// nested file's re-inclusion of the *directory* `sub/x` as that file's
/// verdict for the path, and the deeper file wins. Git re-includes the
/// directory, then the root's `*.env` selects the file; waft reports
/// `ExcludedByNegation` (`!x/`).
pub proof fn lemma_quirk_ancestor_negation_shadows_shallower_selection()
    ensures
        !waft_selects(quirk3_own(), quirk2_depth(), 2, 3),
        git_selects(quirk3_own(), quirk2_depth(), 2, 3),
{
    let own = quirk3_own();
    let depth = quirk2_depth();
    let pm = model_pm(own, depth, 3);
    let ai = model_ai(own, 3);
    assert(walk_up(own, 1, 2, 1) == Some(MatchKind::Whitelist));
    assert(walk_up(own, 1, 3, 1) == Some(MatchKind::Whitelist));
    assert(walk_up(own, 0, 3, 0) == Some(MatchKind::Ignore));
    assert(super::precedence::is_deepest_match(pm, 2, 1));
    super::precedence::theorem_negation_deselects_without_selected_ancestor(pm, ai, 2, 2, 1, ());
    assert(is_git_decider(own, depth, 2, 3, 0));
    let i = choose|i: usize| is_git_decider(own, depth, 2, 3, i);
    assert(i == 0);
    assert(git_level_verdict(own, depth, 2, 3) == Some(MatchKind::Ignore));
}

/// Quirk 4 table: a root `.worktreeinclude` whose only line is `/`, which
/// compiles to a glob matching the empty path, i.e. the rule file's own
/// directory (level 0); any path of three components.
pub open spec fn quirk4_own() -> spec_fn(usize, int) -> Option<MatchKind> {
    |i: usize, k: int|
        if k == 0 {
            Some(MatchKind::Ignore)
        } else {
            None
        }
}

/// Quirk 4 (over-selection): `matched_path_or_any_parents` also tests the
/// empty path, so a bare `/` line selects every path for both the `git` and
/// the `claude-2026-04` engine. Git selects nothing. (Eligibility still
/// requires the path to be git-ignored, so the effect equals all-ignored
/// mode.)
pub proof fn lemma_quirk_bare_slash_selects_everything()
    ensures
        waft_selects(quirk4_own(), root_depth(), 1, 3),
        !git_selects(quirk4_own(), root_depth(), 1, 3),
{
    let own = quirk4_own();
    let depth = root_depth();
    let pm = model_pm(own, depth, 3);
    let ai = model_ai(own, 3);
    assert(walk_up(own, 0, 0, 0) == Some(MatchKind::Ignore));
    assert(walk_up(own, 0, 1, 0) == Some(MatchKind::Ignore));
    assert(walk_up(own, 0, 2, 0) == Some(MatchKind::Ignore));
    assert(walk_up(own, 0, 3, 0) == Some(MatchKind::Ignore));
    super::precedence::theorem_single_file_explain(pm, ai, 2);
    assert forall|k: int| 1 <= k <= 3 implies #[trigger] git_level_verdict(own, depth, 1, k) != Some(
        MatchKind::Ignore,
    ) by {
        if exists|i: usize| is_git_decider(own, depth, 1, k, i) {
            let i = choose|i: usize| is_git_decider(own, depth, 1, k, i);
            assert(own(i, k) is None);
        }
    }
}

} // verus!
