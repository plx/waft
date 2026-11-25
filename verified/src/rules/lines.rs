//! Rule-line grammars: which `.worktreeinclude` lines become patterns for
//! the per-path engines (and the text recorded for `info`), how a matched
//! glob maps back to its line, and which lines are wt-0.39 literal
//! negations.
//!
//! # What it decides
//!
//! - [`rule_line_pattern_len`]: whether a line of a rule file is handed to
//!   the matcher by the `git`/`claude-2026-04` engines, and the length of
//!   the text recorded for it (`build_file_match_context`). A line is
//!   skipped iff it is blank after trimming trailing whitespace or its first
//!   column is `#`; the recorded text is the line without trailing
//!   whitespace.
//! - [`last_line_with_text`]: the provenance lookup of `glob_line_info`:
//!   the last recorded line whose text equals the matched glob's original
//!   text.
//! - [`wt_negation_target`]: the wt-0.39 grammar
//!   (`collect_wt_literal_negations`): after trimming Unicode whitespace at
//!   both ends, a line is a removal iff it starts with `!` and the rest has
//!   no glob metacharacter (`*`, `?`, `[`, `]`); the target is the rest
//!   minus at most one leading `/`.
//!
//! # Theorems
//!
//! - Each function returns its declarative spec (`trim_end_len`,
//!   `spec_rule_line_pattern_len`, `spec_wt_target`); the lookup returns
//!   the greatest matching index and `None` only when no line matches.
//! - `theorem_wt_targets_are_literal`: a wt target comes only from a
//!   trimmed line starting with `!`, holds no glob metacharacter, and drops
//!   at most one `/`; comments, blank lines and positive patterns never
//!   yield one.
//! - `lemma_quirk_indented_hash_is_a_pattern_but_a_wt_comment`: the two
//!   grammars disagree on an indented `#` (current behavior, by design of
//!   each engine, recorded as an inconsistency).
//! - `lemma_quirk_escaped_trailing_space_loses_provenance`: for a line
//!   ending in an escaped space, the recorded text differs from the glob's
//!   original text (as `ignore` keeps it), so the lookup fails and `info`
//!   reports line 0 (current behavior, not fixed).
//!
//! # Trust assumptions
//!
//! `str::lines` splits the file (production), `str::trim`/`trim_end` strip
//! `char::is_whitespace` (vstd's specification of it lists the Unicode
//! White_Space code points), and `ignore_original_text` is a hand-written
//! model of `ignore` 0.4.33's `GitignoreBuilder::add_line`, used only to
//! state the provenance quirk. Production converts each line to its chars
//! and builds the recorded or target `String` from the returned range.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

// ---------------------------------------------------------------------------
// Trimming
// ---------------------------------------------------------------------------

/// The length of `s` without its trailing whitespace (`str::trim_end`).
pub open spec fn trim_end_len(s: Seq<char>) -> nat
    decreases s.len(),
{
    if s.len() == 0 {
        0
    } else if s.last().is_whitespace() {
        trim_end_len(s.drop_last())
    } else {
        s.len()
    }
}

/// The number of leading whitespace characters of `s` (`str::trim_start`).
pub open spec fn trim_start_len(s: Seq<char>) -> nat
    decreases s.len(),
{
    if s.len() == 0 {
        0
    } else if s[0].is_whitespace() {
        1 + trim_start_len(s.drop_first())
    } else {
        0
    }
}

/// `trim_end_len` is bounded by the length and ends on a non-whitespace
/// character, with only whitespace after it.
pub proof fn lemma_trim_end_len(s: Seq<char>)
    ensures
        trim_end_len(s) <= s.len(),
        trim_end_len(s) > 0 ==> !s[trim_end_len(s) - 1].is_whitespace(),
        forall|i: int| trim_end_len(s) <= i < s.len() ==> #[trigger] s[i].is_whitespace(),
    decreases s.len(),
{
    if s.len() > 0 && s.last().is_whitespace() {
        lemma_trim_end_len(s.drop_last());
        assert forall|i: int| trim_end_len(s) <= i < s.len() implies #[trigger] s[i].is_whitespace() by {
            if i < s.len() - 1 {
                assert(s.drop_last()[i] == s[i]);
            }
        }
    }
}

/// Scanning back from `k` over whitespace preserves `trim_end_len`.
proof fn lemma_trim_end_step(s: Seq<char>, k: int)
    requires
        0 < k <= s.len(),
        s[k - 1].is_whitespace(),
    ensures
        trim_end_len(s.subrange(0, k)) == trim_end_len(s.subrange(0, k - 1)),
{
    assert(s.subrange(0, k).drop_last() =~= s.subrange(0, k - 1));
}

/// Scanning forward from `k` over whitespace adds one to the prefix.
proof fn lemma_trim_start_step(s: Seq<char>, k: int)
    requires
        0 <= k < s.len(),
        s[k].is_whitespace(),
    ensures
        trim_start_len(s.subrange(k, s.len() as int)) == 1 + trim_start_len(s.subrange(k + 1, s.len() as int)),
{
    assert(s.subrange(k, s.len() as int).drop_first() =~= s.subrange(k + 1, s.len() as int));
}

/// Scan back from the end of `line` over trailing whitespace. The char at
/// `floor` is not whitespace, so the scan stops above it, at
/// `trim_end_len(line)`.
fn trailing_end(line: &[char], floor: usize) -> (k: usize)
    requires
        floor < line@.len(),
        !line@[floor as int].is_whitespace(),
    ensures
        k == trim_end_len(line@),
        floor < k <= line@.len(),
{
    let mut k = line.len();
    assert(line@.subrange(0, k as int) =~= line@);
    while k > floor + 1 && line[k - 1].is_whitespace()
        invariant
            floor < k <= line@.len(),
            floor < line@.len(),
            !line@[floor as int].is_whitespace(),
            trim_end_len(line@) == trim_end_len(line@.subrange(0, k as int)),
        decreases k,
    {
        proof {
            lemma_trim_end_step(line@, k as int);
        }
        k -= 1;
    }
    assert(!line@.subrange(0, k as int).last().is_whitespace());
    k
}

// ---------------------------------------------------------------------------
// Per-path engines: which lines become patterns, and line provenance
// ---------------------------------------------------------------------------

/// A rule line is skipped by the per-path engines iff it is blank after
/// trimming trailing whitespace or starts with `#` in its first column
/// (leading whitespace is significant, so `  #x` is a pattern). Otherwise
/// the recorded text is the line without trailing whitespace.
pub open spec fn spec_rule_line_pattern_len(line: Seq<char>) -> Option<nat> {
    if trim_end_len(line) == 0 || line[0] == '#' {
        None
    } else {
        Some(trim_end_len(line))
    }
}

/// Decide whether a rule line becomes a pattern, and the length (in chars)
/// of the text recorded for it.
pub fn rule_line_pattern_len(line: &[char]) -> (r: Option<usize>)
    ensures
        match r {
            Some(k) => spec_rule_line_pattern_len(line@) == Some(k as nat),
            None => spec_rule_line_pattern_len(line@) is None,
        },
{
    let mut k = line.len();
    assert(line@.subrange(0, k as int) =~= line@);
    while k > 0 && line[k - 1].is_whitespace()
        invariant
            k <= line@.len(),
            trim_end_len(line@) == trim_end_len(line@.subrange(0, k as int)),
        decreases k,
    {
        proof {
            lemma_trim_end_step(line@, k as int);
        }
        k -= 1;
    }
    proof {
        if k > 0 {
            assert(!line@.subrange(0, k as int).last().is_whitespace());
        } else {
            assert(line@.subrange(0, 0) =~= Seq::<char>::empty());
        }
    }
    if k == 0 || line[0] == '#' {
        None
    } else {
        Some(k)
    }
}

/// The provenance lookup: the last recorded line whose text equals the
/// matched glob's original text. Duplicate patterns resolve to the last
/// one, which is the one `ignore` reports as matching (highest index wins).
pub fn last_line_with_text(entries: &[(usize, String)], original: &str) -> (r: Option<usize>)
    ensures
        match r {
            Some(i) => {
                &&& i < entries@.len()
                &&& entries@[i as int].1@ == original@
                &&& forall|j: int| i < j < entries@.len() ==> #[trigger] entries@[j].1@ != original@
            },
            None => forall|j: int| 0 <= j < entries@.len() ==> #[trigger] entries@[j].1@ != original@,
        },
{
    let mut i = entries.len();
    while i > 0
        invariant
            i <= entries@.len(),
            forall|j: int| i <= j < entries@.len() ==> #[trigger] entries@[j].1@ != original@,
        decreases i,
    {
        i -= 1;
        if entries[i].1.as_str() == original {
            return Some(i);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// wt-0.39 literal negations
// ---------------------------------------------------------------------------

/// The glob metacharacters that make wt ignore a negation.
pub open spec fn is_glob_meta(c: char) -> bool {
    c == '*' || c == '?' || c == '[' || c == ']'
}

/// The wt-0.39 grammar: trim Unicode whitespace at both ends; the line is a
/// literal negation iff the trimmed text starts with `!` and the rest has
/// no glob metacharacter. The target is the rest without one leading `/`,
/// as a `[start, end)` range of the untrimmed line. (Blank lines, `#`
/// comments, positive patterns and escaped `\!` all fail the `!` test.)
pub open spec fn spec_wt_target(line: Seq<char>) -> Option<(int, int)> {
    let a = trim_start_len(line) as int;
    let b = trim_end_len(line) as int;
    if a >= b {
        None
    } else if line[a] != '!' {
        None
    } else if exists|i: int| a < i < b && #[trigger] is_glob_meta(line[i]) {
        None
    } else {
        Some(
            (
                if a + 1 < b && line[a + 1] == '/' {
                    a + 2
                } else {
                    a + 1
                },
                b,
            ),
        )
    }
}

/// Parse one rule line under the wt-0.39 grammar.
pub fn wt_negation_target(line: &[char]) -> (r: Option<(usize, usize)>)
    ensures
        spec_wt_target(line@) == match r {
            Some((s, e)) => Some((s as int, e as int)),
            None => None::<(int, int)>,
        },
{
    let n = line.len();
    let mut a: usize = 0;
    assert(line@.subrange(0, n as int) =~= line@);
    while a < n && line[a].is_whitespace()
        invariant
            a <= n,
            n == line@.len(),
            trim_start_len(line@) == a + trim_start_len(line@.subrange(a as int, n as int)),
        decreases n - a,
    {
        proof {
            lemma_trim_start_step(line@, a as int);
        }
        a += 1;
    }
    if a == n {
        proof {
            assert(line@.subrange(a as int, n as int) =~= Seq::<char>::empty());
            lemma_trim_end_len(line@);
            if trim_end_len(line@) > 0 {
                assert(line@.subrange(0, n as int)[trim_end_len(line@) - 1] == line@[trim_end_len(line@) - 1]);
                lemma_trim_all_ws(line@, trim_end_len(line@) - 1);
            }
        }
        return None;
    }
    proof {
        assert(line@.subrange(a as int, n as int)[0] == line@[a as int]);
    }
    let b = trailing_end(line, a);
    if line[a] != '!' {
        return None;
    }
    let mut i = a + 1;
    while i < b
        invariant
            a < i <= b,
            b <= line@.len(),
            a == trim_start_len(line@),
            b == trim_end_len(line@),
            line@[a as int] == '!',
            forall|j: int| a < j < i ==> !#[trigger] is_glob_meta(line@[j]),
        decreases b - i,
    {
        let c = line[i];
        if c == '*' || c == '?' || c == '[' || c == ']' {
            assert(is_glob_meta(line@[i as int]));
            return None;
        }
        i += 1;
    }
    let start = if a + 1 < b && line[a + 1] == '/' {
        a + 2
    } else {
        a + 1
    };
    Some((start, b))
}

/// Helper: a non-whitespace character means not every char is skipped by
/// the leading-whitespace scan.
proof fn lemma_trim_all_ws(s: Seq<char>, i: int)
    requires
        0 <= i < s.len(),
        !s[i].is_whitespace(),
        trim_start_len(s) == s.len(),
    ensures
        false,
    decreases s.len(),
{
    if s.len() > 0 && s[0].is_whitespace() {
        if i == 0 {
        } else {
            assert(s.drop_first()[i - 1] == s[i]);
            lemma_trim_all_ws(s.drop_first(), i - 1);
        }
    }
}

/// wt targets come only from literal negations: the trimmed line starts
/// with `!`, the target holds no glob metacharacter, and it drops at most
/// one `/` (so blank lines, `#` comments, positive patterns and escaped
/// `\!` never yield a target).
pub proof fn theorem_wt_targets_are_literal(line: Seq<char>)
    ensures
        spec_wt_target(line) matches Some((s, e)) ==> {
            let a = trim_start_len(line) as int;
            &&& a < e
            &&& e <= line.len()
            &&& line[a] == '!'
            &&& (s == a + 1 || (s == a + 2 && line[a + 1] == '/'))
            &&& forall|j: int| s <= j < e ==> !#[trigger] is_glob_meta(line[j])
        },
        trim_start_len(line) < trim_end_len(line) && line[trim_start_len(line) as int] != '!' ==> spec_wt_target(
            line,
        ) is None,
{
    lemma_trim_end_len(line);
}

/// Current behavior, an inconsistency between the engines' grammars: an
/// indented `#` line is a literal pattern for the per-path engines (only
/// column-0 `#` is a comment) but a comment for wt-0.39 (which trims
/// first).
pub proof fn lemma_quirk_indented_hash_is_a_pattern_but_a_wt_comment()
    ensures
        spec_rule_line_pattern_len(seq![' ', '#', 'x']) == Some(3nat),
        spec_wt_target(seq![' ', '#', 'x']) is None,
{
    let s = seq![' ', '#', 'x'];
    assert(s.last() == 'x');
    assert(s[0].is_whitespace());
    assert(s.drop_first() =~= seq!['#', 'x']);
    assert(!seq!['#', 'x'][0].is_whitespace());
    assert(trim_start_len(s) == 1) by {
        reveal_with_fuel(trim_start_len, 2);
    }
}

/// The original text `ignore` 0.4.33 keeps for a pattern line
/// (`GitignoreBuilder::add_line`): the untrimmed line when it ends with an
/// escaped space `\ `, otherwise the line without trailing whitespace.
/// A trusted model, used only to state the provenance quirk.
pub open spec fn ignore_original_text(line: Seq<char>) -> Seq<char> {
    if line.len() >= 2 && line[line.len() - 2] == '\\' && line[line.len() - 1] == ' ' {
        line
    } else {
        line.subrange(0, trim_end_len(line) as int)
    }
}

/// Current behavior, deliberately not fixed: for the line `foo\ ` the text
/// recorded for provenance (`foo\`) differs from the glob's original text
/// (`foo\ `), so a lookup over that file fails and `info` / `list -v`
/// report line 0 for a match of that line.
pub proof fn lemma_quirk_escaped_trailing_space_loses_provenance()
    ensures
        spec_rule_line_pattern_len(seq!['f', 'o', 'o', '\\', ' ']) == Some(4nat),
        seq!['f', 'o', 'o', '\\', ' '].subrange(0, 4) != ignore_original_text(seq!['f', 'o', 'o', '\\', ' ']),
{
    let s = seq!['f', 'o', 'o', '\\', ' '];
    assert(s.last().is_whitespace());
    assert(s.drop_last() =~= seq!['f', 'o', 'o', '\\']);
    assert(!seq!['f', 'o', 'o', '\\'].last().is_whitespace());
    assert(trim_end_len(s) == 4) by {
        reveal_with_fuel(trim_end_len, 2);
    }
    assert(ignore_original_text(s) == s);
    assert(s.subrange(0, 4).len() != s.len());
}

} // verus!
