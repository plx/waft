//! Verified kernel: validation severity gates.
//!
//! `src/validate.rs` collects a `ValidationReport`: a list of issues, each
//! with a severity. This kernel decides, from the severities alone:
//!
//! - whether the report has errors and how many ([`has_errors`],
//!   [`error_count`]), which `ValidationReport::has_errors` and
//!   `ValidationReport::error_count` in `src/model.rs` call;
//! - what a command does with the report ([`validation_view`]): which issues
//!   it prints and in what order, whether it stops with
//!   `Error::Validation { error_count }`, and whether `waft validate` prints
//!   `validation passed`. `copy`, `list` and `info` use
//!   [`ReportMode::Preflight`]; `waft validate` uses [`ReportMode::Validate`].
//!
//! # Theorems
//!
//! - [`error_count`] equals `spec_error_count`, the length of the
//!   subsequence of `Error` severities, and [`has_errors`] equals
//!   `spec_has_errors`, "some severity is `Error`".
//!   `theorem_has_errors_iff_count_positive` proves the two agree:
//!   `has_errors` holds exactly when `error_count > 0`, and the count never
//!   exceeds the number of issues.
//! - [`validation_view`] meets `spec_view_ok`: `shown` lists, in increasing
//!   order and without repeats, exactly the issues `spec_shown` selects;
//!   the outcome is `Abort { error_count }` with the exact error count
//!   exactly when some issue is an error, and `Proceed` otherwise; and
//!   `announce_pass` holds exactly for `waft validate` without `--quiet` on a
//!   report with no errors.
//! - `theorem_gate_is_mode_independent`: the outcome does not depend on
//!   the command or on `--quiet`. So `waft validate` exits non-zero exactly
//!   when `copy`, `list` and `info` would stop at validation, with the same
//!   `error_count`.
//! - `theorem_visibility`: errors are always printed by `waft validate`,
//!   even with `--quiet`, and by a preflight that stops; warnings are never
//!   printed under `--quiet`; a preflight that stops prints only errors; a
//!   preflight that proceeds prints only warnings (there are no errors to
//!   print); and `validation passed` is never printed next to an error.
//!
//! # Trust assumptions
//!
//! - Production passes the issues' severities in report order, mapping
//!   `ValidationSeverity::{Warning, Error}` one to one onto [`Severity`]
//!   (an exhaustive match in `ValidationReport::severities`).
//! - Production prints `issues[k]` for each `k` in `shown`, in order, with the
//!   command's format string, then acts on `outcome` and `announce_pass`.
//!   The message text and the per-command formats stay in production.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// Severity of one validation issue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Reported, but never blocks.
    Warning,
    /// Blocks `copy`, `list` and `info`, and fails `waft validate`.
    Error,
}

/// Whether a severity is `Error`.
pub open spec fn spec_is_error(s: Severity) -> bool {
    s is Error
}

/// Some issue is an error.
pub open spec fn spec_has_errors(s: Seq<Severity>) -> bool {
    exists|i: int| 0 <= i < s.len() && spec_is_error(#[trigger] s[i])
}

/// The number of error issues: the length of the subsequence of errors.
pub open spec fn spec_error_count(s: Seq<Severity>) -> nat {
    s.filter(|x: Severity| spec_is_error(x)).len()
}

/// Counting the errors of a sequence one more issue at a time.
proof fn lemma_error_count_push(s: Seq<Severity>, x: Severity)
    ensures
        spec_error_count(s.push(x)) == spec_error_count(s) + if spec_is_error(x) {
            1nat
        } else {
            0nat
        },
{
    reveal(Seq::filter);
    assert(s.push(x).drop_last() =~= s);
}

/// A sequence with no errors has an error count of zero.
proof fn lemma_no_errors_count_zero(s: Seq<Severity>)
    requires
        !spec_has_errors(s),
    ensures
        spec_error_count(s) == 0,
    decreases s.len(),
{
    reveal(Seq::filter);
    if s.len() > 0 {
        let t = s.drop_last();
        assert(!spec_has_errors(t)) by {
            if spec_has_errors(t) {
                let i = choose|i: int| 0 <= i < t.len() && spec_is_error(#[trigger] t[i]);
                assert(s[i] == t[i]);
            }
        }
        lemma_no_errors_count_zero(t);
        assert(s == t.push(s.last()));
        lemma_error_count_push(t, s.last());
    }
}

/// A sequence with an error has a positive error count.
proof fn lemma_errors_count_positive(s: Seq<Severity>, i: int)
    requires
        0 <= i < s.len(),
        spec_is_error(s[i]),
    ensures
        spec_error_count(s) > 0,
    decreases s.len(),
{
    let t = s.drop_last();
    assert(s == t.push(s.last()));
    lemma_error_count_push(t, s.last());
    if i < s.len() - 1 {
        assert(t[i] == s[i]);
        lemma_errors_count_positive(t, i);
    }
}

/// `has_errors` holds exactly when the error count is positive, and the
/// count never exceeds the number of issues.
pub proof fn theorem_has_errors_iff_count_positive(s: Seq<Severity>)
    ensures
        spec_has_errors(s) <==> spec_error_count(s) > 0,
        spec_error_count(s) <= s.len(),
{
    s.lemma_filter_len(|x: Severity| spec_is_error(x));
    if spec_has_errors(s) {
        let i = choose|i: int| 0 <= i < s.len() && spec_is_error(#[trigger] s[i]);
        lemma_errors_count_positive(s, i);
    } else {
        lemma_no_errors_count_zero(s);
    }
}

/// The number of error-severity issues.
pub fn error_count(severities: &[Severity]) -> (n: usize)
    ensures
        n as nat == spec_error_count(severities@),
{
    let mut n: usize = 0;
    let mut i: usize = 0;
    while i < severities.len()
        invariant
            i <= severities@.len(),
            n as nat == spec_error_count(severities@.subrange(0, i as int)),
            n <= i,
        decreases severities.len() - i,
    {
        let ghost prefix = severities@.subrange(0, i as int);
        assert(severities@.subrange(0, i + 1) =~= prefix.push(severities@[i as int]));
        proof {
            lemma_error_count_push(prefix, severities@[i as int]);
        }
        match severities[i] {
            Severity::Error => n += 1,
            Severity::Warning => {},
        }
        i += 1;
    }
    assert(severities@.subrange(0, severities@.len() as int) =~= severities@);
    n
}

/// Whether any issue has error severity.
pub fn has_errors(severities: &[Severity]) -> (r: bool)
    ensures
        r == spec_has_errors(severities@),
{
    let mut i: usize = 0;
    while i < severities.len()
        invariant
            i <= severities@.len(),
            forall|k: int| 0 <= k < i ==> !spec_is_error(#[trigger] severities@[k]),
        decreases severities.len() - i,
    {
        match severities[i] {
            Severity::Error => return true,
            Severity::Warning => {},
        }
        i += 1;
    }
    false
}

/// Which command is presenting the report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportMode {
    /// `copy`, `list` and `info`: validation gates the command before it
    /// acts.
    Preflight,
    /// `waft validate`: the report is the output.
    Validate,
}

/// What a command does after validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateOutcome {
    /// Carry on.
    Proceed,
    /// Stop with `Error::Validation { error_count }`.
    Abort {
        /// The number of error issues; at least one.
        error_count: usize,
    },
}

/// How a command presents a validation report.
#[derive(Clone, Debug)]
pub struct ValidationView {
    /// Indices of the issues to print, in report order.
    pub shown: Vec<usize>,
    /// Whether the command stops here.
    pub outcome: GateOutcome,
    /// Whether to print `validation passed` after the issues.
    pub announce_pass: bool,
}

/// Whether issue `i` is printed.
///
/// A preflight that stops prints the errors; one that proceeds prints the
/// warnings unless `--quiet`. `waft validate` prints errors always and
/// warnings unless `--quiet`.
pub open spec fn spec_shown(s: Seq<Severity>, mode: ReportMode, quiet: bool, i: int) -> bool {
    match mode {
        ReportMode::Preflight => if spec_has_errors(s) {
            spec_is_error(s[i])
        } else {
            !quiet
        },
        ReportMode::Validate => spec_is_error(s[i]) || !quiet,
    }
}

/// The outcome for a report: stop with the exact error count when there is
/// an error, otherwise proceed.
pub open spec fn spec_outcome(s: Seq<Severity>) -> GateOutcome {
    if spec_has_errors(s) {
        GateOutcome::Abort { error_count: spec_error_count(s) as usize }
    } else {
        GateOutcome::Proceed
    }
}

/// `view` is the presentation of the report `s` for `mode` and `quiet`.
pub open spec fn spec_view_ok(
    s: Seq<Severity>,
    mode: ReportMode,
    quiet: bool,
    view: ValidationView,
) -> bool {
    &&& forall|k: int|
        0 <= k < view.shown@.len() ==> {
            &&& #[trigger] view.shown@[k] < s.len()
            &&& spec_shown(s, mode, quiet, view.shown@[k] as int)
        }
    &&& forall|k1: int, k2: int|
        0 <= k1 < k2 < view.shown@.len() ==> #[trigger] view.shown@[k1] < #[trigger] view.shown@[k2]
    &&& forall|i: usize|
        i < s.len() && #[trigger] spec_shown(s, mode, quiet, i as int) ==> view.shown@.contains(i)
    &&& view.outcome == spec_outcome(s)
    &&& view.announce_pass == (mode == ReportMode::Validate && !quiet && !spec_has_errors(s))
}

/// Decide how a command presents a validation report.
pub fn validation_view(severities: &[Severity], mode: ReportMode, quiet: bool) -> (view:
    ValidationView)
    ensures
        spec_view_ok(severities@, mode, quiet, view),
{
    let errors = has_errors(severities);
    let mut shown: Vec<usize> = Vec::new();
    let mut i: usize = 0;
    while i < severities.len()
        invariant
            i <= severities@.len(),
            errors == spec_has_errors(severities@),
            forall|k: int|
                0 <= k < shown@.len() ==> {
                    &&& #[trigger] shown@[k] < i
                    &&& spec_shown(severities@, mode, quiet, shown@[k] as int)
                },
            forall|k1: int, k2: int|
                0 <= k1 < k2 < shown@.len() ==> #[trigger] shown@[k1] < #[trigger] shown@[k2],
            forall|j: usize|
                j < i && #[trigger] spec_shown(severities@, mode, quiet, j as int) ==> shown@.contains(
                    j,
                ),
        decreases severities.len() - i,
    {
        let is_error = match severities[i] {
            Severity::Error => true,
            Severity::Warning => false,
        };
        let show = match mode {
            ReportMode::Preflight => if errors {
                is_error
            } else {
                !quiet
            },
            ReportMode::Validate => is_error || !quiet,
        };
        let ghost before = shown@;
        if show {
            shown.push(i);
            assert(shown@[shown@.len() - 1] == i);
        }
        assert forall|j: usize|
            j < i + 1 && #[trigger] spec_shown(severities@, mode, quiet, j as int) implies shown@.contains(
            j,
        ) by {
            if j < i {
                let k = choose|k: int| 0 <= k < before.len() && before[k] == j;
                assert(shown@[k] == j);
            } else {
                assert(shown@[shown@.len() - 1] == i);
            }
        }
        i += 1;
    }
    let outcome = if errors {
        let n = error_count(severities);
        GateOutcome::Abort { error_count: n }
    } else {
        GateOutcome::Proceed
    };
    let announce_pass = match mode {
        ReportMode::Validate => !quiet && !errors,
        ReportMode::Preflight => false,
    };
    ValidationView { shown, outcome, announce_pass }
}

/// The outcome depends only on the severities: not on the command and not on
/// `--quiet`. `waft validate` therefore fails exactly when `copy`, `list` and
/// `info` would stop at validation, with the same error count, which is at
/// least one and at most the number of issues.
pub proof fn theorem_gate_is_mode_independent(
    s: Seq<Severity>,
    m1: ReportMode,
    q1: bool,
    v1: ValidationView,
    m2: ReportMode,
    q2: bool,
    v2: ValidationView,
)
    requires
        s.len() <= usize::MAX,
        spec_view_ok(s, m1, q1, v1),
        spec_view_ok(s, m2, q2, v2),
    ensures
        v1.outcome == v2.outcome,
        v1.outcome is Abort <==> spec_has_errors(s),
        v1.outcome matches GateOutcome::Abort { error_count } ==> 1 <= error_count <= s.len()
            && error_count == spec_error_count(s),
{
    theorem_has_errors_iff_count_positive(s);
}

/// Which issues are printed.
pub proof fn theorem_visibility(
    s: Seq<Severity>,
    mode: ReportMode,
    quiet: bool,
    view: ValidationView,
    i: usize,
)
    requires
        spec_view_ok(s, mode, quiet, view),
        i < s.len(),
    ensures
        // `waft validate` prints every error, even with `--quiet`.
        mode == ReportMode::Validate && spec_is_error(s[i as int]) ==> view.shown@.contains(i),
        // A preflight that stops prints every error and nothing else.
        mode == ReportMode::Preflight && view.outcome is Abort ==> (view.shown@.contains(i)
            <==> spec_is_error(s[i as int])),
        // A preflight that proceeds prints warnings only, and none under `--quiet`.
        mode == ReportMode::Preflight && view.outcome is Proceed ==> !spec_is_error(s[i as int]) && (
        view.shown@.contains(i) <==> !quiet),
        // `--quiet` never prints a warning.
        quiet && !spec_is_error(s[i as int]) ==> !view.shown@.contains(i),
        // `validation passed` never accompanies an error.
        view.announce_pass ==> !spec_is_error(s[i as int]) && view.outcome is Proceed,
{
    assert(view.outcome is Abort <==> spec_has_errors(s));
    if view.shown@.contains(i) {
        let k = choose|k: int| 0 <= k < view.shown@.len() && view.shown@[k] == i;
        assert(spec_shown(s, mode, quiet, view.shown@[k] as int));
        assert(spec_shown(s, mode, quiet, i as int));
    }
    if spec_shown(s, mode, quiet, i as int) {
        assert(view.shown@.contains(i));
    }
    if !spec_has_errors(s) {
        assert(!spec_is_error(s[i as int]));
    }
}

} // verus!
