//! Verified kernel: rule-file lint heuristics.
//!
//! Production (`src/validate.rs`) splits a `.gitignore`, `.worktreeinclude`
//! or excludes file into lines with `str::lines`, trims each line with
//! `str::trim`, and hands the trimmed lines to this kernel as byte slices.
//! The kernel decides two things:
//!
//! - which lines are blank or comments and therefore skipped, both by the
//!   lint and by the pattern validator ([`is_blank_or_comment`]);
//! - for every line, whether it gets a "duplicate pattern" warning and which
//!   earlier line it names, and whether it gets a "negation is likely
//!   shadowed by directory exclusion" warning and which directory line it
//!   names ([`lint_lines`]).
//!
//! Production formats one warning per `Some` verdict, line by line, the
//! duplicate warning before the shadow warning, with today's message text.
//!
//! # Theorems
//!
//! - [`lint_lines`] returns one [`LineLint`] per input line, and each verdict
//!   satisfies the declarative specification exactly
//!   (`spec_duplicate_verdict`, `spec_shadow_verdict`):
//!   - `duplicate_of == Some(j)` exactly when line `i` is neither blank nor a
//!     comment and `j` is the *least* index below `i` whose trimmed text is
//!     byte-for-byte equal to line `i`; otherwise `None`.
//!   - `shadowed_by == Some(d)` exactly when line `i` is a negation (its
//!     first byte is `!`) and `d` is the least index below `i` such that line
//!     `d` is a literal directory exclusion (`spec_dir_exclusion`: not blank,
//!     not a comment, not a negation, ends in `/`, contains none of `*`, `?`,
//!     `[`) and line `d` is a byte prefix of line `i` without its `!`.
//! - `theorem_verdicts_are_functions`: the two verdict specifications admit
//!   at most one value per line, so the ensures of [`lint_lines`] pin its
//!   output completely.
//! - `theorem_skipped_lines_are_silent`: blank and comment lines never get
//!   a finding and are never named by one.
//! - `theorem_duplicate_points_at_first_occurrence`: the line a duplicate
//!   warning names is itself never reported as a duplicate, and every later
//!   copy of a pattern names that same first line.
//! - `theorem_shadowing_line_is_plain_directory`: the line a shadow warning
//!   names is not a negation and carries no glob metacharacter, so a
//!   wildcard exclusion like `build/*` never shadows anything.
//! - `theorem_escaped_bang_is_not_negation`: a line starting with `\!` is
//!   never a negation. This is why the old `!trimmed.starts_with("\\!")`
//!   test was dead code and was dropped.
//! - The exec predicates [`is_blank_or_comment`], [`is_negation`] and
//!   [`is_dir_exclusion`] equal their specifications.
//! - `theorem_bytes_agree_with_chars`: on the UTF-8 encoding of any
//!   character sequence (vstd's `encode_utf8`, the bytes of a `&str`), the
//!   byte predicates equal the character predicates the pre-kernel code used:
//!   blank or comment is "empty or `starts_with('#')`", negation is
//!   `starts_with('!')` and its body (`&line[1..]`) is the encoding of the
//!   remaining characters, a literal directory exclusion is "not blank, not a
//!   comment, not a negation, `ends_with('/')`, and no `contains` of `*`,
//!   `?` or `[`", and byte equality is `str` equality. So moving from `&str`
//!   to bytes changed no verdict.
//!
//! # Trust assumptions
//!
//! - Production passes `line.trim().as_bytes()` for every line of
//!   `content.lines()`, in order, and formats one warning per `Some` verdict
//!   in line order (duplicate first): line `i`'s trimmed text with `j + 1`,
//!   or line `i`'s trimmed text with `lines[d].trim_end_matches('/')`, at
//!   line number `i + 1`. `str::lines` and `str::trim` (Unicode
//!   `White_Space`) stay in production. A differential property test in
//!   `src/validate.rs` checks the result against the pre-kernel code.
//! - vstd's model of `&str` bytes as `encode_utf8` of its characters, and
//!   `str::starts_with(&str)` being a byte-prefix test (it is, in `core`),
//!   which is the only string test the shadow check made on bytes already.
//!
//! # Quirks encoded faithfully (current behavior, not fixed here)
//!
//! - Lines are trimmed at both ends before every test, so `  # x` counts as a
//!   comment and ` foo` duplicates `foo`. Git (and `ignore`'s
//!   `GitignoreBuilder::add_line`, and `src/worktreeinclude.rs`) treats `#` as
//!   a comment only in column 0 and keeps leading whitespace, so the lint and
//!   the validator's skip rule disagree with the matcher on indented lines.
//! - The shadow test is a plain byte-prefix test: `build/` shadows
//!   `!build/x` but also `!build/` and `!build//x`, and it does not
//!   understand anchoring (`/build/` and `!build/x` do not match each other).

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// The byte `#`, which starts a comment.
pub const HASH: u8 = 0x23;

/// The byte `!`, which starts a negation.
pub const BANG: u8 = 0x21;

/// The byte `/`, which ends a directory pattern.
pub const SLASH: u8 = 0x2f;

/// The byte `*`, a glob metacharacter.
pub const STAR: u8 = 0x2a;

/// The byte `?`, a glob metacharacter.
pub const QUESTION: u8 = 0x3f;

/// The byte `[`, a glob metacharacter.
pub const LBRACKET: u8 = 0x5b;

/// The byte `\`, the escape character.
pub const BACKSLASH: u8 = 0x5c;

/// A trimmed line that is empty or starts with `#`.
pub open spec fn spec_blank_or_comment(line: Seq<u8>) -> bool {
    line.len() == 0 || line[0] == HASH
}

/// A trimmed line whose first byte is `!`.
pub open spec fn spec_negation(line: Seq<u8>) -> bool {
    line.len() > 0 && line[0] == BANG
}

/// A trimmed line that names a literal directory to exclude: a pattern that
/// is neither blank, a comment nor a negation, ends in `/`, and contains no
/// `*`, `?` or `[`.
pub open spec fn spec_dir_exclusion(line: Seq<u8>) -> bool {
    &&& !spec_blank_or_comment(line)
    &&& !spec_negation(line)
    &&& line.last() == SLASH
    &&& !line.contains(STAR)
    &&& !line.contains(QUESTION)
    &&& !line.contains(LBRACKET)
}

/// `p` is a byte prefix of `s`.
pub open spec fn spec_is_prefix(p: Seq<u8>, s: Seq<u8>) -> bool {
    p.len() <= s.len() && s.subrange(0, p.len() as int) == p
}

/// Line `j` is an earlier line with exactly the same trimmed text as line `i`.
pub open spec fn spec_earlier_copy(lines: Seq<Seq<u8>>, i: int, j: int) -> bool {
    0 <= j < i && lines[j] == lines[i]
}

/// Line `d` is an earlier literal directory exclusion that is a prefix of
/// negation line `i` without its leading `!`.
pub open spec fn spec_earlier_shadow(lines: Seq<Seq<u8>>, i: int, d: int) -> bool {
    &&& 0 <= d < i
    &&& spec_dir_exclusion(lines[d])
    &&& spec_is_prefix(lines[d], lines[i].drop_first())
}

/// The duplicate verdict for line `i`: `Some(j)` names the least earlier
/// copy of a line that is not blank or a comment; `None` means the line is
/// blank, a comment, or has no earlier copy.
pub open spec fn spec_duplicate_verdict(lines: Seq<Seq<u8>>, i: int, v: Option<usize>) -> bool {
    match v {
        Some(j) => {
            &&& !spec_blank_or_comment(lines[i])
            &&& spec_earlier_copy(lines, i, j as int)
            &&& forall|k: int| 0 <= k < j ==> !#[trigger] spec_earlier_copy(lines, i, k)
        },
        None => spec_blank_or_comment(lines[i]) || forall|j: int|
            !#[trigger] spec_earlier_copy(lines, i, j),
    }
}

/// The shadow verdict for line `i`: `Some(d)` names the least earlier literal
/// directory exclusion that prefixes negation `i`; `None` means the line is
/// not a negation or nothing earlier shadows it.
pub open spec fn spec_shadow_verdict(lines: Seq<Seq<u8>>, i: int, v: Option<usize>) -> bool {
    match v {
        Some(d) => {
            &&& spec_negation(lines[i])
            &&& spec_earlier_shadow(lines, i, d as int)
            &&& forall|k: int| 0 <= k < d ==> !#[trigger] spec_earlier_shadow(lines, i, k)
        },
        None => !spec_negation(lines[i]) || forall|d: int|
            !#[trigger] spec_earlier_shadow(lines, i, d),
    }
}

/// The byte contents of each line.
pub open spec fn spec_views(lines: Seq<&[u8]>) -> Seq<Seq<u8>> {
    lines.map_values(|l: &[u8]| l@)
}

/// The lint verdicts for one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineLint {
    /// 0-based index of the first earlier line with the same trimmed text.
    pub duplicate_of: Option<usize>,
    /// 0-based index of the first earlier literal directory exclusion that
    /// prefixes this negation.
    pub shadowed_by: Option<usize>,
}

/// Whether a trimmed line is skipped as blank or a comment.
pub fn is_blank_or_comment(line: &[u8]) -> (r: bool)
    ensures
        r == spec_blank_or_comment(line@),
{
    line.is_empty() || line[0] == HASH
}

/// Whether a trimmed line is a negation.
pub fn is_negation(line: &[u8]) -> (r: bool)
    ensures
        r == spec_negation(line@),
{
    !line.is_empty() && line[0] == BANG
}

/// Whether `line` contains the byte `b`.
fn contains_byte(line: &[u8], b: u8) -> (r: bool)
    ensures
        r == line@.contains(b),
{
    let mut i: usize = 0;
    while i < line.len()
        invariant
            i <= line@.len(),
            forall|k: int| 0 <= k < i ==> line@[k] != b,
        decreases line.len() - i,
    {
        if line[i] == b {
            assert(line@[i as int] == b);
            return true;
        }
        i += 1;
    }
    false
}

/// Whether a trimmed line is a literal directory exclusion.
pub fn is_dir_exclusion(line: &[u8]) -> (r: bool)
    ensures
        r == spec_dir_exclusion(line@),
{
    if is_blank_or_comment(line) || is_negation(line) {
        return false;
    }
    line[line.len() - 1] == SLASH && !contains_byte(line, STAR) && !contains_byte(line, QUESTION)
        && !contains_byte(line, LBRACKET)
}

/// Byte equality of two slices.
fn bytes_eq(a: &[u8], b: &[u8]) -> (r: bool)
    ensures
        r == (a@ == b@),
{
    if a.len() != b.len() {
        return false;
    }
    let mut i: usize = 0;
    while i < a.len()
        invariant
            i <= a@.len(),
            a@.len() == b@.len(),
            forall|k: int| 0 <= k < i ==> a@[k] == b@[k],
        decreases a.len() - i,
    {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    assert(a@ =~= b@);
    true
}

/// Whether `p` is a byte prefix of `s`.
fn is_prefix(p: &[u8], s: &[u8]) -> (r: bool)
    ensures
        r == spec_is_prefix(p@, s@),
{
    if p.len() > s.len() {
        return false;
    }
    let mut i: usize = 0;
    while i < p.len()
        invariant
            i <= p@.len(),
            p@.len() <= s@.len(),
            forall|k: int| 0 <= k < i ==> p@[k] == s@[k],
        decreases p.len() - i,
    {
        if p[i] != s[i] {
            assert(s@.subrange(0, p@.len() as int)[i as int] == s@[i as int]);
            return false;
        }
        i += 1;
    }
    assert(s@.subrange(0, p@.len() as int) =~= p@);
    true
}

/// The least `j < i` whose line equals line `i`, if any.
fn first_earlier_copy(lines: &[&[u8]], i: usize) -> (r: Option<usize>)
    requires
        i < lines@.len(),
    ensures
        match r {
            Some(j) => spec_earlier_copy(spec_views(lines@), i as int, j as int) && forall|k: int|
                0 <= k < j ==> !#[trigger] spec_earlier_copy(spec_views(lines@), i as int, k),
            None => forall|j: int| !#[trigger] spec_earlier_copy(spec_views(lines@), i as int, j),
        },
{
    let ghost v = spec_views(lines@);
    let cur = lines[i];
    let mut j: usize = 0;
    while j < i
        invariant
            j <= i,
            i < lines@.len(),
            v == spec_views(lines@),
            cur@ == v[i as int],
            forall|k: int| 0 <= k < j ==> !#[trigger] spec_earlier_copy(v, i as int, k),
        decreases i - j,
    {
        assert(lines@[j as int]@ == v[j as int]);
        if bytes_eq(lines[j], cur) {
            return Some(j);
        }
        j += 1;
    }
    None
}

/// The least `d < i` that is a literal directory exclusion prefixing `body`.
fn first_shadowing_dir(lines: &[&[u8]], dir_flags: &[bool], i: usize, body: &[u8]) -> (r: Option<
    usize,
>)
    requires
        i < lines@.len(),
        dir_flags@.len() == i,
        forall|k: int| 0 <= k < i ==> dir_flags@[k] == spec_dir_exclusion(#[trigger] lines@[k]@),
        spec_negation(lines@[i as int]@),
        body@ == lines@[i as int]@.drop_first(),
    ensures
        match r {
            Some(d) => spec_earlier_shadow(spec_views(lines@), i as int, d as int) && forall|k: int|
                0 <= k < d ==> !#[trigger] spec_earlier_shadow(spec_views(lines@), i as int, k),
            None => forall|d: int| !#[trigger] spec_earlier_shadow(spec_views(lines@), i as int, d),
        },
{
    let ghost v = spec_views(lines@);
    let mut d: usize = 0;
    while d < i
        invariant
            d <= i,
            i < lines@.len(),
            v == spec_views(lines@),
            dir_flags@.len() == i,
            forall|k: int| 0 <= k < i ==> dir_flags@[k] == spec_dir_exclusion(#[trigger] lines@[k]@),
            body@ == v[i as int].drop_first(),
            forall|k: int| 0 <= k < d ==> !#[trigger] spec_earlier_shadow(v, i as int, k),
        decreases i - d,
    {
        assert(lines@[d as int]@ == v[d as int]);
        if dir_flags[d] && is_prefix(lines[d], body) {
            return Some(d);
        }
        d += 1;
    }
    None
}

/// Lint the trimmed lines of one rule file.
///
/// Returns one verdict per line, in line order. Line indices are 0-based.
pub fn lint_lines(lines: &[&[u8]]) -> (out: Vec<LineLint>)
    ensures
        spec_lint_ok(spec_views(lines@), out@),
{
    let ghost v = spec_views(lines@);
    let mut out: Vec<LineLint> = Vec::new();
    let mut dir_flags: Vec<bool> = Vec::new();
    let mut i: usize = 0;
    while i < lines.len()
        invariant
            i <= lines@.len(),
            v == spec_views(lines@),
            out@.len() == i,
            dir_flags@.len() == i,
            forall|k: int| 0 <= k < i ==> dir_flags@[k] == spec_dir_exclusion(#[trigger] lines@[k]@),
            forall|k: int| 0 <= k < i ==> spec_duplicate_verdict(v, k, #[trigger] out@[k].duplicate_of),
            forall|k: int| 0 <= k < i ==> spec_shadow_verdict(v, k, #[trigger] out@[k].shadowed_by),
        decreases lines.len() - i,
    {
        let line = lines[i];
        assert(line@ == v[i as int]);
        let duplicate_of = if is_blank_or_comment(line) {
            None
        } else {
            first_earlier_copy(lines, i)
        };
        let shadowed_by = if is_negation(line) {
            let body = &line[1..];
            assert(body@ =~= line@.drop_first());
            first_shadowing_dir(lines, dir_flags.as_slice(), i, body)
        } else {
            None
        };
        dir_flags.push(is_dir_exclusion(line));
        out.push(LineLint { duplicate_of, shadowed_by });
        i += 1;
    }
    out
}

/// `out` holds exactly one verdict per line and every verdict meets its
/// specification.
pub open spec fn spec_lint_ok(lines: Seq<Seq<u8>>, out: Seq<LineLint>) -> bool {
    &&& out.len() == lines.len()
    &&& forall|i: int| 0 <= i < lines.len() ==> spec_duplicate_verdict(lines, i, #[trigger] out[i].duplicate_of)
    &&& forall|i: int| 0 <= i < lines.len() ==> spec_shadow_verdict(lines, i, #[trigger] out[i].shadowed_by)
}

/// Each verdict specification admits at most one value per line, so
/// `spec_lint_ok` determines the lint output completely.
pub proof fn theorem_verdicts_are_functions(
    lines: Seq<Seq<u8>>,
    i: int,
    a: Option<usize>,
    b: Option<usize>,
)
    requires
        0 <= i < lines.len(),
    ensures
        spec_duplicate_verdict(lines, i, a) && spec_duplicate_verdict(lines, i, b) ==> a == b,
        spec_shadow_verdict(lines, i, a) && spec_shadow_verdict(lines, i, b) ==> a == b,
{
    if spec_duplicate_verdict(lines, i, a) && spec_duplicate_verdict(lines, i, b) {
        match (a, b) {
            (Some(x), Some(y)) => {
                if x < y {
                    assert(spec_earlier_copy(lines, i, x as int));
                } else if y < x {
                    assert(spec_earlier_copy(lines, i, y as int));
                }
            },
            (Some(x), None) => {
                assert(spec_earlier_copy(lines, i, x as int));
            },
            (None, Some(y)) => {
                assert(spec_earlier_copy(lines, i, y as int));
            },
            (None, None) => {},
        }
    }
    if spec_shadow_verdict(lines, i, a) && spec_shadow_verdict(lines, i, b) {
        match (a, b) {
            (Some(x), Some(y)) => {
                if x < y {
                    assert(spec_earlier_shadow(lines, i, x as int));
                } else if y < x {
                    assert(spec_earlier_shadow(lines, i, y as int));
                }
            },
            (Some(x), None) => {
                assert(spec_earlier_shadow(lines, i, x as int));
            },
            (None, Some(y)) => {
                assert(spec_earlier_shadow(lines, i, y as int));
            },
            (None, None) => {},
        }
    }
}

/// Blank and comment lines never get a finding, and no finding ever names a
/// blank or comment line.
pub proof fn theorem_skipped_lines_are_silent(lines: Seq<Seq<u8>>, out: Seq<LineLint>, i: int)
    requires
        spec_lint_ok(lines, out),
        0 <= i < lines.len(),
    ensures
        spec_blank_or_comment(lines[i]) ==> out[i].duplicate_of is None && out[i].shadowed_by is None,
        out[i].duplicate_of matches Some(j) ==> !spec_blank_or_comment(lines[j as int]),
        out[i].shadowed_by matches Some(d) ==> !spec_blank_or_comment(lines[d as int]),
{
    assert(spec_duplicate_verdict(lines, i, out[i].duplicate_of));
    assert(spec_shadow_verdict(lines, i, out[i].shadowed_by));
}

/// A duplicate warning always names the first occurrence of the pattern:
/// that line is never itself reported as a duplicate, and every later copy
/// of the same text names the same line.
pub proof fn theorem_duplicate_points_at_first_occurrence(
    lines: Seq<Seq<u8>>,
    out: Seq<LineLint>,
    i1: int,
    i2: int,
)
    requires
        spec_lint_ok(lines, out),
        0 <= i1 < lines.len(),
        0 <= i2 < lines.len(),
    ensures
        out[i1].duplicate_of matches Some(j) ==> out[j as int].duplicate_of is None,
        lines[i1] == lines[i2] && out[i1].duplicate_of is Some && out[i2].duplicate_of is Some
            ==> out[i1].duplicate_of == out[i2].duplicate_of,
{
    assert(spec_duplicate_verdict(lines, i1, out[i1].duplicate_of));
    assert(spec_duplicate_verdict(lines, i2, out[i2].duplicate_of));
    if let Some(j) = out[i1].duplicate_of {
        let j = j as int;
        assert(spec_duplicate_verdict(lines, j, out[j].duplicate_of));
        // No line before `j` equals line `j`, so `None` is the verdict for `j`.
        assert forall|k: int| !#[trigger] spec_earlier_copy(lines, j, k) by {
            if spec_earlier_copy(lines, j, k) {
                assert(spec_earlier_copy(lines, i1, k));
            }
        }
        assert(spec_duplicate_verdict(lines, j, None));
        theorem_verdicts_are_functions(lines, j, out[j].duplicate_of, None);
    }
    if lines[i1] == lines[i2] {
        if let (Some(x), Some(y)) = (out[i1].duplicate_of, out[i2].duplicate_of) {
            if x < y {
                assert(spec_earlier_copy(lines, i2, x as int));
            } else if y < x {
                assert(spec_earlier_copy(lines, i1, y as int));
            }
        }
    }
}

/// The line a shadow warning names is a plain directory exclusion: not a
/// negation, ending in `/`, with no `*`, `?` or `[`. In particular a
/// wildcard exclusion such as `build/*` never shadows a negation.
pub proof fn theorem_shadowing_line_is_plain_directory(
    lines: Seq<Seq<u8>>,
    out: Seq<LineLint>,
    i: int,
)
    requires
        spec_lint_ok(lines, out),
        0 <= i < lines.len(),
    ensures
        out[i].shadowed_by matches Some(d) ==> {
            &&& d < i
            &&& spec_negation(lines[i])
            &&& !spec_negation(lines[d as int])
            &&& lines[d as int].last() == SLASH
            &&& !lines[d as int].contains(STAR)
            &&& !lines[d as int].contains(QUESTION)
            &&& !lines[d as int].contains(LBRACKET)
        },
{
    assert(spec_shadow_verdict(lines, i, out[i].shadowed_by));
}

/// A line that starts with `\!` (an escaped `!`) is never a negation. The
/// original code's extra `!trimmed.starts_with("\\!")` test was therefore
/// dead and has been dropped.
pub proof fn theorem_escaped_bang_is_not_negation(line: Seq<u8>)
    requires
        line.len() >= 2,
        line[0] == BACKSLASH,
        line[1] == BANG,
    ensures
        !spec_negation(line),
{
}

// ---------------------------------------------------------------------------
// Bytes versus characters.
//
// Production hands the kernel `line.trim().as_bytes()`, the UTF-8 encoding
// of the trimmed line's characters. The pre-kernel code tested the `&str`
// with `starts_with('#')`, `starts_with('!')`, `ends_with('/')`,
// `contains('*' | '?' | '[')` and `==`. The lemmas below prove that the
// kernel's byte predicates on the encoding are exactly those character
// predicates, so moving to bytes changed nothing.
// ---------------------------------------------------------------------------

/// Setting a high bit leaves a byte at or above `0x80`.
proof fn lemma_high_bit_bytes(y: u8)
    ensures
        (0x80u8 | y) >= 0x80u8,
        (0xC0u8 | y) >= 0x80u8,
        (0xE0u8 | y) >= 0x80u8,
        (0xF0u8 | y) >= 0x80u8,
{
    assert((0x80u8 | y) >= 0x80u8) by (bit_vector);
    assert((0xC0u8 | y) >= 0x80u8) by (bit_vector);
    assert((0xE0u8 | y) >= 0x80u8) by (bit_vector);
    assert((0xF0u8 | y) >= 0x80u8) by (bit_vector);
}

/// The encoding of one character contains an ASCII byte `b`, starts with it,
/// or ends with it, exactly when the character is `b` itself: an ASCII
/// character encodes as its own single byte, and every byte of a longer
/// encoding is at or above `0x80`.
proof fn lemma_scalar_ascii_byte(c: char, b: u8)
    requires
        b < 0x80,
    ensures
        vstd::utf8::encode_scalar(c as u32).len() >= 1,
        vstd::utf8::encode_scalar(c as u32).contains(b) <==> c as u32 == b as u32,
        vstd::utf8::encode_scalar(c as u32)[0] == b <==> c as u32 == b as u32,
        vstd::utf8::encode_scalar(c as u32).last() == b <==> c as u32 == b as u32,
{
    let v = c as u32;
    let e = vstd::utf8::encode_scalar(v);
    if vstd::utf8::has_width_1_encoding(v) {
        assert((v & 0x7F) as u8 as u32 == v) by (bit_vector)
            requires
                v <= 0x7F,
        ;
        assert(e[0] as u32 == v);
        if e.contains(b) {
            let k = choose|k: int| 0 <= k < e.len() && e[k] == b;
            assert(k == 0);
        }
        if v == b as u32 {
            assert(e[0] == b);
        }
    } else {
        lemma_high_bit_bytes(((v >> 6) & 0x1F) as u8);
        lemma_high_bit_bytes(((v >> 12) & 0x0F) as u8);
        lemma_high_bit_bytes(((v >> 18) & 0x7) as u8);
        lemma_high_bit_bytes((v & 0x3F) as u8);
        lemma_high_bit_bytes(((v >> 6) & 0x3F) as u8);
        lemma_high_bit_bytes(((v >> 12) & 0x3F) as u8);
        assert forall|k: int| 0 <= k < e.len() implies e[k] >= 0x80 by {}
        if e.contains(b) {
            let k = choose|k: int| 0 <= k < e.len() && e[k] == b;
            assert(e[k] >= 0x80);
        }
    }
}

/// The encoding of a character sequence contains an ASCII byte exactly when
/// the sequence contains that character.
proof fn lemma_encoding_contains_ascii(cs: Seq<char>, c: char)
    requires
        (c as u32) < 0x80,
    ensures
        vstd::utf8::encode_utf8(cs).contains(c as u8) <==> cs.contains(c),
    decreases cs.len(),
{
    if cs.len() > 0 {
        let rest = cs.drop_first();
        let head = vstd::utf8::encode_scalar(cs[0] as u32);
        let tail = vstd::utf8::encode_utf8(rest);
        let all = vstd::utf8::encode_utf8(cs);
        assert(all == head + tail);
        lemma_encoding_contains_ascii(rest, c);
        lemma_scalar_ascii_byte(cs[0], c as u8);
        assert((c as u8) as u32 == c as u32);
        if all.contains(c as u8) {
            let k = choose|k: int| 0 <= k < all.len() && all[k] == c as u8;
            if k < head.len() {
                assert(head[k] == c as u8);
                assert(head.contains(c as u8));
                assert(cs[0] == c);
            } else {
                assert(tail[k - head.len()] == c as u8);
                assert(tail.contains(c as u8));
                let j = choose|j: int| 0 <= j < rest.len() && rest[j] == c;
                assert(cs[j + 1] == c);
            }
        }
        if cs.contains(c) {
            let j = choose|j: int| 0 <= j < cs.len() && cs[j] == c;
            if j == 0 {
                let k = choose|k: int| 0 <= k < head.len() && head[k] == c as u8;
                assert(all[k] == c as u8);
            } else {
                assert(rest[j - 1] == c);
                assert(rest.contains(c));
                let k = choose|k: int| 0 <= k < tail.len() && tail[k] == c as u8;
                assert(all[head.len() + k] == c as u8);
            }
        }
    }
}

/// The first and last bytes of an encoding come from the first and last
/// characters, and only the empty sequence encodes to no bytes.
proof fn lemma_encoding_ends(cs: Seq<char>)
    ensures
        vstd::utf8::encode_utf8(cs).len() == 0 <==> cs.len() == 0,
        cs.len() > 0 ==> vstd::utf8::encode_utf8(cs)[0] == vstd::utf8::encode_scalar(
            cs[0] as u32,
        )[0],
        cs.len() > 0 ==> vstd::utf8::encode_utf8(cs).last() == vstd::utf8::encode_scalar(
            cs.last() as u32,
        ).last(),
{
    if cs.len() > 0 {
        lemma_scalar_ascii_byte(cs[0], 0);
        lemma_scalar_ascii_byte(cs.last(), 0);
        let init = cs.drop_last();
        assert(cs == init.push(cs.last()));
        vstd::utf8::encode_utf8_push(init, cs.last());
    }
}

/// The kernel's byte predicates on a line's UTF-8 encoding are exactly the
/// pre-kernel `&str` predicates on its characters: blank or comment is
/// "empty or starts with `#`", negation is "starts with `!`" (whose body
/// `&line[1..]` is the encoding of the remaining characters), a literal
/// directory exclusion is "not blank, not a comment, not a negation, ends
/// with `/`, contains no `*`, `?` or `[`", and byte equality is `str`
/// equality.
pub proof fn theorem_bytes_agree_with_chars(cs: Seq<char>, other: Seq<char>)
    ensures
        spec_blank_or_comment(vstd::utf8::encode_utf8(cs)) <==> (cs.len() == 0 || cs[0] == '#'),
        spec_negation(vstd::utf8::encode_utf8(cs)) <==> (cs.len() > 0 && cs[0] == '!'),
        spec_negation(vstd::utf8::encode_utf8(cs)) ==> vstd::utf8::encode_utf8(cs).drop_first()
            == vstd::utf8::encode_utf8(cs.drop_first()),
        spec_dir_exclusion(vstd::utf8::encode_utf8(cs)) <==> {
            &&& cs.len() > 0
            &&& cs[0] != '#'
            &&& cs[0] != '!'
            &&& cs.last() == '/'
            &&& !cs.contains('*')
            &&& !cs.contains('?')
            &&& !cs.contains('[')
        },
        vstd::utf8::encode_utf8(cs) == vstd::utf8::encode_utf8(other) <==> cs == other,
{
    let e = vstd::utf8::encode_utf8(cs);
    lemma_encoding_ends(cs);
    if cs.len() > 0 {
        lemma_scalar_ascii_byte(cs[0], HASH);
        lemma_scalar_ascii_byte(cs[0], BANG);
        lemma_scalar_ascii_byte(cs.last(), SLASH);
        if cs[0] == '!' {
            let head = vstd::utf8::encode_scalar(cs[0] as u32);
            assert(head =~= seq![BANG]);
            assert(e =~= head + vstd::utf8::encode_utf8(cs.drop_first()));
            assert(e.drop_first() =~= vstd::utf8::encode_utf8(cs.drop_first()));
        }
    }
    lemma_encoding_contains_ascii(cs, '*');
    lemma_encoding_contains_ascii(cs, '?');
    lemma_encoding_contains_ascii(cs, '[');
    assert('*' as u8 == STAR && '?' as u8 == QUESTION && '[' as u8 == LBRACKET);
    if e == vstd::utf8::encode_utf8(other) {
        vstd::utf8::encode_utf8_decode_utf8(cs);
        vstd::utf8::encode_utf8_decode_utf8(other);
    }
}

} // verus!
