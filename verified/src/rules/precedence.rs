//! Rule-file precedence for the `git` and `claude-2026-04` engines.
//!
//! # What it decides
//!
//! Given the applicable `.worktreeinclude` files of a path (shallowest
//! first) and their matchers, [`explain`] decides the path's verdict for the
//! `git` engine: `NoMatch`, `Included` by a rule file through the path or an
//! ancestor-directory match, or `ExcludedByNegation`. It mirrors the former
//! `explain_path` exactly: every rule file is evaluated in order and the last
//! one that matches decides; a rule file whose verdict is a negation first
//! scans the path's ancestors (deepest first) for one it selects (the
//! within-file caveat); and when the decision is a negation, every ancestor
//! is asked of every file (ancestors deepest first, files shallowest first)
//! for a selection (the cross-file caveat). [`explain_root_only`] decides
//! the `claude-2026-04` engine's verdict from the root rule file alone.
//!
//! The kernel never sees globs or paths: the matchers are reached through
//! the [`RuleMatchOracle`] trait, and the verdict carries the deciding glob
//! back as an opaque witness `W`.
//!
//! # Theorems
//!
//! - `explain` returns `spec_explain` (the deepest matching file's
//!   verdict, overridden by the first cross-file ancestor selection when it
//!   is a negation), and `explain_root_only` returns `spec_explain` for the
//!   root file alone. Both prove their exact query sequence
//!   (`explain_trace`, `file_trace`): the matcher queries are the old
//!   code's, in the old order, and every query is in range.
//! - `theorem_no_match_iff_no_file_matches`: `NoMatch` exactly when no
//!   applicable rule file matches the path.
//! - `theorem_deeper_file_wins` and
//!   `theorem_deepest_selection_decides`: once a deeper file matches, the
//!   path verdicts of shallower files are irrelevant.
//! - `theorem_negation_needs_no_selected_ancestor` (caveat soundness): a
//!   negation deselects only when it is the deepest file's verdict and no
//!   applicable file selects any ancestor directory; and
//!   `theorem_negation_deselects_without_selected_ancestor` (caveat
//!   completeness): otherwise such a negation does deselect.
//! - `theorem_verdict_provenance`: the reported file and witness come from
//!   a query that really produced them, with the exact tie-break order that
//!   `info` and `list -v` print.
//! - `theorem_single_file_explain` (root-only reuse): with one rule file
//!   the cross-file caveat is a no-op, so the root-only engine and the `git`
//!   engine agree on a repository whose only rule file is the root one.
//!
//! The companion module [`super::reference`] relates these verdicts to Git's
//! own per-directory semantics and proves five concrete divergences of the
//! current behavior (kept on purpose; see its documentation).
//!
//! # Trust assumptions
//!
//! [`RuleMatchOracle`] is the trust boundary. Production answers
//! `query_path(ctx)` with rule file `ctx`'s
//! `Gitignore::matched_path_or_any_parents(path, is_dir)` and
//! `query_ancestor(anc, ctx)` with the glob of an ignore match of
//! `Gitignore::matched(ancestor, true)`; the matchers are pure, so the
//! answers are a fixed function of the arguments. Which rule files apply
//! (finding and reading them, the consult gate) and the ancestor list
//! (`Path::parent` steps, empty path excluded) are computed by production.
//! Glob compilation and matching stay in the `ignore` and `globset` crates.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// The kind of rule line that decided a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchKind {
    /// A selecting pattern (an "ignore" match in `ignore` crate terms).
    Ignore,
    /// A negated `!pattern` (a "whitelist" match).
    Whitelist,
}

/// Which query supplied the deciding glob of a [`Verdict::Included`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    /// The path query of the reporting rule file.
    Path,
    /// The ancestor-directory query for ancestor `a` (0 is the parent).
    Ancestor(usize),
}

/// The verdict for one path, carrying the production witness (the glob
/// that decided it) so the caller never has to re-derive it.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict<W> {
    /// No applicable rule file matched the path or (for the caveats) an
    /// ancestor.
    NoMatch,
    /// Selected by rule file `ctx`, through the query named by `by`.
    Included {
        /// Index of the reporting rule file (shallowest first).
        ctx: usize,
        /// Which query matched.
        by: By,
        /// The deciding glob.
        witness: W,
    },
    /// Deselected by a negation in rule file `ctx`.
    ExcludedByNegation {
        /// Index of the reporting rule file (shallowest first).
        ctx: usize,
        /// The deciding negated glob.
        witness: W,
    },
}

/// One matcher query, as recorded in an oracle's ghost query log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleQuery {
    /// `matched_path_or_any_parents(path, is_dir)` on rule file `ctx`.
    Path {
        /// Rule file index.
        ctx: usize,
    },
    /// `matched(ancestor, true)` on rule file `ctx` for ancestor `anc`.
    Ancestor {
        /// Ancestor index (0 is the parent).
        anc: usize,
        /// Rule file index.
        ctx: usize,
    },
}

/// The glob matchers of the applicable rule files, implemented by
/// production on top of the `ignore` crate.
///
/// Trusted: `query_path(ctx)` returns rule file `ctx`'s verdict for the
/// queried path (`Gitignore::matched_path_or_any_parents`) and
/// `query_ancestor(anc, ctx)` returns the glob when rule file `ctx` selects
/// ancestor directory `anc` (`Gitignore::matched(.., true)` is an ignore
/// match). The answers are a fixed function of the arguments for the whole
/// evaluation (the matchers are pure), and `queries` is a ghost log of the
/// questions asked, so the kernel can state exactly which matcher queries it
/// issues and in what order.
pub trait RuleMatchOracle<W> {
    /// Number of applicable rule files, shallowest first.
    spec fn spec_contexts(&self) -> usize;

    /// Number of non-empty ancestor directories of the path, deepest
    /// (the parent) first.
    spec fn spec_ancestors(&self) -> usize;

    /// Rule file `ctx`'s verdict for the path.
    spec fn path_match(&self, ctx: usize) -> Option<(MatchKind, W)>;

    /// The selecting glob of rule file `ctx` for ancestor `anc`, if any.
    spec fn ancestor_match(&self, anc: usize, ctx: usize) -> Option<W>;

    /// Every query issued so far, in order.
    spec fn queries(&self) -> Seq<RuleQuery>;

    /// The number of applicable rule files.
    fn contexts(&self) -> (n: usize)
        ensures
            n == self.spec_contexts(),
    ;

    /// The number of non-empty ancestor directories.
    fn ancestors(&self) -> (m: usize)
        ensures
            m == self.spec_ancestors(),
    ;

    /// Ask rule file `ctx` about the path.
    fn query_path(&mut self, ctx: usize) -> (r: Option<(MatchKind, W)>)
        requires
            ctx < old(self).spec_contexts(),
        ensures
            r == old(self).path_match(ctx),
            final(self).queries() == old(self).queries().push(RuleQuery::Path { ctx }),
            final(self).spec_contexts() == old(self).spec_contexts(),
            final(self).spec_ancestors() == old(self).spec_ancestors(),
            forall|c: usize| #[trigger] final(self).path_match(c) == old(self).path_match(c),
            forall|a: usize, c: usize| #[trigger]
                final(self).ancestor_match(a, c) == old(self).ancestor_match(a, c),
    ;

    /// Ask rule file `ctx` whether it selects ancestor directory `anc`.
    fn query_ancestor(&mut self, anc: usize, ctx: usize) -> (r: Option<W>)
        requires
            anc < old(self).spec_ancestors(),
            ctx < old(self).spec_contexts(),
        ensures
            r == old(self).ancestor_match(anc, ctx),
            final(self).queries() == old(self).queries().push(RuleQuery::Ancestor { anc, ctx }),
            final(self).spec_contexts() == old(self).spec_contexts(),
            final(self).spec_ancestors() == old(self).spec_ancestors(),
            forall|c: usize| #[trigger] final(self).path_match(c) == old(self).path_match(c),
            forall|a: usize, c: usize| #[trigger]
                final(self).ancestor_match(a, c) == old(self).ancestor_match(a, c),
    ;
}

/// The oracle's answers (not its log) are the same in `a` and `b`.
pub open spec fn same_answers<W, O: RuleMatchOracle<W>>(a: &O, b: &O) -> bool {
    &&& a.spec_contexts() == b.spec_contexts()
    &&& a.spec_ancestors() == b.spec_ancestors()
    &&& forall|c: usize| #[trigger] a.path_match(c) == b.path_match(c)
    &&& forall|x: usize, c: usize| #[trigger] a.ancestor_match(x, c) == b.ancestor_match(x, c)
}

/// The path verdicts of an oracle, as a plain function.
pub open spec fn pm_of<W, O: RuleMatchOracle<W>>(o: &O) -> spec_fn(usize) -> Option<(MatchKind, W)> {
    |c: usize| o.path_match(c)
}

/// The ancestor selections of an oracle, as a plain function.
pub open spec fn ai_of<W, O: RuleMatchOracle<W>>(o: &O) -> spec_fn(usize, usize) -> Option<W> {
    |a: usize, c: usize| o.ancestor_match(a, c)
}

// ---------------------------------------------------------------------------
// Declarative semantics
// ---------------------------------------------------------------------------

/// `j` is the deepest of the first `n` rule files whose patterns match the
/// path.
pub open spec fn is_deepest_match<W>(pm: spec_fn(usize) -> Option<(MatchKind, W)>, n: usize, j: usize) -> bool {
    &&& j < n
    &&& pm(j) is Some
    &&& forall|k: usize| j < k < n ==> #[trigger] pm(k) is None
}

/// `a` is the deepest of the `m` ancestors that rule file `ctx` selects.
pub open spec fn is_first_ancestor_hit<W>(ai: spec_fn(usize, usize) -> Option<W>, m: usize, ctx: usize, a: usize) -> bool {
    &&& a < m
    &&& ai(a, ctx) is Some
    &&& forall|b: usize| b < a ==> #[trigger] ai(b, ctx) is None
}

/// `(a, i)` is the first ancestor selection in the cross-file order: the
/// deepest selected ancestor, and for it the shallowest selecting file.
pub open spec fn is_first_cross_hit<W>(
    ai: spec_fn(usize, usize) -> Option<W>,
    n: usize,
    m: usize,
    a: usize,
    i: usize,
) -> bool {
    &&& a < m
    &&& i < n
    &&& ai(a, i) is Some
    &&& forall|b: usize, k: usize| b < a && k < n ==> #[trigger] ai(b, k) is None
    &&& forall|k: usize| k < i ==> #[trigger] ai(a, k) is None
}

/// One rule file's own verdict, with the within-file negation caveat: a
/// negation that matches the path does not deselect it when the same file
/// selects an ancestor directory (the deepest such ancestor is reported).
pub open spec fn spec_file_verdict<W>(
    pm: spec_fn(usize) -> Option<(MatchKind, W)>,
    ai: spec_fn(usize, usize) -> Option<W>,
    m: usize,
    ctx: usize,
) -> Option<Verdict<W>> {
    match pm(ctx) {
        None => None,
        Some((MatchKind::Ignore, w)) => Some(Verdict::Included { ctx, by: By::Path, witness: w }),
        Some((MatchKind::Whitelist, w)) => {
            if exists|a: usize| is_first_ancestor_hit(ai, m, ctx, a) {
                let a = choose|a: usize| is_first_ancestor_hit(ai, m, ctx, a);
                Some(Verdict::Included { ctx, by: By::Ancestor(a), witness: ai(a, ctx)->Some_0 })
            } else {
                Some(Verdict::ExcludedByNegation { ctx, witness: w })
            }
        },
    }
}

/// The verdict of the deepest matching rule file ("a deeper file wins").
pub open spec fn spec_deepest_verdict<W>(
    pm: spec_fn(usize) -> Option<(MatchKind, W)>,
    ai: spec_fn(usize, usize) -> Option<W>,
    n: usize,
    m: usize,
) -> Verdict<W> {
    if exists|j: usize| is_deepest_match(pm, n, j) {
        let j = choose|j: usize| is_deepest_match(pm, n, j);
        match spec_file_verdict(pm, ai, m, j) {
            Some(v) => v,
            None => Verdict::NoMatch,
        }
    } else {
        Verdict::NoMatch
    }
}

/// The cross-file negation caveat: the first ancestor selection in the
/// cross-file order, if any file selects any ancestor directory.
pub open spec fn spec_cross_file<W>(ai: spec_fn(usize, usize) -> Option<W>, n: usize, m: usize) -> Option<Verdict<W>> {
    if exists|a: usize, i: usize| is_first_cross_hit(ai, n, m, a, i) {
        let (a, i) = choose|a: usize, i: usize| is_first_cross_hit(ai, n, m, a, i);
        Some(Verdict::Included { ctx: i, by: By::Ancestor(a), witness: ai(a, i)->Some_0 })
    } else {
        None
    }
}

/// The verdict of the `git` engine: the deepest matching file's verdict,
/// except that a negation never deselects a path when any applicable file
/// selects one of its ancestor directories.
///
/// This mirrors current behavior, including where it departs from Git
/// (each is proved on a concrete input in [`super::reference`]):
///
/// - Scope: an ancestor selection counts even when the ancestor lies at or
///   above the selecting file's own directory, which Git never consults for
///   that file (`*` plus `!secret.key` in `app/config` still selects
///   `app/config/secret.key`).
/// - Isolation: the cross-file caveat asks each file about an ancestor in
///   isolation, so a deeper file re-including that directory (`!b/`) does
///   not stop a shallower `a/b/` from selecting `a/b/x`.
/// - Ancestor negations as path verdicts: a file's path verdict comes from
///   `matched_path_or_any_parents`, so a re-included *directory* (`!x/`) is
///   that file's verdict for every path below it and, being deeper, beats a
///   shallower selection of the path itself (`*.env`).
/// - The empty path: `matched_path_or_any_parents` also tests the rule
///   file's own directory, as the empty path and as a directory. When the
///   file selects it, the file selects every path for which no deeper level
///   has a verdict in that file, and Git does not. A bare `/` line selects
///   every path this way. A directory-only wildcard such as `*/` (anchored
///   with a leading `/` or not) or `**/` selects every file directly in the
///   rule file's directory (`.env` for the root file).
pub open spec fn spec_explain<W>(
    pm: spec_fn(usize) -> Option<(MatchKind, W)>,
    ai: spec_fn(usize, usize) -> Option<W>,
    n: usize,
    m: usize,
) -> Verdict<W> {
    let deepest = spec_deepest_verdict(pm, ai, n, m);
    if deepest is ExcludedByNegation {
        match spec_cross_file(ai, n, m) {
            Some(v) => v,
            None => deepest,
        }
    } else {
        deepest
    }
}

// ---------------------------------------------------------------------------
// Query traces (which matcher queries are issued, in order)
// ---------------------------------------------------------------------------

/// Ancestor queries `0..k` of rule file `ctx`.
pub open spec fn ancestor_scan(ctx: usize, k: nat) -> Seq<RuleQuery> {
    Seq::new(k, |a: int| RuleQuery::Ancestor { anc: a as usize, ctx })
}

/// Ancestor `anc` asked of rule files `0..k`.
pub open spec fn ancestor_row(anc: usize, k: nat) -> Seq<RuleQuery> {
    Seq::new(k, |i: int| RuleQuery::Ancestor { anc, ctx: i as usize })
}

/// How many ancestors the within-file caveat of rule file `ctx` asks about:
/// up to and including the first selected one, or all of them.
pub open spec fn file_scan_len<W>(ai: spec_fn(usize, usize) -> Option<W>, m: usize, ctx: usize) -> nat {
    if exists|a: usize| is_first_ancestor_hit(ai, m, ctx, a) {
        ((choose|a: usize| is_first_ancestor_hit(ai, m, ctx, a)) + 1) as nat
    } else {
        m as nat
    }
}

/// The queries evaluating rule file `ctx`: its path query, then (only for a
/// negation match) the within-file ancestor scan.
pub open spec fn file_trace<W>(
    pm: spec_fn(usize) -> Option<(MatchKind, W)>,
    ai: spec_fn(usize, usize) -> Option<W>,
    m: usize,
    ctx: usize,
) -> Seq<RuleQuery> {
    seq![RuleQuery::Path { ctx }] + if pm(ctx) matches Some((MatchKind::Whitelist, _)) {
        ancestor_scan(ctx, file_scan_len(ai, m, ctx))
    } else {
        Seq::empty()
    }
}

/// The queries evaluating rule files `0..k`, in order.
pub open spec fn files_trace<W>(
    pm: spec_fn(usize) -> Option<(MatchKind, W)>,
    ai: spec_fn(usize, usize) -> Option<W>,
    m: usize,
    k: usize,
) -> Seq<RuleQuery>
    decreases k,
{
    if k == 0 {
        Seq::empty()
    } else {
        files_trace(pm, ai, m, (k - 1) as usize) + file_trace(pm, ai, m, (k - 1) as usize)
    }
}

/// Full ancestor rows `0..a`, each asked of all `n` rule files.
pub open spec fn rows_trace(n: usize, a: nat) -> Seq<RuleQuery>
    decreases a,
{
    if a == 0 {
        Seq::empty()
    } else {
        rows_trace(n, (a - 1) as nat) + ancestor_row((a - 1) as usize, n as nat)
    }
}

/// The queries of the cross-file caveat: ancestors deepest first, files
/// shallowest first, up to and including the first selection.
pub open spec fn cross_trace<W>(ai: spec_fn(usize, usize) -> Option<W>, n: usize, m: usize) -> Seq<RuleQuery> {
    if exists|a: usize, i: usize| is_first_cross_hit(ai, n, m, a, i) {
        let (a, i) = choose|a: usize, i: usize| is_first_cross_hit(ai, n, m, a, i);
        rows_trace(n, a as nat) + ancestor_row(a, (i + 1) as nat)
    } else {
        rows_trace(n, m as nat)
    }
}

/// Every query [`explain`] issues: each rule file in order, then the
/// cross-file scan only when the deepest verdict is a negation.
pub open spec fn explain_trace<W>(
    pm: spec_fn(usize) -> Option<(MatchKind, W)>,
    ai: spec_fn(usize, usize) -> Option<W>,
    n: usize,
    m: usize,
) -> Seq<RuleQuery> {
    files_trace(pm, ai, m, n) + if spec_deepest_verdict(pm, ai, n, m) is ExcludedByNegation {
        cross_trace(ai, n, m)
    } else {
        Seq::empty()
    }
}

// ---------------------------------------------------------------------------
// Uniqueness of the chosen witnesses
// ---------------------------------------------------------------------------

proof fn lemma_same_answers_same_views<W, O: RuleMatchOracle<W>>(a: &O, b: &O)
    requires
        same_answers(a, b),
    ensures
        pm_of(a) == pm_of(b),
        ai_of(a) == ai_of(b),
{
    assert(pm_of(a) =~= pm_of(b));
    assert(ai_of(a) =~= ai_of(b));
}

/// The deepest matching rule file is unique.
pub proof fn lemma_deepest_unique<W>(pm: spec_fn(usize) -> Option<(MatchKind, W)>, n: usize, j1: usize, j2: usize)
    requires
        is_deepest_match(pm, n, j1),
        is_deepest_match(pm, n, j2),
    ensures
        j1 == j2,
{
    if j1 < j2 {
        assert(pm(j2) is None);
    } else if j2 < j1 {
        assert(pm(j1) is None);
    }
}

/// The first selected ancestor of a rule file is unique.
pub proof fn lemma_first_ancestor_hit_unique<W>(ai: spec_fn(usize, usize) -> Option<W>, m: usize, ctx: usize, a1: usize, a2: usize)
    requires
        is_first_ancestor_hit(ai, m, ctx, a1),
        is_first_ancestor_hit(ai, m, ctx, a2),
    ensures
        a1 == a2,
{
    if a1 < a2 {
        assert(ai(a1, ctx) is None);
    } else if a2 < a1 {
        assert(ai(a2, ctx) is None);
    }
}

/// The first cross-file ancestor selection is unique.
pub proof fn lemma_first_cross_hit_unique<W>(
    ai: spec_fn(usize, usize) -> Option<W>,
    n: usize,
    m: usize,
    a1: usize,
    i1: usize,
    a2: usize,
    i2: usize,
)
    requires
        is_first_cross_hit(ai, n, m, a1, i1),
        is_first_cross_hit(ai, n, m, a2, i2),
    ensures
        a1 == a2 && i1 == i2,
{
    if a1 < a2 {
        assert(ai(a1, i1) is None);
    } else if a2 < a1 {
        assert(ai(a2, i2) is None);
    } else if i1 < i2 {
        assert(ai(a1, i1) is None);
    } else if i2 < i1 {
        assert(ai(a2, i2) is None);
    }
}

// ---------------------------------------------------------------------------
// Executable kernel
// ---------------------------------------------------------------------------

/// Evaluate one rule file: its path verdict, and for a negation the
/// within-file ancestor scan (the former `evaluate_against_context`).
fn evaluate_file<W, O: RuleMatchOracle<W>>(o: &mut O, ctx: usize) -> (r: Option<Verdict<W>>)
    requires
        ctx < old(o).spec_contexts(),
    ensures
        same_answers(final(o), old(o)),
        r == spec_file_verdict(pm_of(old(o)), ai_of(old(o)), old(o).spec_ancestors(), ctx),
        final(o).queries() == old(o).queries() + file_trace(
            pm_of(old(o)),
            ai_of(old(o)),
            old(o).spec_ancestors(),
            ctx,
        ),
{
    let ghost o0 = *o;
    let ghost pm = pm_of(&o0);
    let ghost ai = ai_of(&o0);
    let ghost m0 = o0.spec_ancestors();
    match o.query_path(ctx) {
        None => {
            assert(o.queries() =~= o0.queries() + file_trace(pm, ai, m0, ctx));
            None
        },
        Some((MatchKind::Ignore, w)) => {
            assert(o.queries() =~= o0.queries() + file_trace(pm, ai, m0, ctx));
            Some(Verdict::Included { ctx, by: By::Path, witness: w })
        },
        Some((MatchKind::Whitelist, w)) => {
            let m = o.ancestors();
            let mut a: usize = 0;
            assert(ancestor_scan(ctx, 0) =~= Seq::<RuleQuery>::empty());
            while a < m
                invariant
                    a <= m,
                    m == m0,
                    ctx < o0.spec_contexts(),
                    o0 == *old(o),
                    m0 == o0.spec_ancestors(),
                    pm == pm_of(&o0),
                    ai == ai_of(&o0),
                    same_answers(o, &o0),
                    pm(ctx) == Some((MatchKind::Whitelist, w)),
                    forall|b: usize| b < a ==> #[trigger] ai(b, ctx) is None,
                    o.queries() == o0.queries().push(RuleQuery::Path { ctx }) + ancestor_scan(ctx, a as nat),
                decreases m - a,
            {
                let hit = o.query_ancestor(a, ctx);
                proof {
                    assert(ancestor_scan(ctx, (a + 1) as nat) =~= ancestor_scan(ctx, a as nat).push(
                        RuleQuery::Ancestor { anc: a, ctx },
                    ));
                }
                if let Some(aw) = hit {
                    proof {
                        assert(is_first_ancestor_hit(ai, m0, ctx, a));
                        let c = choose|c: usize| is_first_ancestor_hit(ai, m0, ctx, c);
                        lemma_first_ancestor_hit_unique(ai, m0, ctx, a, c);
                        assert(file_scan_len(ai, m0, ctx) == a + 1);
                        assert(o.queries() =~= o0.queries() + file_trace(pm, ai, m0, ctx));
                    }
                    return Some(Verdict::Included { ctx, by: By::Ancestor(a), witness: aw });
                }
                a += 1;
            }
            proof {
                assert(!exists|c: usize| is_first_ancestor_hit(ai, m0, ctx, c));
                assert(o.queries() =~= o0.queries() + file_trace(pm, ai, m0, ctx));
            }
            Some(Verdict::ExcludedByNegation { ctx, witness: w })
        },
    }
}

/// Ask every ancestor of every rule file, ancestors deepest first and files
/// shallowest first, stopping at the first selection (the former
/// `cross_file_ancestor_check`).
fn cross_file_check<W, O: RuleMatchOracle<W>>(o: &mut O) -> (r: Option<Verdict<W>>)
    ensures
        same_answers(final(o), old(o)),
        r == spec_cross_file(ai_of(old(o)), old(o).spec_contexts(), old(o).spec_ancestors()),
        final(o).queries() == old(o).queries() + cross_trace(
            ai_of(old(o)),
            old(o).spec_contexts(),
            old(o).spec_ancestors(),
        ),
{
    let ghost o0 = *o;
    let ghost ai = ai_of(&o0);
    let n = o.contexts();
    let m = o.ancestors();
    let mut a: usize = 0;
    assert(rows_trace(n, 0) =~= Seq::<RuleQuery>::empty());
    assert(o.queries() =~= o0.queries() + rows_trace(n, 0));
    while a < m
        invariant
            a <= m,
            o0 == *old(o),
            ai == ai_of(&o0),
            n == o0.spec_contexts(),
            m == o0.spec_ancestors(),
            same_answers(o, &o0),
            forall|b: usize, k: usize| b < a && k < n ==> #[trigger] ai(b, k) is None,
            o.queries() == o0.queries() + rows_trace(n, a as nat),
        decreases m - a,
    {
        let mut i: usize = 0;
        assert(ancestor_row(a, 0) =~= Seq::<RuleQuery>::empty());
        assert(o.queries() =~= o0.queries() + rows_trace(n, a as nat) + ancestor_row(a, 0));
        while i < n
            invariant
                i <= n,
                a < m,
                o0 == *old(o),
                ai == ai_of(&o0),
                n == o0.spec_contexts(),
                m == o0.spec_ancestors(),
                same_answers(o, &o0),
                forall|b: usize, k: usize| b < a && k < n ==> #[trigger] ai(b, k) is None,
                forall|k: usize| k < i ==> #[trigger] ai(a, k) is None,
                o.queries() == o0.queries() + rows_trace(n, a as nat) + ancestor_row(a, i as nat),
            decreases n - i,
        {
            let hit = o.query_ancestor(a, i);
            proof {
                assert(ancestor_row(a, (i + 1) as nat) =~= ancestor_row(a, i as nat).push(
                    RuleQuery::Ancestor { anc: a, ctx: i },
                ));
            }
            if let Some(w) = hit {
                proof {
                    assert(is_first_cross_hit(ai, n, m, a, i));
                    let (b, k) = choose|b: usize, k: usize| is_first_cross_hit(ai, n, m, b, k);
                    lemma_first_cross_hit_unique(ai, n, m, a, i, b, k);
                    assert(o.queries() =~= o0.queries() + cross_trace(ai, n, m));
                }
                return Some(Verdict::Included { ctx: i, by: By::Ancestor(a), witness: w });
            }
            i += 1;
        }
        proof {
            assert forall|b: usize, k: usize| b < a + 1 && k < n implies #[trigger] ai(b, k) is None by {
                if b == a {
                    assert(ai(a, k) is None);
                }
            }
            assert(rows_trace(n, (a + 1) as nat) == rows_trace(n, a as nat) + ancestor_row(a, n as nat));
            assert(o.queries() =~= o0.queries() + rows_trace(n, (a + 1) as nat));
        }
        a += 1;
    }
    proof {
        if exists|b: usize, k: usize| is_first_cross_hit(ai, n, m, b, k) {
            let (b, k) = choose|b: usize, k: usize| is_first_cross_hit(ai, n, m, b, k);
            assert(ai(b, k) is None);
        }
    }
    None
}

/// Adding rule file `i` changes the deepest verdict only when `i` matches.
pub proof fn lemma_deepest_verdict_step<W>(
    pm: spec_fn(usize) -> Option<(MatchKind, W)>,
    ai: spec_fn(usize, usize) -> Option<W>,
    m: usize,
    i: usize,
)
    requires
        i < usize::MAX,
    ensures
        spec_deepest_verdict(pm, ai, (i + 1) as usize, m) == match spec_file_verdict(pm, ai, m, i) {
            Some(v) => v,
            None => spec_deepest_verdict(pm, ai, i, m),
        },
{
    let i1 = (i + 1) as usize;
    if pm(i) is Some {
        assert(is_deepest_match(pm, i1, i));
        let j = choose|j: usize| is_deepest_match(pm, i1, j);
        lemma_deepest_unique(pm, i1, i, j);
    } else {
        if exists|j: usize| is_deepest_match(pm, i, j) {
            let j = choose|j: usize| is_deepest_match(pm, i, j);
            assert(is_deepest_match(pm, i1, j));
            let j1 = choose|j: usize| is_deepest_match(pm, i1, j);
            lemma_deepest_unique(pm, i1, j, j1);
        }
        if exists|j: usize| is_deepest_match(pm, i1, j) {
            let j = choose|j: usize| is_deepest_match(pm, i1, j);
            assert(j != i);
            assert(is_deepest_match(pm, i, j));
        }
    }
}

/// Evaluate the applicable rule files (the `git` engine's `explain_path`).
///
/// Every rule file is evaluated, shallowest first, and the last one that
/// matches decides; only when that decision is a negation does the
/// cross-file caveat scan run. The query log proves the kernel issues
/// exactly these matcher queries, in this order.
pub fn explain<W, O: RuleMatchOracle<W>>(o: &mut O) -> (v: Verdict<W>)
    ensures
        same_answers(final(o), old(o)),
        v == spec_explain(pm_of(old(o)), ai_of(old(o)), old(o).spec_contexts(), old(o).spec_ancestors()),
        final(o).queries() == old(o).queries() + explain_trace(
            pm_of(old(o)),
            ai_of(old(o)),
            old(o).spec_contexts(),
            old(o).spec_ancestors(),
        ),
{
    let ghost o0 = *o;
    let ghost pm = pm_of(&o0);
    let ghost ai = ai_of(&o0);
    let ghost m0 = o0.spec_ancestors();
    let n = o.contexts();
    let mut last: Verdict<W> = Verdict::NoMatch;
    let mut i: usize = 0;
    proof {
        assert(!exists|j: usize| is_deepest_match(pm, 0, j));
        assert(o.queries() =~= o0.queries() + files_trace(pm, ai, m0, 0));
    }
    while i < n
        invariant
            i <= n,
            o0 == *old(o),
            pm == pm_of(&o0),
            ai == ai_of(&o0),
            m0 == o0.spec_ancestors(),
            n == o0.spec_contexts(),
            same_answers(o, &o0),
            last == spec_deepest_verdict(pm, ai, i, m0),
            o.queries() == o0.queries() + files_trace(pm, ai, m0, i),
        decreases n - i,
    {
        let ghost pre = *o;
        let r = evaluate_file(o, i);
        proof {
            lemma_same_answers_same_views(&pre, &o0);
            lemma_deepest_verdict_step(pm, ai, m0, i);
            assert(files_trace(pm, ai, m0, (i + 1) as usize) == files_trace(pm, ai, m0, i) + file_trace(
                pm,
                ai,
                m0,
                i,
            ));
            assert(o.queries() =~= o0.queries() + files_trace(pm, ai, m0, (i + 1) as usize));
        }
        if let Some(v) = r {
            last = v;
        }
        i += 1;
    }
    if matches!(last, Verdict::ExcludedByNegation { .. }) {
        let ghost pre = *o;
        let cross = cross_file_check(o);
        proof {
            lemma_same_answers_same_views(&pre, &o0);
            assert(o.queries() =~= o0.queries() + explain_trace(pm, ai, n, m0));
        }
        if let Some(v) = cross {
            return v;
        }
    } else {
        proof {
            assert(o.queries() =~= o0.queries() + explain_trace(pm, ai, n, m0));
        }
    }
    last
}

/// Evaluate only rule file 0, the repository-root file (the
/// `claude-2026-04` engine's `evaluate_root_only_path`).
///
/// `theorem_single_file_explain` shows that this is [`explain`]'s verdict
/// for the root file alone: with one rule file the cross-file caveat never
/// changes the result, so it is not run (and its queries are not issued).
pub fn explain_root_only<W, O: RuleMatchOracle<W>>(o: &mut O) -> (v: Verdict<W>)
    ensures
        same_answers(final(o), old(o)),
        old(o).spec_contexts() == 0 ==> v is NoMatch && final(o).queries() == old(o).queries(),
        old(o).spec_contexts() > 0 ==> {
            &&& v == spec_explain(pm_of(old(o)), ai_of(old(o)), 1, old(o).spec_ancestors())
            &&& final(o).queries() == old(o).queries() + file_trace(
                pm_of(old(o)),
                ai_of(old(o)),
                old(o).spec_ancestors(),
                0,
            )
        },
{
    if o.contexts() == 0 {
        return Verdict::NoMatch;
    }
    let ghost o0 = *o;
    let r = evaluate_file(o, 0);
    proof {
        theorem_single_file_explain(pm_of(&o0), ai_of(&o0), o0.spec_ancestors());
    }
    match r {
        Some(v) => v,
        None => Verdict::NoMatch,
    }
}

// ---------------------------------------------------------------------------
// Existence of the chosen witnesses
// ---------------------------------------------------------------------------

/// Some rule file matches, so a deepest matching one exists.
pub proof fn lemma_deepest_exists<W>(pm: spec_fn(usize) -> Option<(MatchKind, W)>, n: usize, i: usize)
    requires
        i < n,
        pm(i) is Some,
    ensures
        exists|j: usize| is_deepest_match(pm, n, j),
    decreases n - i,
{
    if exists|k: usize| i < k < n && #[trigger] pm(k) is Some {
        let k = choose|k: usize| i < k < n && #[trigger] pm(k) is Some;
        lemma_deepest_exists(pm, n, k);
    } else {
        assert(is_deepest_match(pm, n, i));
    }
}

/// Some ancestor is selected by `ctx`, so a first one exists.
pub proof fn lemma_first_ancestor_hit_exists<W>(ai: spec_fn(usize, usize) -> Option<W>, m: usize, ctx: usize, a: usize)
    requires
        a < m,
        ai(a, ctx) is Some,
    ensures
        exists|b: usize| is_first_ancestor_hit(ai, m, ctx, b),
    decreases a,
{
    if exists|b: usize| b < a && #[trigger] ai(b, ctx) is Some {
        let b = choose|b: usize| b < a && #[trigger] ai(b, ctx) is Some;
        lemma_first_ancestor_hit_exists(ai, m, ctx, b);
    } else {
        assert(is_first_ancestor_hit(ai, m, ctx, a));
    }
}

/// Some file selects some ancestor, so a first cross-file selection exists.
pub proof fn lemma_first_cross_hit_exists<W>(ai: spec_fn(usize, usize) -> Option<W>, n: usize, m: usize, a: usize, i: usize)
    requires
        a < m,
        i < n,
        ai(a, i) is Some,
    ensures
        exists|b: usize, k: usize| is_first_cross_hit(ai, n, m, b, k),
    decreases a, i,
{
    if exists|b: usize, k: usize| b < a && k < n && #[trigger] ai(b, k) is Some {
        let (b, k) = choose|b: usize, k: usize| b < a && k < n && #[trigger] ai(b, k) is Some;
        lemma_first_cross_hit_exists(ai, n, m, b, k);
    } else if exists|k: usize| k < i && #[trigger] ai(a, k) is Some {
        let k = choose|k: usize| k < i && #[trigger] ai(a, k) is Some;
        lemma_first_cross_hit_exists(ai, n, m, a, k);
    } else {
        assert(is_first_cross_hit(ai, n, m, a, i));
    }
}

// ---------------------------------------------------------------------------
// Theorems
// ---------------------------------------------------------------------------

/// Root-only reuse: with a single rule file the cross-file caveat is a
/// no-op, so the verdict is that file's own verdict (or `NoMatch`).
pub proof fn theorem_single_file_explain<W>(
    pm: spec_fn(usize) -> Option<(MatchKind, W)>,
    ai: spec_fn(usize, usize) -> Option<W>,
    m: usize,
)
    ensures
        spec_explain(pm, ai, 1, m) == match spec_file_verdict(pm, ai, m, 0) {
            Some(v) => v,
            None => Verdict::NoMatch,
        },
{
    lemma_deepest_verdict_step(pm, ai, m, 0);
    assert(!exists|j: usize| is_deepest_match(pm, 0, j));
    if exists|a: usize, i: usize| is_first_cross_hit(ai, 1, m, a, i) {
        let (a, i) = choose|a: usize, i: usize| is_first_cross_hit(ai, 1, m, a, i);
        assert(i == 0);
        lemma_first_ancestor_hit_exists(ai, m, 0, a);
    }
}

/// Totality of "no match": the verdict is `NoMatch` exactly when no
/// applicable rule file matches the path. The caveats never invent a match
/// and never drop one.
pub proof fn theorem_no_match_iff_no_file_matches<W>(
    pm: spec_fn(usize) -> Option<(MatchKind, W)>,
    ai: spec_fn(usize, usize) -> Option<W>,
    n: usize,
    m: usize,
)
    ensures
        spec_explain(pm, ai, n, m) is NoMatch <==> forall|i: usize| i < n ==> #[trigger] pm(i) is None,
{
    if exists|i: usize| i < n && #[trigger] pm(i) is Some {
        let i = choose|i: usize| i < n && #[trigger] pm(i) is Some;
        lemma_deepest_exists(pm, n, i);
        let j = choose|j: usize| is_deepest_match(pm, n, j);
        assert(spec_file_verdict(pm, ai, m, j) is Some);
    } else {
        if exists|j: usize| is_deepest_match(pm, n, j) {
            let j = choose|j: usize| is_deepest_match(pm, n, j);
            assert(pm(j) is Some);
        }
    }
}

/// A deeper file wins: once rule file `j` matches the path, the path
/// verdicts of every shallower file are irrelevant (only files `j..n` and
/// the ancestor selections can influence the verdict).
pub proof fn theorem_deeper_file_wins<W>(
    pm1: spec_fn(usize) -> Option<(MatchKind, W)>,
    pm2: spec_fn(usize) -> Option<(MatchKind, W)>,
    ai: spec_fn(usize, usize) -> Option<W>,
    n: usize,
    m: usize,
    j: usize,
)
    requires
        j < n,
        pm1(j) is Some,
        forall|k: usize| j <= k < n ==> #[trigger] pm1(k) == pm2(k),
    ensures
        spec_explain(pm1, ai, n, m) == spec_explain(pm2, ai, n, m),
{
    lemma_deepest_exists(pm1, n, j);
    let d1 = choose|d: usize| is_deepest_match(pm1, n, d);
    if d1 < j {
        assert(pm1(j) is None);
    }
    assert(pm1(d1) == pm2(d1));
    assert forall|k: usize| d1 < k < n implies #[trigger] pm2(k) is None by {
        assert(pm1(k) == pm2(k));
    }
    assert(is_deepest_match(pm2, n, d1));
    let d2 = choose|d: usize| is_deepest_match(pm2, n, d);
    lemma_deepest_unique(pm2, n, d1, d2);
}

/// When the deepest matching rule file selects the path, that file and its
/// selecting glob are the verdict, whatever shallower files say.
pub proof fn theorem_deepest_selection_decides<W>(
    pm: spec_fn(usize) -> Option<(MatchKind, W)>,
    ai: spec_fn(usize, usize) -> Option<W>,
    n: usize,
    m: usize,
    j: usize,
    w: W,
)
    requires
        is_deepest_match(pm, n, j),
        pm(j) == Some((MatchKind::Ignore, w)),
    ensures
        spec_explain(pm, ai, n, m) == (Verdict::Included { ctx: j, by: By::Path, witness: w }),
{
    let d = choose|d: usize| is_deepest_match(pm, n, d);
    lemma_deepest_unique(pm, n, j, d);
}

/// Negation-caveat soundness (the documented invariant of `explain`): a
/// negation deselects the path only when it is the deepest matching rule
/// file's verdict and no applicable rule file selects any ancestor
/// directory of the path. In particular a negation never deselects a file
/// inside a directory that any applicable rule file selects.
pub proof fn theorem_negation_needs_no_selected_ancestor<W>(
    pm: spec_fn(usize) -> Option<(MatchKind, W)>,
    ai: spec_fn(usize, usize) -> Option<W>,
    n: usize,
    m: usize,
    ctx: usize,
    w: W,
)
    requires
        spec_explain(pm, ai, n, m) == (Verdict::ExcludedByNegation { ctx, witness: w }),
    ensures
        is_deepest_match(pm, n, ctx),
        pm(ctx) == Some((MatchKind::Whitelist, w)),
        forall|a: usize, i: usize| a < m && i < n ==> #[trigger] ai(a, i) is None,
{
    assert forall|a: usize, i: usize| a < m && i < n implies #[trigger] ai(a, i) is None by {
        if ai(a, i) is Some {
            lemma_first_cross_hit_exists(ai, n, m, a, i);
        }
    }
    if !exists|j: usize| is_deepest_match(pm, n, j) {
        assert(spec_explain(pm, ai, n, m) is NoMatch);
    }
}

/// Negation-caveat completeness: when the deepest matching rule file's
/// verdict is a negation and no applicable file selects an ancestor
/// directory, the path is deselected by that negation (a negation of a
/// plain file pattern still works).
pub proof fn theorem_negation_deselects_without_selected_ancestor<W>(
    pm: spec_fn(usize) -> Option<(MatchKind, W)>,
    ai: spec_fn(usize, usize) -> Option<W>,
    n: usize,
    m: usize,
    j: usize,
    w: W,
)
    requires
        is_deepest_match(pm, n, j),
        pm(j) == Some((MatchKind::Whitelist, w)),
        forall|a: usize, i: usize| a < m && i < n ==> #[trigger] ai(a, i) is None,
    ensures
        spec_explain(pm, ai, n, m) == (Verdict::ExcludedByNegation { ctx: j, witness: w }),
{
    let d = choose|d: usize| is_deepest_match(pm, n, d);
    lemma_deepest_unique(pm, n, j, d);
    if exists|a: usize| is_first_ancestor_hit(ai, m, j, a) {
        let a = choose|a: usize| is_first_ancestor_hit(ai, m, j, a);
        assert(ai(a, j) is None);
    }
    if exists|a: usize, i: usize| is_first_cross_hit(ai, n, m, a, i) {
        let (a, i) = choose|a: usize, i: usize| is_first_cross_hit(ai, n, m, a, i);
        assert(ai(a, i) is None);
    }
}

/// Provenance: every verdict names a rule file and a query that really
/// produced its witness, so the file, line and pattern printed by `info`
/// and `list -v` belong to the deciding glob. An ancestor selection is
/// reported either by the deepest matching file itself (its deepest
/// selected ancestor) or, failing that, by the cross-file order (the
/// deepest selected ancestor, then the shallowest file selecting it).
pub proof fn theorem_verdict_provenance<W>(
    pm: spec_fn(usize) -> Option<(MatchKind, W)>,
    ai: spec_fn(usize, usize) -> Option<W>,
    n: usize,
    m: usize,
)
    ensures
        match spec_explain(pm, ai, n, m) {
            Verdict::NoMatch => true,
            Verdict::Included { ctx, by: By::Path, witness } => {
                &&& is_deepest_match(pm, n, ctx)
                &&& pm(ctx) == Some((MatchKind::Ignore, witness))
            },
            Verdict::Included { ctx, by: By::Ancestor(a), witness } => {
                &&& ctx < n
                &&& a < m
                &&& ai(a, ctx) == Some(witness)
                &&& exists|j: usize|
                    {
                        &&& #[trigger] is_deepest_match(pm, n, j)
                        &&& pm(j) matches Some((MatchKind::Whitelist, _))
                        &&& {
                            ||| ctx == j && is_first_ancestor_hit(ai, m, j, a)
                            ||| !(exists|b: usize| is_first_ancestor_hit(ai, m, j, b))
                                && is_first_cross_hit(ai, n, m, a, ctx)
                        }
                    }
            },
            Verdict::ExcludedByNegation { ctx, witness } => {
                &&& is_deepest_match(pm, n, ctx)
                &&& pm(ctx) == Some((MatchKind::Whitelist, witness))
            },
        },
{
    if exists|j: usize| is_deepest_match(pm, n, j) {
        let j = choose|j: usize| is_deepest_match(pm, n, j);
        if exists|a: usize| is_first_ancestor_hit(ai, m, j, a) {
            let a = choose|a: usize| is_first_ancestor_hit(ai, m, j, a);
            assert(is_first_ancestor_hit(ai, m, j, a));
        }
        if exists|a: usize, i: usize| is_first_cross_hit(ai, n, m, a, i) {
            let (a, i) = choose|a: usize, i: usize| is_first_cross_hit(ai, n, m, a, i);
            assert(is_first_cross_hit(ai, n, m, a, i));
        }
    }
}

} // verus!
