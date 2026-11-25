//! Verified kernel: parsers for Git command output.
//!
//! # What it decides
//!
//! How waft reads the machine-readable output of the `git` subprocess
//! backend: `ls-files -s -z` (tracked names and gitlinks,
//! [`parse_ls_files_stage`]), `worktree list --porcelain -z`
//! ([`parse_worktree_list`]), `check-ignore --stdin -z -v -n`
//! ([`parse_check_ignore`]), NUL-separated path lists ([`parse_nul_list`]),
//! and single-line output ([`line_without_ending`]). Parsers return byte
//! ranges ([`Span`]) into the output; production converts the bytes.
//!
//! Every parser is proved equal to a declarative grammar over
//! `nul_fields`, the NUL split of the output, including how malformed
//! input is handled today.
//!
//! # Theorems
//!
//! - `split_nul` returns exactly `nul_fields`, and
//!   `theorem_nul_fields_is_the_split` characterizes that as the unique
//!   kind of split: never empty, NUL-free fields, and joining them with NULs
//!   gives back the output.
//! - `parse_ls_files_stage` returns exactly `stage_records`: one entry per
//!   record that has a TAB, whose path is every byte after the first TAB
//!   (TABs, spaces and non-UTF-8 bytes in paths survive) and which is a
//!   gitlink exactly when its metadata starts with `160000 `.
//! - `parse_worktree_list` returns exactly `worktree_records`; the first
//!   record (and only it) is the main worktree, and
//!   `theorem_one_record_per_worktree_field` shows there is exactly one
//!   record per `worktree ` field, in order.
//! - `parse_check_ignore` returns exactly `ignore_records`: consecutive
//!   groups of four fields, with the line number read exactly as
//!   `str::parse::<usize>` reads it (`usize_from_str`), and
//!   `theorem_negation_never_ignores`: a negated or absent match never
//!   ignores a path.
//! - Every returned span lies inside the output, so production slicing
//!   cannot panic.
//! - Round trips against Git's writers: `theorem_nul_terminated_fields`
//!   (NUL-terminated NUL-free fields split back exactly), and
//!   `theorem_ls_files_round_trip`, `theorem_worktree_list_round_trip` and
//!   `theorem_check_ignore_round_trip`: for every well-formed list of
//!   records, parsing the bytes Git writes for them yields exactly those
//!   records' paths and flags, in order. The fail-open quirks below
//!   therefore never affect well-formed output.
//!
//! # Fail-open behaviour kept for byte-identical results
//!
//! - `ls-files -s` records without a TAB are skipped, which would drop a
//!   tracked name from the tracked set.
//! - A trailing partial `check-ignore` record is dropped.
//! - A stray `bare` field outside any worktree record marks the next record
//!   bare.
//! - The single-line trim removes a CR that belongs to the path.
//!
//! # Trust assumptions
//!
//! Git's documented `-z` output formats; production's conversion of the
//! returned byte ranges (UTF-8 checks and lossy text) is unverified glue.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

// ---------------------------------------------------------------------------
// Spans and NUL-separated fields
// ---------------------------------------------------------------------------
/// A byte range `[start, end)` of a command's output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub struct Span {
    /// First byte of the range.
    pub start: usize,
    /// One past the last byte of the range.
    pub end: usize,
}

/// The bytes a span covers.
pub open spec fn span_bytes(s: Seq<u8>, sp: Span) -> Seq<u8> {
    s.subrange(sp.start as int, sp.end as int)
}

/// The span lies inside an output of length `len`.
pub open spec fn span_ok(len: nat, sp: Span) -> bool {
    sp.start <= sp.end && sp.end <= len
}

/// Every span lies inside an output of length `len`.
pub open spec fn spans_ok(len: nat, spans: Seq<Span>) -> bool {
    forall|k: int| 0 <= k < spans.len() ==> span_ok(len, #[trigger] spans[k])
}

/// The bytes each span covers.
pub open spec fn span_fields(s: Seq<u8>, spans: Seq<Span>) -> Seq<Seq<u8>> {
    spans.map_values(|sp: Span| span_bytes(s, sp))
}

/// The fields of `s` separated by NUL bytes, exactly as
/// `<[u8]>::split(|b| *b == 0)` yields them: `n` NULs give `n + 1` fields, so
/// a trailing NUL gives a trailing empty field and empty input one empty
/// field.
pub open spec fn nul_fields(s: Seq<u8>) -> Seq<Seq<u8>>
    decreases s.len(),
{
    if s.len() == 0 {
        seq![Seq::empty()]
    } else {
        let prev = nul_fields(s.drop_last());
        if s.last() == 0 {
            prev.push(Seq::empty())
        } else {
            prev.update(prev.len() - 1, prev.last().push(s.last()))
        }
    }
}

/// Fields joined back together with a NUL between neighbours.
pub open spec fn join_nul(fields: Seq<Seq<u8>>) -> Seq<u8>
    decreases fields.len(),
{
    if fields.len() <= 1 {
        if fields.len() == 0 {
            Seq::empty()
        } else {
            fields[0]
        }
    } else {
        join_nul(fields.drop_last()) + seq![0u8] + fields.last()
    }
}

/// `nul_fields` is the NUL split: it is never empty, its fields contain no
/// NUL, and joining them with NULs gives back the input.
pub proof fn theorem_nul_fields_is_the_split(s: Seq<u8>)
    ensures
        nul_fields(s).len() >= 1,
        forall|k: int| 0 <= k < nul_fields(s).len() ==> !(#[trigger] nul_fields(s)[k]).contains(0u8),
        join_nul(nul_fields(s)) == s,
    decreases s.len(),
{
    if s.len() > 0 {
        let init = s.drop_last();
        theorem_nul_fields_is_the_split(init);
        let prev = nul_fields(init);
        let fs = nul_fields(s);
        if s.last() == 0 {
            assert(fs.drop_last() == prev);
            assert forall|k: int| 0 <= k < fs.len() implies !(#[trigger] fs[k]).contains(0u8) by {
                if k < prev.len() {
                    assert(fs[k] == prev[k]);
                }
            }
            assert(join_nul(fs) =~= s);
        } else {
            let last = prev.last().push(s.last());
            assert forall|k: int| 0 <= k < fs.len() implies !(#[trigger] fs[k]).contains(0u8) by {
                if k < prev.len() - 1 {
                    assert(fs[k] == prev[k]);
                } else {
                    assert(fs[k] == last);
                    assert forall|j: int| 0 <= j < last.len() implies last[j] != 0u8 by {
                        if j < prev.last().len() {
                            assert(prev[prev.len() - 1][j] == last[j]);
                        }
                    }
                }
            }
            if prev.len() == 1 {
                assert(join_nul(fs) == last);
                assert(prev.last() == join_nul(prev));
                assert(join_nul(fs) =~= s);
            } else {
                assert(fs.drop_last() == prev.drop_last());
                assert(fs.last() == last);
                assert(join_nul(prev) == join_nul(prev.drop_last()) + seq![0u8] + prev.last());
                assert(join_nul(fs) == join_nul(prev.drop_last()) + seq![0u8] + last);
                assert(s =~= init.push(s.last()));
                assert(join_nul(fs) =~= join_nul(prev).push(s.last()));
            }
        }
    }
}

proof fn lemma_nul_fields_nonempty(s: Seq<u8>)
    ensures
        nul_fields(s).len() >= 1,
    decreases s.len(),
{
    if s.len() > 0 {
        lemma_nul_fields_nonempty(s.drop_last());
    }
}

/// Split `s` at every NUL byte.
pub fn split_nul(s: &[u8]) -> (r: Vec<Span>)
    ensures
        spans_ok(s@.len(), r@),
        span_fields(s@, r@) == nul_fields(s@),
{
    let mut out: Vec<Span> = Vec::new();
    let mut start: usize = 0;
    let mut i: usize = 0;
    assert(s@.subrange(0, 0) =~= Seq::<u8>::empty());
    assert(span_fields(s@, out@).push(s@.subrange(0, 0)) =~= seq![Seq::<u8>::empty()]);
    while i < s.len()
        invariant
            start <= i <= s@.len(),
            spans_ok(s@.len(), out@),
            span_fields(s@, out@).push(s@.subrange(start as int, i as int)) == nul_fields(
                s@.subrange(0, i as int),
            ),
        decreases s.len() - i,
    {
        let ghost prefix = s@.subrange(0, i + 1);
        assert(prefix.drop_last() == s@.subrange(0, i as int));
        assert(prefix.last() == s@[i as int]);
        proof {
            lemma_nul_fields_nonempty(s@.subrange(0, i as int));
        }
        if s[i] == 0 {
            let ghost old_out = out@;
            out.push(Span { start, end: i });
            assert(span_fields(s@, out@) =~= span_fields(s@, old_out).push(
                s@.subrange(start as int, i as int),
            ));
            assert(s@.subrange(i + 1, i + 1) =~= Seq::<u8>::empty());
            start = i + 1;
        } else {
            assert(s@.subrange(start as int, i + 1) =~= s@.subrange(start as int, i as int).push(
                s@[i as int],
            ));
            assert(span_fields(s@, out@).push(s@.subrange(start as int, i + 1)) =~= nul_fields(
                prefix,
            ));
        }
        i += 1;
    }
    assert(s@.subrange(0, s@.len() as int) == s@);
    let ghost old_out = out@;
    out.push(Span { start, end: s.len() });
    assert(span_fields(s@, out@) =~= span_fields(s@, old_out).push(
        s@.subrange(start as int, s@.len() as int),
    ));
    out
}

// ---------------------------------------------------------------------------
// Byte helpers
// ---------------------------------------------------------------------------
/// `p` is a prefix of `s`.
pub open spec fn has_prefix(s: Seq<u8>, p: Seq<u8>) -> bool {
    p.len() <= s.len() && s.subrange(0, p.len() as int) == p
}

/// Index of the first `b` in `s`, or `s.len()` when there is none.
pub open spec fn first_index_of(s: Seq<u8>, b: u8) -> int
    decreases s.len(),
{
    if s.len() == 0 {
        0
    } else if s[0] == b {
        0
    } else {
        1 + first_index_of(s.drop_first(), b)
    }
}

/// `first_index_of` is the first occurrence, or the length when absent.
pub proof fn lemma_first_index_of(s: Seq<u8>, b: u8)
    ensures
        0 <= first_index_of(s, b) <= s.len(),
        first_index_of(s, b) < s.len() ==> s[first_index_of(s, b)] == b,
        forall|j: int| 0 <= j < first_index_of(s, b) ==> s[j] != b,
    decreases s.len(),
{
    if s.len() > 0 && s[0] != b {
        lemma_first_index_of(s.drop_first(), b);
        assert forall|j: int| 0 <= j < first_index_of(s, b) implies s[j] != b by {
            if j > 0 {
                assert(s[j] == s.drop_first()[j - 1]);
            }
        }
    }
}

proof fn lemma_first_index_of_unique(s: Seq<u8>, b: u8, i: int)
    requires
        0 <= i <= s.len(),
        i < s.len() ==> s[i] == b,
        forall|j: int| 0 <= j < i ==> s[j] != b,
    ensures
        first_index_of(s, b) == i,
{
    lemma_first_index_of(s, b);
    let f = first_index_of(s, b);
    if f < i {
        assert(s[f] == b);
    }
    if i < f {
        assert(s[i] != b);
    }
}

/// The first position of `b` in `s[sp]`, if any.
fn find_in_span(s: &[u8], sp: Span, b: u8) -> (r: Option<usize>)
    requires
        span_ok(s@.len(), sp),
    ensures
        match r {
            Some(t) => sp.start <= t < sp.end && t - sp.start == first_index_of(span_bytes(s@, sp), b),
            None => first_index_of(span_bytes(s@, sp), b) == span_bytes(s@, sp).len(),
        },
{
    let ghost f = span_bytes(s@, sp);
    let mut i = sp.start;
    while i < sp.end
        invariant
            sp.start <= i <= sp.end,
            span_ok(s@.len(), sp),
            f == span_bytes(s@, sp),
            forall|j: int| 0 <= j < i - sp.start ==> f[j] != b,
        decreases sp.end - i,
    {
        if s[i] == b {
            proof {
                assert(f[i - sp.start] == b);
                lemma_first_index_of_unique(f, b, i - sp.start);
            }
            return Some(i);
        }
        assert(f[i - sp.start] != b);
        i += 1;
    }
    proof {
        lemma_first_index_of_unique(f, b, f.len() as int);
    }
    None
}

/// Whether `s[sp]` starts with `p`.
fn span_has_prefix(s: &[u8], sp: Span, p: &[u8]) -> (r: bool)
    requires
        span_ok(s@.len(), sp),
    ensures
        r == has_prefix(span_bytes(s@, sp), p@),
{
    if sp.end - sp.start < p.len() {
        return false;
    }
    let head = &s[sp.start..sp.start + p.len()];
    let r = head == p;
    assert(head@ =~= span_bytes(s@, sp).subrange(0, p@.len() as int));
    if r {
        assert(head@ =~= p@);
    }
    r
}

/// Whether `s[sp]` is exactly `p`.
fn span_equals(s: &[u8], sp: Span, p: &[u8]) -> (r: bool)
    requires
        span_ok(s@.len(), sp),
    ensures
        r == (span_bytes(s@, sp) == p@),
{
    let field = &s[sp.start..sp.end];
    let r = field == p;
    assert(field@ =~= span_bytes(s@, sp));
    if r {
        assert(field@ =~= p@);
    }
    r
}

// ---------------------------------------------------------------------------
// `git ls-files -s -z --full-name`
// ---------------------------------------------------------------------------
/// Mode prefix of a gitlink (submodule) entry: `"160000 "`.
pub open spec fn gitlink_mode_prefix() -> Seq<u8> {
    seq![b'1', b'6', b'0', b'0', b'0', b'0', b' ']
}

/// One `ls-files -s` entry: the path span and whether it is a gitlink.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub struct StageEntry {
    /// The entry's path (everything after the record's first TAB).
    pub path: Span,
    /// Whether the record's metadata starts with mode `160000`.
    pub is_gitlink: bool,
}

/// The meaning of one record `<mode> <object> <stage>\t<path>`: the path is
/// every byte after the first TAB, and the entry is a gitlink when the
/// metadata before that TAB starts with `"160000 "`. A record without a TAB
/// (including an empty one) yields nothing; that skip is the current
/// fail-open behaviour for malformed records.
pub open spec fn stage_record(f: Seq<u8>) -> Option<(Seq<u8>, bool)> {
    let t = first_index_of(f, 9u8);
    if t == f.len() {
        None
    } else {
        Some((f.subrange(t + 1, f.len() as int), has_prefix(f.subrange(0, t), gitlink_mode_prefix())))
    }
}

/// The entries of a sequence of records, in order.
pub open spec fn stage_records(fs: Seq<Seq<u8>>) -> Seq<(Seq<u8>, bool)>
    decreases fs.len(),
{
    if fs.len() == 0 {
        Seq::empty()
    } else {
        let rest = stage_records(fs.drop_last());
        match stage_record(fs.last()) {
            Some(e) => rest.push(e),
            None => rest,
        }
    }
}

/// What a parsed entry denotes.
pub open spec fn stage_entry_view(s: Seq<u8>, e: StageEntry) -> (Seq<u8>, bool) {
    (span_bytes(s, e.path), e.is_gitlink)
}

/// Parse `git ls-files -s -z --full-name` output.
pub fn parse_ls_files_stage(output: &[u8]) -> (r: Vec<StageEntry>)
    ensures
        forall|k: int| 0 <= k < r@.len() ==> span_ok(output@.len(), #[trigger] r@[k].path),
        r@.map_values(|e: StageEntry| stage_entry_view(output@, e)) == stage_records(
            nul_fields(output@),
        ),
{
    let spans = split_nul(output);
    let ghost fs = nul_fields(output@);
    let mut out: Vec<StageEntry> = Vec::new();
    let mut k: usize = 0;
    while k < spans.len()
        invariant
            k <= spans@.len(),
            spans_ok(output@.len(), spans@),
            span_fields(output@, spans@) == fs,
            fs == nul_fields(output@),
            forall|j: int| 0 <= j < out@.len() ==> span_ok(output@.len(), #[trigger] out@[j].path),
            out@.map_values(|e: StageEntry| stage_entry_view(output@, e)) == stage_records(
                fs.subrange(0, k as int),
            ),
        decreases spans.len() - k,
    {
        let sp = spans[k];
        let ghost f = span_bytes(output@, sp);
        assert(fs[k as int] == f);
        assert(fs.subrange(0, k + 1).drop_last() == fs.subrange(0, k as int));
        assert(fs.subrange(0, k + 1).last() == f);
        let ghost old_out = out@;
        let tab: u8 = 9;
        match find_in_span(output, sp, tab) {
            None => {},
            Some(t) => {
                let prefix: [u8; 7] = [b'1', b'6', b'0', b'0', b'0', b'0', b' '];
                let meta = Span { start: sp.start, end: t };
                let is_gitlink = span_has_prefix(output, meta, prefix.as_slice());
                let entry = StageEntry { path: Span { start: t + 1, end: sp.end }, is_gitlink };
                proof {
                    let ti = t - sp.start;
                    assert(prefix@ =~= gitlink_mode_prefix());
                    assert(span_bytes(output@, meta) =~= f.subrange(0, ti));
                    assert(span_bytes(output@, entry.path) =~= f.subrange(ti + 1, f.len() as int));
                }
                out.push(entry);
                assert(out@.map_values(|e: StageEntry| stage_entry_view(output@, e)) =~= old_out.map_values(
                    |e: StageEntry| stage_entry_view(output@, e),
                ).push(stage_entry_view(output@, entry)));
            },
        }
        k += 1;
    }
    assert(fs.subrange(0, fs.len() as int) == fs);
    out
}

// ---------------------------------------------------------------------------
// NUL-terminated path lists (`git ls-files --others ... -z`)
// ---------------------------------------------------------------------------
/// The non-empty fields, in order.
pub open spec fn nonempty_fields(fs: Seq<Seq<u8>>) -> Seq<Seq<u8>>
    decreases fs.len(),
{
    if fs.len() == 0 {
        Seq::empty()
    } else {
        let rest = nonempty_fields(fs.drop_last());
        if fs.last().len() > 0 {
            rest.push(fs.last())
        } else {
            rest
        }
    }
}

/// Parse a NUL-separated path list, skipping empty fields.
pub fn parse_nul_list(output: &[u8]) -> (r: Vec<Span>)
    ensures
        spans_ok(output@.len(), r@),
        span_fields(output@, r@) == nonempty_fields(nul_fields(output@)),
{
    let spans = split_nul(output);
    let ghost fs = nul_fields(output@);
    let mut out: Vec<Span> = Vec::new();
    let mut k: usize = 0;
    while k < spans.len()
        invariant
            k <= spans@.len(),
            spans_ok(output@.len(), spans@),
            span_fields(output@, spans@) == fs,
            spans_ok(output@.len(), out@),
            span_fields(output@, out@) == nonempty_fields(fs.subrange(0, k as int)),
        decreases spans.len() - k,
    {
        let sp = spans[k];
        assert(fs[k as int] == span_bytes(output@, sp));
        assert(fs.subrange(0, k + 1).drop_last() == fs.subrange(0, k as int));
        let ghost old_out = out@;
        if sp.start < sp.end {
            out.push(sp);
            assert(span_fields(output@, out@) =~= span_fields(output@, old_out).push(
                span_bytes(output@, sp),
            ));
        }
        k += 1;
    }
    assert(fs.subrange(0, fs.len() as int) == fs);
    out
}

// ---------------------------------------------------------------------------
// Single-line output
// ---------------------------------------------------------------------------
/// `s` without one trailing `b`, if it ends with one.
pub open spec fn strip_suffix_byte(s: Seq<u8>, b: u8) -> Seq<u8> {
    if s.len() > 0 && s.last() == b {
        s.drop_last()
    } else {
        s
    }
}

/// Git's single-line output without its line ending: one trailing LF, then
/// one trailing CR. The CR is removed even when it belongs to the path (a
/// known quirk: a path really ending in CR loses that byte).
pub open spec fn trim_line_ending(s: Seq<u8>) -> Seq<u8> {
    strip_suffix_byte(strip_suffix_byte(s, 10u8), 13u8)
}

/// The length of `bytes` without its line ending (see `trim_line_ending`).
pub fn line_without_ending(bytes: &[u8]) -> (end: usize)
    ensures
        end <= bytes@.len(),
        bytes@.len() - end <= 2,
        bytes@.subrange(0, end as int) == trim_line_ending(bytes@),
{
    let mut end = bytes.len();
    if end > 0 && bytes[end - 1] == 10 {
        end -= 1;
    }
    if end > 0 && bytes[end - 1] == 13 {
        end -= 1;
    }
    end
}

// ---------------------------------------------------------------------------
// `git worktree list --porcelain -z`
// ---------------------------------------------------------------------------
/// The field prefix that starts a record: `"worktree "`.
pub open spec fn worktree_field_prefix() -> Seq<u8> {
    seq![b'w', b'o', b'r', b'k', b't', b'r', b'e', b'e', b' ']
}

/// The attribute field marking a bare repository: `"bare"`.
pub open spec fn bare_field() -> Seq<u8> {
    seq![b'b', b'a', b'r', b'e']
}

/// The field starts a worktree record.
pub open spec fn starts_record(f: Seq<u8>) -> bool {
    has_prefix(f, worktree_field_prefix())
}

/// The field ends the open record: the empty field between records (from
/// the double NUL), or the start of the next record.
pub open spec fn ends_record(f: Seq<u8>) -> bool {
    f.len() == 0 || starts_record(f)
}

/// Some field is exactly `"bare"`.
pub open spec fn has_bare(fs: Seq<Seq<u8>>) -> bool {
    exists|j: int| 0 <= j < fs.len() && fs[j] == bare_field()
}

/// How many leading fields belong to a record body (fields up to the next
/// one that ends the record).
pub open spec fn body_len(fs: Seq<Seq<u8>>) -> nat
    decreases fs.len(),
{
    if fs.len() == 0 || ends_record(fs[0]) {
        0
    } else {
        1 + body_len(fs.drop_first())
    }
}

proof fn lemma_body_len_bound(fs: Seq<Seq<u8>>)
    ensures
        body_len(fs) <= fs.len(),
    decreases fs.len(),
{
    if fs.len() > 0 && !ends_record(fs[0]) {
        lemma_body_len_bound(fs.drop_first());
    }
}

/// Mark the first record bare when `b` holds.
pub open spec fn mark_bare(b: bool, rs: Seq<(Seq<u8>, bool)>) -> Seq<(Seq<u8>, bool)> {
    if b && rs.len() > 0 {
        rs.update(0, (rs[0].0, true))
    } else {
        rs
    }
}

/// The worktree records `(path, is_bare)` of a field sequence.
///
/// A record is a `worktree <path>` field followed by its body: every field
/// up to the next empty field or `worktree` field. It is bare when a `bare`
/// field occurs in its body. Fields before a record that do not start one
/// are skipped, except that a stray `bare` among them also marks the next
/// record bare; that quirk only arises on malformed output (Git always
/// prints `bare` inside its record) and is kept for byte-identical
/// behaviour. The first record is the main worktree.
pub open spec fn worktree_records(fs: Seq<Seq<u8>>) -> Seq<(Seq<u8>, bool)>
    decreases fs.len(),
    via worktree_records_decreases
{
    if fs.len() == 0 {
        Seq::empty()
    } else if starts_record(fs[0]) {
        let b = body_len(fs.drop_first());
        seq![(fs[0].subrange(9, fs[0].len() as int), has_bare(fs.subrange(1, 1 + b as int)))]
            + worktree_records(fs.subrange(1 + b as int, fs.len() as int))
    } else {
        mark_bare(fs[0] == bare_field(), worktree_records(fs.drop_first()))
    }
}

#[via_fn]
proof fn worktree_records_decreases(fs: Seq<Seq<u8>>) {
    if fs.len() > 0 {
        lemma_body_len_bound(fs.drop_first());
    }
}

/// One parsed worktree record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub struct WorktreeEntry {
    /// The worktree path (the bytes after `"worktree "`).
    pub path: Span,
    /// Whether this is the first record, which Git always lists for the main
    /// worktree.
    pub is_main: bool,
    /// Whether the record carries a `bare` attribute.
    pub is_bare: bool,
}

/// What a parsed worktree record denotes.
pub open spec fn worktree_entry_view(s: Seq<u8>, e: WorktreeEntry) -> (Seq<u8>, bool) {
    (span_bytes(s, e.path), e.is_bare)
}

/// Skipping fields that do not start a record only accumulates `bare`.
proof fn lemma_skip_to_record(fs: Seq<Seq<u8>>, i: int, j: int)
    requires
        0 <= i <= j <= fs.len(),
        forall|k: int| i <= k < j ==> !starts_record(#[trigger] fs[k]),
    ensures
        worktree_records(fs.subrange(i, fs.len() as int)) == mark_bare(
            has_bare(fs.subrange(i, j)),
            worktree_records(fs.subrange(j, fs.len() as int)),
        ),
    decreases j - i,
{
    if i == j {
        assert(!has_bare(fs.subrange(i, j)));
    } else {
        lemma_skip_to_record(fs, i + 1, j);
        let tail = fs.subrange(i, fs.len() as int);
        assert(tail[0] == fs[i]);
        assert(tail.drop_first() == fs.subrange(i + 1, fs.len() as int));
        let rest = worktree_records(fs.subrange(j, fs.len() as int));
        let b1 = fs[i] == bare_field();
        let b2 = has_bare(fs.subrange(i + 1, j));
        if b1 || b2 {
            if b1 {
                assert(fs.subrange(i, j)[0] == fs[i]);
            } else {
                let inner = fs.subrange(i + 1, j);
                let k = choose|k: int| 0 <= k < inner.len() && #[trigger] inner[k] == bare_field();
                assert(fs.subrange(i, j)[k + 1] == inner[k]);
            }
            assert(has_bare(fs.subrange(i, j)));
        } else {
            assert forall|k: int| 0 <= k < j - i implies fs.subrange(i, j)[k] != bare_field() by {
                if k > 0 {
                    assert(fs.subrange(i, j)[k] == fs.subrange(i + 1, j)[k - 1]);
                }
            }
            assert(!has_bare(fs.subrange(i, j)));
        }
        if rest.len() > 0 {
            assert(mark_bare(b1, mark_bare(b2, rest)) =~= mark_bare(b1 || b2, rest));
        }
    }
}

/// A record body ends exactly at the first field that ends the record.
proof fn lemma_body_end(fs: Seq<Seq<u8>>, a: int, c: int)
    requires
        0 <= a <= c <= fs.len(),
        forall|k: int| a <= k < c ==> !ends_record(#[trigger] fs[k]),
        c < fs.len() ==> ends_record(fs[c]),
    ensures
        body_len(fs.subrange(a, fs.len() as int)) == c - a,
    decreases c - a,
{
    let tail = fs.subrange(a, fs.len() as int);
    if a == c {
        if c < fs.len() {
            assert(tail[0] == fs[c]);
        }
    } else {
        lemma_body_end(fs, a + 1, c);
        assert(tail[0] == fs[a]);
        assert(tail.drop_first() == fs.subrange(a + 1, fs.len() as int));
    }
}

/// Scan from `i` to the next field that starts a record (or the end),
/// noting whether a `bare` field was passed.
fn skip_to_record(s: &[u8], spans: &[Span], i: usize) -> (r: (usize, bool))
    requires
        spans_ok(s@.len(), spans@),
        i <= spans@.len(),
    ensures
        i <= r.0 <= spans@.len(),
        r.0 < spans@.len() ==> starts_record(span_fields(s@, spans@)[r.0 as int]),
        forall|k: int| i <= k < r.0 ==> !starts_record(#[trigger] span_fields(s@, spans@)[k]),
        r.1 == has_bare(span_fields(s@, spans@).subrange(i as int, r.0 as int)),
{
    let ghost fs = span_fields(s@, spans@);
    let prefix: [u8; 9] = [b'w', b'o', b'r', b'k', b't', b'r', b'e', b'e', b' '];
    let bare: [u8; 4] = [b'b', b'a', b'r', b'e'];
    assert(prefix@ =~= worktree_field_prefix());
    assert(bare@ =~= bare_field());
    let mut j = i;
    let mut saw_bare = false;
    while j < spans.len()
        invariant
            i <= j <= spans@.len(),
            spans_ok(s@.len(), spans@),
            fs == span_fields(s@, spans@),
            prefix@ == worktree_field_prefix(),
            bare@ == bare_field(),
            forall|k: int| i <= k < j ==> !starts_record(#[trigger] fs[k]),
            saw_bare == has_bare(fs.subrange(i as int, j as int)),
        decreases spans.len() - j,
    {
        let sp = spans[j];
        assert(fs[j as int] == span_bytes(s@, sp));
        if span_has_prefix(s, sp, prefix.as_slice()) {
            return (j, saw_bare);
        }
        let is_bare = span_equals(s, sp, bare.as_slice());
        proof {
            lemma_has_bare_push(fs, i as int, j as int);
        }
        saw_bare = saw_bare || is_bare;
        j += 1;
    }
    (j, saw_bare)
}

/// Scan a record body from `i` to the next field that ends the record (or
/// the end), noting whether a `bare` field was passed.
fn record_body_end(s: &[u8], spans: &[Span], i: usize) -> (r: (usize, bool))
    requires
        spans_ok(s@.len(), spans@),
        i <= spans@.len(),
    ensures
        i <= r.0 <= spans@.len(),
        r.0 < spans@.len() ==> ends_record(span_fields(s@, spans@)[r.0 as int]),
        forall|k: int| i <= k < r.0 ==> !ends_record(#[trigger] span_fields(s@, spans@)[k]),
        r.1 == has_bare(span_fields(s@, spans@).subrange(i as int, r.0 as int)),
{
    let ghost fs = span_fields(s@, spans@);
    let prefix: [u8; 9] = [b'w', b'o', b'r', b'k', b't', b'r', b'e', b'e', b' '];
    let bare: [u8; 4] = [b'b', b'a', b'r', b'e'];
    assert(prefix@ =~= worktree_field_prefix());
    assert(bare@ =~= bare_field());
    let mut j = i;
    let mut saw_bare = false;
    while j < spans.len()
        invariant
            i <= j <= spans@.len(),
            spans_ok(s@.len(), spans@),
            fs == span_fields(s@, spans@),
            prefix@ == worktree_field_prefix(),
            bare@ == bare_field(),
            forall|k: int| i <= k < j ==> !ends_record(#[trigger] fs[k]),
            saw_bare == has_bare(fs.subrange(i as int, j as int)),
        decreases spans.len() - j,
    {
        let sp = spans[j];
        assert(fs[j as int] == span_bytes(s@, sp));
        if sp.start == sp.end || span_has_prefix(s, sp, prefix.as_slice()) {
            return (j, saw_bare);
        }
        let is_bare = span_equals(s, sp, bare.as_slice());
        proof {
            lemma_has_bare_push(fs, i as int, j as int);
        }
        saw_bare = saw_bare || is_bare;
        j += 1;
    }
    (j, saw_bare)
}

proof fn lemma_has_bare_push(fs: Seq<Seq<u8>>, i: int, j: int)
    requires
        0 <= i <= j < fs.len(),
    ensures
        has_bare(fs.subrange(i, j + 1)) == (has_bare(fs.subrange(i, j)) || fs[j] == bare_field()),
{
    let a = fs.subrange(i, j);
    let b = fs.subrange(i, j + 1);
    assert(b == a.push(fs[j]));
    if has_bare(a) {
        let k = choose|k: int| 0 <= k < a.len() && a[k] == bare_field();
        assert(b[k] == a[k]);
    }
    if fs[j] == bare_field() {
        assert(b[j - i] == fs[j]);
    }
    if has_bare(b) {
        let k = choose|k: int| 0 <= k < b.len() && b[k] == bare_field();
        if k < a.len() {
            assert(a[k] == b[k]);
        }
    }
}

/// Parse `git worktree list --porcelain -z` output (see `worktree_records`).
/// An empty result means Git listed no worktree at all.
pub fn parse_worktree_list(output: &[u8]) -> (r: Vec<WorktreeEntry>)
    ensures
        forall|k: int| 0 <= k < r@.len() ==> span_ok(output@.len(), #[trigger] r@[k].path),
        forall|k: int| 0 <= k < r@.len() ==> (#[trigger] r@[k].is_main) == (k == 0),
        r@.map_values(|e: WorktreeEntry| worktree_entry_view(output@, e)) == worktree_records(
            nul_fields(output@),
        ),
{
    let spans = split_nul(output);
    let ghost fs = nul_fields(output@);
    let n = spans.len();
    let mut out: Vec<WorktreeEntry> = Vec::new();
    let mut i: usize = 0;
    assert(fs.subrange(0, n as int) == fs);
    assert(out@.map_values(|e: WorktreeEntry| worktree_entry_view(output@, e)) =~= Seq::<
        (Seq<u8>, bool),
    >::empty());
    while i < n
        invariant
            i <= n,
            n == spans@.len(),
            spans_ok(output@.len(), spans@),
            span_fields(output@, spans@) == fs,
            fs == nul_fields(output@),
            forall|k: int| 0 <= k < out@.len() ==> span_ok(output@.len(), #[trigger] out@[k].path),
            forall|k: int| 0 <= k < out@.len() ==> (#[trigger] out@[k].is_main) == (k == 0),
            worktree_records(fs) == out@.map_values(
                |e: WorktreeEntry| worktree_entry_view(output@, e),
            ) + worktree_records(fs.subrange(i as int, n as int)),
        decreases n - i,
    {
        let ghost done = out@.map_values(|e: WorktreeEntry| worktree_entry_view(output@, e));
        let (w, junk_bare) = skip_to_record(output, spans.as_slice(), i);
        proof {
            lemma_skip_to_record(fs, i as int, w as int);
        }
        if w == n {
            assert(fs.subrange(n as int, n as int) =~= Seq::<Seq<u8>>::empty());
            assert(worktree_records(fs.subrange(i as int, n as int)) == Seq::<
                (Seq<u8>, bool),
            >::empty());
            i = n;
        } else {
            let (c, body_bare) = record_body_end(output, spans.as_slice(), w + 1);
            let sp = spans[w];
            let entry = WorktreeEntry {
                path: Span { start: sp.start + 9, end: sp.end },
                is_main: out.is_empty(),
                is_bare: junk_bare || body_bare,
            };
            proof {
                let tail = fs.subrange(w as int, n as int);
                let f = fs[w as int];
                assert(f == span_bytes(output@, sp));
                assert(tail[0] == f);
                assert(tail.drop_first() == fs.subrange(w + 1, n as int));
                lemma_body_end(fs, w + 1, c as int);
                assert(tail.subrange(1, 1 + (c - w - 1)) == fs.subrange(w + 1, c as int));
                assert(tail.subrange(1 + (c - w - 1), tail.len() as int) == fs.subrange(
                    c as int,
                    n as int,
                ));
                assert(span_bytes(output@, entry.path) =~= f.subrange(9, f.len() as int));
                let rest = worktree_records(fs.subrange(c as int, n as int));
                let rec = (f.subrange(9, f.len() as int), body_bare);
                assert(worktree_records(tail) == seq![rec] + rest);
                assert(mark_bare(junk_bare, seq![rec] + rest) =~= seq![
                    (f.subrange(9, f.len() as int), junk_bare || body_bare),
                ] + rest);
            }
            let ghost old_out = out@;
            out.push(entry);
            assert(out@.map_values(|e: WorktreeEntry| worktree_entry_view(output@, e)) =~= done.push(
                worktree_entry_view(output@, entry),
            ));
            i = c;
        }
    }
    assert(fs.subrange(n as int, n as int) =~= Seq::<Seq<u8>>::empty());
    assert(out@.map_values(|e: WorktreeEntry| worktree_entry_view(output@, e)) + Seq::<
        (Seq<u8>, bool),
    >::empty() =~= out@.map_values(|e: WorktreeEntry| worktree_entry_view(output@, e)));
    out
}

/// One `worktree` record per `worktree` field, in order: the parser neither
/// drops nor invents records, and attribute fields never become one.
pub proof fn theorem_one_record_per_worktree_field(fs: Seq<Seq<u8>>)
    ensures
        worktree_records(fs).len() == worktree_field_count(fs),
    decreases fs.len(),
{
    if fs.len() > 0 {
        if starts_record(fs[0]) {
            let b = body_len(fs.drop_first());
            lemma_body_len_bound(fs.drop_first());
            theorem_one_record_per_worktree_field(fs.subrange(1 + b as int, fs.len() as int));
            lemma_body_has_no_worktree_field(fs, b);
        } else {
            theorem_one_record_per_worktree_field(fs.drop_first());
            assert(worktree_field_count(fs) == worktree_field_count(fs.drop_first()));
        }
    }
}

/// The number of fields that start a record.
pub open spec fn worktree_field_count(fs: Seq<Seq<u8>>) -> nat
    decreases fs.len(),
{
    if fs.len() == 0 {
        0
    } else {
        (if starts_record(fs[0]) {
            1nat
        } else {
            0nat
        }) + worktree_field_count(fs.drop_first())
    }
}

proof fn lemma_body_has_no_worktree_field(fs: Seq<Seq<u8>>, b: nat)
    requires
        fs.len() > 0,
        starts_record(fs[0]),
        b == body_len(fs.drop_first()),
    ensures
        1 + b <= fs.len(),
        worktree_field_count(fs) == 1 + worktree_field_count(fs.subrange(1 + b as int, fs.len() as int)),
{
    lemma_body_len_bound(fs.drop_first());
    lemma_count_skips_body(fs.drop_first());
    assert(fs.drop_first().subrange(b as int, fs.len() - 1) == fs.subrange(1 + b as int, fs.len() as int));
}

proof fn lemma_count_skips_body(fs: Seq<Seq<u8>>)
    ensures
        body_len(fs) <= fs.len(),
        worktree_field_count(fs) == worktree_field_count(
            fs.subrange(body_len(fs) as int, fs.len() as int),
        ),
    decreases fs.len(),
{
    lemma_body_len_bound(fs);
    if fs.len() == 0 || ends_record(fs[0]) {
        assert(fs.subrange(0, fs.len() as int) == fs);
    } else {
        lemma_count_skips_body(fs.drop_first());
        let b = body_len(fs.drop_first());
        assert(fs.drop_first().subrange(b as int, fs.len() - 1) == fs.subrange(
            1 + b as int,
            fs.len() as int,
        ));
    }
}

// ---------------------------------------------------------------------------
// `git check-ignore --stdin -z -v -n`
// ---------------------------------------------------------------------------
/// An ASCII decimal digit.
pub open spec fn is_digit(b: u8) -> bool {
    48 <= b <= 57
}

/// Every byte is a decimal digit.
pub open spec fn all_digits(s: Seq<u8>) -> bool {
    forall|i: int| 0 <= i < s.len() ==> is_digit(#[trigger] s[i])
}

/// The value of a string of decimal digits.
pub open spec fn decimal_value(s: Seq<u8>) -> nat
    decreases s.len(),
{
    if s.len() == 0 {
        0
    } else {
        decimal_value(s.drop_last()) * 10 + (s.last() - 48) as nat
    }
}

/// `str::parse::<usize>`: an optional leading `+`, then at least one ASCII
/// digit and nothing else, with a value that fits.
pub open spec fn usize_from_str(s: Seq<u8>) -> Option<usize> {
    let digits = if s.len() > 0 && s[0] == b'+' {
        s.drop_first()
    } else {
        s
    };
    if digits.len() > 0 && all_digits(digits) && decimal_value(digits) <= usize::MAX {
        Some(decimal_value(digits) as usize)
    } else {
        None
    }
}

proof fn lemma_decimal_prefix(s: Seq<u8>, k: int)
    requires
        0 <= k <= s.len(),
        all_digits(s),
    ensures
        decimal_value(s.subrange(0, k)) <= decimal_value(s),
    decreases s.len(),
{
    if k < s.len() {
        let init = s.drop_last();
        assert(all_digits(init)) by {
            assert forall|i: int| 0 <= i < init.len() implies is_digit(#[trigger] init[i]) by {
                assert(init[i] == s[i]);
            }
        }
        lemma_decimal_prefix(init, k);
        assert(init.subrange(0, k) == s.subrange(0, k));
    } else {
        assert(s.subrange(0, k) == s);
    }
}

/// Parse the bytes of `s[sp]` exactly as `str::parse::<usize>` would.
#[allow(clippy::manual_is_ascii_check)]
fn parse_usize(s: &[u8], sp: Span) -> (r: Option<usize>)
    requires
        span_ok(s@.len(), sp),
    ensures
        r == usize_from_str(span_bytes(s@, sp)),
{
    let ghost f = span_bytes(s@, sp);
    let mut i = sp.start;
    if i < sp.end && s[i] == b'+' {
        i += 1;
    }
    let ghost d0 = i;
    let ghost digits = s@.subrange(d0 as int, sp.end as int);
    assert(digits == (if f.len() > 0 && f[0] == b'+' {
        f.drop_first()
    } else {
        f
    }));
    if i == sp.end {
        return None;
    }
    let mut acc: usize = 0;
    while i < sp.end
        invariant
            d0 <= i <= sp.end,
            span_ok(s@.len(), sp),
            digits == s@.subrange(d0 as int, sp.end as int),
            f == span_bytes(s@, sp),
            digits == (if f.len() > 0 && f[0] == b'+' {
                f.drop_first()
            } else {
                f
            }),
            digits.len() > 0,
            all_digits(s@.subrange(d0 as int, i as int)),
            acc as nat == decimal_value(s@.subrange(d0 as int, i as int)),
        decreases sp.end - i,
    {
        let b = s[i];
        let ghost before = s@.subrange(d0 as int, i as int);
        let ghost after = s@.subrange(d0 as int, i + 1);
        assert(after.drop_last() == before);
        assert(after.last() == b);
        assert(digits.subrange(0, i + 1 - d0) == after);
        if !(b'0'..=b'9').contains(&b) {
            assert(!is_digit(digits[i - d0]));
            return None;
        }
        assert(all_digits(after)) by {
            assert forall|j: int| 0 <= j < after.len() implies is_digit(#[trigger] after[j]) by {
                if j < before.len() {
                    assert(after[j] == before[j]);
                }
            }
        }
        let digit = (b - b'0') as usize;
        match acc.checked_mul(10) {
            None => {
                proof {
                    if all_digits(digits) {
                        lemma_decimal_prefix(digits, i + 1 - d0);
                    }
                }
                return None;
            },
            Some(scaled) => match scaled.checked_add(digit) {
                None => {
                    proof {
                        if all_digits(digits) {
                            lemma_decimal_prefix(digits, i + 1 - d0);
                        }
                    }
                    return None;
                },
                Some(next) => {
                    acc = next;
                },
            },
        }
        i += 1;
    }
    assert(s@.subrange(d0 as int, i as int) == digits);
    Some(acc)
}

/// One `check-ignore -v -n` record: `source NUL linenum NUL pattern NUL path NUL`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub struct IgnoreEntry {
    /// The file holding the matching rule (empty when nothing matched).
    pub source: Span,
    /// The rule's line number text (empty when nothing matched).
    pub line_number: Span,
    /// The matching rule (empty when nothing matched).
    pub pattern: Span,
    /// The checked path.
    pub path: Span,
    /// Some rule matched: source and line number are not both empty.
    pub has_match: bool,
    /// A rule matched and it is not a negation (`!pattern`).
    pub ignored: bool,
    /// The line number as `str::parse::<usize>` reads it, or 0.
    pub line: usize,
}

/// What one record denotes: its four fields, whether a rule matched,
/// whether the path is ignored, and the line number (0 when unparsable).
pub open spec fn ignore_meaning(
    source: Seq<u8>,
    line_number: Seq<u8>,
    pattern: Seq<u8>,
    path: Seq<u8>,
) -> (Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>, bool, bool, usize) {
    let has_match = !(source.len() == 0 && line_number.len() == 0);
    let negated = pattern.len() > 0 && pattern[0] == b'!';
    let line = match usize_from_str(line_number) {
        Some(v) => v,
        None => 0,
    };
    (source, line_number, pattern, path, has_match, has_match && !negated, line)
}

/// Records are consecutive groups of four fields; leftover fields (the
/// trailing empty field Git's final NUL produces, or a truncated partial
/// record) yield nothing. Dropping a partial record is the current
/// fail-open behaviour.
pub open spec fn ignore_records(fs: Seq<Seq<u8>>) -> Seq<
    (Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>, bool, bool, usize),
>
    decreases fs.len(),
{
    if fs.len() < 4 {
        Seq::empty()
    } else {
        seq![ignore_meaning(fs[0], fs[1], fs[2], fs[3])] + ignore_records(
            fs.subrange(4, fs.len() as int),
        )
    }
}

/// What a parsed record denotes.
pub open spec fn ignore_entry_view(s: Seq<u8>, e: IgnoreEntry) -> (
    Seq<u8>,
    Seq<u8>,
    Seq<u8>,
    Seq<u8>,
    bool,
    bool,
    usize,
) {
    (
        span_bytes(s, e.source),
        span_bytes(s, e.line_number),
        span_bytes(s, e.pattern),
        span_bytes(s, e.path),
        e.has_match,
        e.ignored,
        e.line,
    )
}

/// Every span of a parsed record lies inside the output.
pub open spec fn ignore_entry_ok(len: nat, e: IgnoreEntry) -> bool {
    span_ok(len, e.source) && span_ok(len, e.line_number) && span_ok(len, e.pattern) && span_ok(
        len,
        e.path,
    )
}

/// Parse `git check-ignore --stdin -z -v -n` output (see `ignore_records`).
pub fn parse_check_ignore(output: &[u8]) -> (r: Vec<IgnoreEntry>)
    ensures
        forall|k: int| 0 <= k < r@.len() ==> ignore_entry_ok(output@.len(), #[trigger] r@[k]),
        r@.map_values(|e: IgnoreEntry| ignore_entry_view(output@, e)) == ignore_records(
            nul_fields(output@),
        ),
{
    let spans = split_nul(output);
    let ghost fs = nul_fields(output@);
    let n = spans.len();
    let mut out: Vec<IgnoreEntry> = Vec::new();
    let mut i: usize = 0;
    assert(fs.subrange(0, n as int) == fs);
    assert(out@.map_values(|e: IgnoreEntry| ignore_entry_view(output@, e)) =~= Seq::<
        (Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>, bool, bool, usize),
    >::empty());
    while n - i >= 4
        invariant
            i <= n,
            n == spans@.len(),
            spans_ok(output@.len(), spans@),
            span_fields(output@, spans@) == fs,
            fs == nul_fields(output@),
            forall|k: int| 0 <= k < out@.len() ==> ignore_entry_ok(output@.len(), #[trigger] out@[k]),
            ignore_records(fs) == out@.map_values(|e: IgnoreEntry| ignore_entry_view(output@, e))
                + ignore_records(fs.subrange(i as int, n as int)),
        decreases n - i,
    {
        let ghost done = out@.map_values(|e: IgnoreEntry| ignore_entry_view(output@, e));
        let source = spans[i];
        let line_number = spans[i + 1];
        let pattern = spans[i + 2];
        let path = spans[i + 3];
        let has_match = !(source.start == source.end && line_number.start == line_number.end);
        let negated = pattern.start < pattern.end && output[pattern.start] == b'!';
        let line = parse_usize(output, line_number).unwrap_or(0);
        let entry = IgnoreEntry {
            source,
            line_number,
            pattern,
            path,
            has_match,
            ignored: has_match && !negated,
            line,
        };
        proof {
            let tail = fs.subrange(i as int, n as int);
            assert(tail[0] == span_bytes(output@, source));
            assert(tail[1] == span_bytes(output@, line_number));
            assert(tail[2] == span_bytes(output@, pattern));
            assert(tail[3] == span_bytes(output@, path));
            assert(tail.subrange(4, tail.len() as int) == fs.subrange(i + 4, n as int));
            if pattern.start < pattern.end {
                assert(span_bytes(output@, pattern)[0] == output@[pattern.start as int]);
            }
            assert(ignore_entry_view(output@, entry) == ignore_meaning(
                tail[0],
                tail[1],
                tail[2],
                tail[3],
            ));
        }
        out.push(entry);
        assert(out@.map_values(|e: IgnoreEntry| ignore_entry_view(output@, e)) =~= done.push(
            ignore_entry_view(output@, entry),
        ));
        i += 4;
    }
    assert(ignore_records(fs.subrange(i as int, n as int)) == Seq::<
        (Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>, bool, bool, usize),
    >::empty());
    assert(out@.map_values(|e: IgnoreEntry| ignore_entry_view(output@, e)) + Seq::<
        (Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>, bool, bool, usize),
    >::empty() =~= out@.map_values(|e: IgnoreEntry| ignore_entry_view(output@, e)));
    out
}

/// A negated rule never makes a path ignored, and a record with no match is
/// never ignored.
pub proof fn theorem_negation_never_ignores(
    source: Seq<u8>,
    line_number: Seq<u8>,
    pattern: Seq<u8>,
    path: Seq<u8>,
)
    ensures
        pattern.len() > 0 && pattern[0] == b'!' ==> !ignore_meaning(
            source,
            line_number,
            pattern,
            path,
        ).5,
        source.len() == 0 && line_number.len() == 0 ==> !ignore_meaning(
            source,
            line_number,
            pattern,
            path,
        ).4 && !ignore_meaning(source, line_number, pattern, path).5,
{
}

// ---------------------------------------------------------------------------
// Round trips: the grammars read back exactly what Git writes
// ---------------------------------------------------------------------------
/// The byte string contains no NUL.
pub open spec fn nul_free(s: Seq<u8>) -> bool {
    !s.contains(0u8)
}

/// Every field followed by a NUL: the shape of Git's `-z` listings.
pub open spec fn nul_terminated(fs: Seq<Seq<u8>>) -> Seq<u8>
    decreases fs.len(),
{
    if fs.len() == 0 {
        Seq::empty()
    } else {
        nul_terminated(fs.drop_last()) + fs.last() + seq![0u8]
    }
}

proof fn lemma_nul_fields_append_nul_free(s: Seq<u8>, r: Seq<u8>)
    requires
        nul_free(r),
    ensures
        nul_fields(s + r) == nul_fields(s).update(
            nul_fields(s).len() - 1,
            nul_fields(s).last() + r,
        ),
    decreases r.len(),
{
    lemma_nul_fields_nonempty(s);
    let f = nul_fields(s);
    if r.len() == 0 {
        assert(s + r =~= s);
        assert(f.last() + r =~= f.last());
        assert(f.update(f.len() - 1, f.last()) =~= f);
    } else {
        let r0 = r.drop_last();
        assert(nul_free(r0)) by {
            if r0.contains(0u8) {
                let j = choose|j: int| 0 <= j < r0.len() && r0[j] == 0u8;
                assert(r[j] == 0u8);
            }
        }
        assert(r.last() != 0u8) by {
            assert(r[r.len() - 1] == r.last());
        }
        lemma_nul_fields_append_nul_free(s, r0);
        let sr = s + r;
        assert(sr.drop_last() =~= s + r0);
        assert(sr.last() == r.last());
        lemma_nul_fields_nonempty(s + r0);
        let prev = nul_fields(s + r0);
        assert(prev.last() == f.last() + r0);
        assert(prev.last().push(r.last()) =~= f.last() + r);
        assert(nul_fields(sr) =~= f.update(f.len() - 1, f.last() + r));
    }
}

/// NUL-terminated NUL-free fields split back into exactly those fields,
/// plus the empty field after the final NUL.
pub proof fn theorem_nul_terminated_fields(fs: Seq<Seq<u8>>)
    requires
        forall|i: int| 0 <= i < fs.len() ==> nul_free(#[trigger] fs[i]),
    ensures
        nul_fields(nul_terminated(fs)) == fs.push(Seq::<u8>::empty()),
    decreases fs.len(),
{
    if fs.len() == 0 {
        assert(nul_fields(Seq::<u8>::empty()) =~= fs.push(Seq::<u8>::empty()));
    } else {
        let init = fs.drop_last();
        assert forall|i: int| 0 <= i < init.len() implies nul_free(#[trigger] init[i]) by {
            assert(init[i] == fs[i]);
        }
        theorem_nul_terminated_fields(init);
        let t0 = nul_terminated(init);
        assert(nul_free(fs.last())) by {
            assert(fs[fs.len() - 1] == fs.last());
        }
        lemma_nul_fields_append_nul_free(t0, fs.last());
        let body = t0 + fs.last();
        assert(nul_fields(body) =~= fs);
        let t = nul_terminated(fs);
        assert(t =~= body.push(0u8));
        assert(t.drop_last() =~= body);
    }
}

/// An empty trailing field adds no `ls-files -s` entry.
proof fn lemma_stage_records_trailing_empty(fs: Seq<Seq<u8>>)
    ensures
        stage_records(fs.push(Seq::<u8>::empty())) == stage_records(fs),
{
    let g = fs.push(Seq::<u8>::empty());
    assert(g.drop_last() =~= fs);
    assert(first_index_of(Seq::<u8>::empty(), 9u8) == 0);
}

/// One `ls-files -s` line: `<mode> SP <object> SP <stage> TAB <path>`.
pub open spec fn stage_line(mode: Seq<u8>, oid: Seq<u8>, stage: Seq<u8>, path: Seq<u8>) -> Seq<
    u8,
> {
    mode + seq![b' '] + oid + seq![b' '] + stage + seq![9u8] + path
}

/// The byte string contains neither `a` nor `b`.
pub open spec fn avoids(s: Seq<u8>, a: u8, b: u8) -> bool {
    !s.contains(a) && !s.contains(b)
}

/// A well-formed `ls-files -s` record: metadata free of TAB and NUL, a mode
/// free of spaces (Git prints six octal digits), and a NUL-free path that
/// may hold any other byte, TABs and newlines included.
pub open spec fn stage_record_ok(r: (Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>)) -> bool {
    &&& avoids(r.0, 9u8, 0u8)
    &&& !r.0.contains(b' ')
    &&& avoids(r.1, 9u8, 0u8)
    &&& avoids(r.2, 9u8, 0u8)
    &&& nul_free(r.3)
}

/// The lines Git writes for `recs`, in order.
pub open spec fn stage_lines(recs: Seq<(Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>)>) -> Seq<Seq<u8>> {
    recs.map_values(|r: (Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>)| stage_line(r.0, r.1, r.2, r.3))
}

/// What waft should read from `recs`: each path, and whether its mode is
/// the gitlink mode `160000`.
pub open spec fn stage_expected(recs: Seq<(Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>)>) -> Seq<
    (Seq<u8>, bool),
> {
    recs.map_values(
        |r: (Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>)|
            (r.3, r.0 == seq![b'1', b'6', b'0', b'0', b'0', b'0']),
    )
}

proof fn lemma_stage_line_record(mode: Seq<u8>, oid: Seq<u8>, stage: Seq<u8>, path: Seq<u8>)
    requires
        stage_record_ok((mode, oid, stage, path)),
    ensures
        nul_free(stage_line(mode, oid, stage, path)),
        stage_record(stage_line(mode, oid, stage, path)) == Some(
            (path, mode == seq![b'1', b'6', b'0', b'0', b'0', b'0']),
        ),
{
    let meta = mode + seq![b' '] + oid + seq![b' '] + stage;
    let line = stage_line(mode, oid, stage, path);
    assert(line =~= meta + seq![9u8] + path);
    let t = meta.len() as int;
    assert(line[t] == 9u8);
    assert forall|j: int| 0 <= j < t implies line[j] != 9u8 by {
        assert(line[j] == meta[j]);
        if j < mode.len() {
            assert(meta[j] == mode[j]);
            assert(!mode.contains(9u8));
        } else if j == mode.len() {
        } else if j < mode.len() + 1 + oid.len() {
            assert(meta[j] == oid[j - mode.len() - 1]);
            assert(!oid.contains(9u8));
        } else if j == mode.len() + 1 + oid.len() {
        } else {
            assert(meta[j] == stage[j - mode.len() - 2 - oid.len()]);
            assert(!stage.contains(9u8));
        }
    }
    lemma_first_index_of_unique(line, 9u8, t);
    assert(line.subrange(t + 1, line.len() as int) =~= path);
    assert(line.subrange(0, t) =~= meta);
    assert(nul_free(line)) by {
        if line.contains(0u8) {
            let j = choose|j: int| 0 <= j < line.len() && line[j] == 0u8;
            if j < mode.len() {
                assert(line[j] == mode[j]);
            } else if j == mode.len() {
            } else if j < mode.len() + 1 + oid.len() {
                assert(line[j] == oid[j - mode.len() - 1]);
            } else if j == mode.len() + 1 + oid.len() {
            } else if j < t {
                assert(line[j] == stage[j - mode.len() - 2 - oid.len()]);
            } else if j == t {
            } else {
                assert(line[j] == path[j - t - 1]);
            }
        }
    }
    let gl = seq![b'1', b'6', b'0', b'0', b'0', b'0'];
    let prefix = gitlink_mode_prefix();
    if mode == gl {
        assert(meta.subrange(0, 7) =~= prefix);
    }
    if has_prefix(meta, prefix) {
        // A space-free mode followed by a space must be exactly "160000".
        if mode.len() < 6 {
            assert(meta[mode.len() as int] == b' ');
            assert(meta.subrange(0, 7)[mode.len() as int] == prefix[mode.len() as int]);
        } else if mode.len() > 6 {
            assert(meta[6] == mode[6]);
            assert(meta.subrange(0, 7)[6] == prefix[6]);
            assert(!mode.contains(b' '));
        } else {
            assert forall|j: int| 0 <= j < 6 implies mode[j] == gl[j] by {
                assert(meta[j] == mode[j]);
                assert(meta.subrange(0, 7)[j] == prefix[j]);
            }
            assert(mode =~= gl);
        }
    }
}

/// `ls-files -s -z` round trip: for any well-formed records (see
/// `stage_record_ok`), parsing what Git writes yields exactly each record's
/// path and whether its mode is `160000`. Paths with TABs, newlines, spaces
/// or non-UTF-8 bytes survive byte for byte.
pub proof fn theorem_ls_files_round_trip(recs: Seq<(Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>)>)
    requires
        forall|i: int| 0 <= i < recs.len() ==> stage_record_ok(#[trigger] recs[i]),
    ensures
        stage_records(nul_fields(nul_terminated(stage_lines(recs)))) == stage_expected(recs),
{
    let lines = stage_lines(recs);
    assert forall|i: int| 0 <= i < lines.len() implies nul_free(#[trigger] lines[i]) by {
        lemma_stage_line_record(recs[i].0, recs[i].1, recs[i].2, recs[i].3);
    }
    theorem_nul_terminated_fields(lines);
    lemma_stage_records_trailing_empty(lines);
    lemma_stage_records_of_lines(recs);
}

proof fn lemma_stage_records_of_lines(recs: Seq<(Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>)>)
    requires
        forall|i: int| 0 <= i < recs.len() ==> stage_record_ok(#[trigger] recs[i]),
    ensures
        stage_records(stage_lines(recs)) == stage_expected(recs),
    decreases recs.len(),
{
    if recs.len() > 0 {
        let init = recs.drop_last();
        assert forall|i: int| 0 <= i < init.len() implies stage_record_ok(#[trigger] init[i]) by {
            assert(init[i] == recs[i]);
        }
        lemma_stage_records_of_lines(init);
        let r = recs.last();
        assert(stage_record_ok(r)) by {
            assert(recs[recs.len() - 1] == r);
        }
        lemma_stage_line_record(r.0, r.1, r.2, r.3);
        assert(stage_lines(recs).drop_last() =~= stage_lines(init));
        assert(stage_lines(recs).last() == stage_line(r.0, r.1, r.2, r.3));
        assert(stage_expected(recs) =~= stage_expected(init).push(
            (r.3, r.0 == seq![b'1', b'6', b'0', b'0', b'0', b'0']),
        ));
    } else {
        assert(stage_expected(recs) =~= Seq::<(Seq<u8>, bool)>::empty());
    }
}

/// The fields of `check-ignore -z` records, four per record, in order.
pub open spec fn ignore_fields(recs: Seq<(Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>)>) -> Seq<Seq<u8>>
    decreases recs.len(),
{
    if recs.len() == 0 {
        Seq::empty()
    } else {
        seq![recs[0].0, recs[0].1, recs[0].2, recs[0].3] + ignore_fields(recs.drop_first())
    }
}

/// `check-ignore --stdin -z -v -n` round trip: for any records whose four
/// fields are NUL-free, parsing what Git writes yields exactly each
/// record's meaning, in order (a record with empty source and line number,
/// which `-n` prints for an unmatched path, reads as no match).
pub proof fn theorem_check_ignore_round_trip(recs: Seq<(Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>)>)
    requires
        forall|i: int|
            0 <= i < recs.len() ==> nul_free((#[trigger] recs[i]).0) && nul_free(recs[i].1)
                && nul_free(recs[i].2) && nul_free(recs[i].3),
    ensures
        ignore_records(nul_fields(nul_terminated(ignore_fields(recs)))) == recs.map_values(
            |r: (Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>)| ignore_meaning(r.0, r.1, r.2, r.3),
        ),
{
    let fs = ignore_fields(recs);
    lemma_ignore_fields_shape(recs);
    theorem_nul_terminated_fields(fs);
    lemma_ignore_records_of_fields(recs);
}

proof fn lemma_ignore_fields_shape(recs: Seq<(Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>)>)
    requires
        forall|i: int|
            0 <= i < recs.len() ==> nul_free((#[trigger] recs[i]).0) && nul_free(recs[i].1)
                && nul_free(recs[i].2) && nul_free(recs[i].3),
    ensures
        ignore_fields(recs).len() == 4 * recs.len(),
        forall|k: int|
            0 <= k < ignore_fields(recs).len() ==> nul_free(#[trigger] ignore_fields(recs)[k]),
    decreases recs.len(),
{
    if recs.len() > 0 {
        let rest = recs.drop_first();
        assert forall|i: int|
            0 <= i < rest.len() implies nul_free((#[trigger] rest[i]).0) && nul_free(rest[i].1)
                && nul_free(rest[i].2) && nul_free(rest[i].3) by {
            assert(rest[i] == recs[i + 1]);
        }
        lemma_ignore_fields_shape(rest);
        let head = seq![recs[0].0, recs[0].1, recs[0].2, recs[0].3];
        let fs = ignore_fields(recs);
        assert(fs == head + ignore_fields(rest));
        assert forall|k: int| 0 <= k < fs.len() implies nul_free(#[trigger] fs[k]) by {
            if k >= 4 {
                assert(fs[k] == ignore_fields(rest)[k - 4]);
            }
        }
    }
}

proof fn lemma_ignore_records_of_fields(recs: Seq<(Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>)>)
    ensures
        ignore_records(ignore_fields(recs).push(Seq::<u8>::empty())) == recs.map_values(
            |r: (Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>)| ignore_meaning(r.0, r.1, r.2, r.3),
        ),
    decreases recs.len(),
{
    let expected = recs.map_values(
        |r: (Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>)| ignore_meaning(r.0, r.1, r.2, r.3),
    );
    if recs.len() == 0 {
        assert(ignore_fields(recs).push(Seq::<u8>::empty()).len() == 1);
        assert(expected =~= Seq::<(Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>, bool, bool, usize)>::empty());
    } else {
        let rest = recs.drop_first();
        lemma_ignore_records_of_fields(rest);
        let head = seq![recs[0].0, recs[0].1, recs[0].2, recs[0].3];
        let fs = ignore_fields(recs).push(Seq::<u8>::empty());
        assert(fs =~= head + ignore_fields(rest).push(Seq::<u8>::empty()));
        assert(fs.subrange(4, fs.len() as int) =~= ignore_fields(rest).push(Seq::<u8>::empty()));
        assert(fs[0] == recs[0].0 && fs[1] == recs[0].1 && fs[2] == recs[0].2 && fs[3] == recs[0].3);
        let rest_expected = rest.map_values(
            |r: (Seq<u8>, Seq<u8>, Seq<u8>, Seq<u8>)| ignore_meaning(r.0, r.1, r.2, r.3),
        );
        assert(expected =~= seq![ignore_meaning(recs[0].0, recs[0].1, recs[0].2, recs[0].3)]
            + rest_expected);
    }
}

/// The fields Git writes for worktree records `(path, attributes)`: a
/// `worktree <path>` field, the attribute fields, then the empty field that
/// becomes the record-separating double NUL.
pub open spec fn worktree_fields(recs: Seq<(Seq<u8>, Seq<Seq<u8>>)>) -> Seq<Seq<u8>>
    decreases recs.len(),
{
    if recs.len() == 0 {
        Seq::empty()
    } else {
        seq![worktree_field_prefix() + recs[0].0] + recs[0].1 + seq![Seq::<u8>::empty()]
            + worktree_fields(recs.drop_first())
    }
}

/// A well-formed worktree record: a NUL-free path, and attribute fields
/// (`HEAD <oid>`, `branch <ref>`, `detached`, `bare`, `locked [reason]`,
/// `prunable [reason]`, or any future one) that are non-empty, NUL-free, and
/// do not start with `worktree `.
pub open spec fn worktree_record_ok(r: (Seq<u8>, Seq<Seq<u8>>)) -> bool {
    &&& nul_free(r.0)
    &&& forall|j: int|
        0 <= j < r.1.len() ==> {
            let f = #[trigger] r.1[j];
            &&& f.len() > 0
            &&& !starts_record(f)
            &&& nul_free(f)
        }
}

/// What waft should read from `recs`: each path, and whether its record
/// carries a `bare` attribute.
pub open spec fn worktree_expected(recs: Seq<(Seq<u8>, Seq<Seq<u8>>)>) -> Seq<(Seq<u8>, bool)> {
    recs.map_values(|r: (Seq<u8>, Seq<Seq<u8>>)| (r.0, has_bare(r.1)))
}

proof fn lemma_worktree_fields_nul_free(recs: Seq<(Seq<u8>, Seq<Seq<u8>>)>)
    requires
        forall|i: int| 0 <= i < recs.len() ==> worktree_record_ok(#[trigger] recs[i]),
    ensures
        forall|k: int|
            0 <= k < worktree_fields(recs).len() ==> nul_free(#[trigger] worktree_fields(recs)[k]),
    decreases recs.len(),
{
    if recs.len() > 0 {
        let rest = recs.drop_first();
        assert forall|i: int| 0 <= i < rest.len() implies worktree_record_ok(#[trigger] rest[i]) by {
            assert(rest[i] == recs[i + 1]);
        }
        lemma_worktree_fields_nul_free(rest);
        let r = recs[0];
        assert(worktree_record_ok(r));
        let wt = worktree_field_prefix() + r.0;
        let head = seq![wt] + r.1 + seq![Seq::<u8>::empty()];
        let fs = worktree_fields(recs);
        assert(fs == head + worktree_fields(rest));
        assert forall|k: int| 0 <= k < fs.len() implies nul_free(#[trigger] fs[k]) by {
            if k == 0 {
                assert(fs[0] == wt);
                if wt.contains(0u8) {
                    let j = choose|j: int| 0 <= j < wt.len() && wt[j] == 0u8;
                    if j >= 9 {
                        assert(wt[j] == r.0[j - 9]);
                    }
                }
            } else if k < 1 + r.1.len() {
                assert(fs[k] == r.1[k - 1]);
            } else if k == 1 + r.1.len() {
                assert(fs[k] == Seq::<u8>::empty());
            } else {
                assert(fs[k] == worktree_fields(rest)[k - head.len()]);
            }
        }
    }
}

proof fn lemma_worktree_records_of_fields(recs: Seq<(Seq<u8>, Seq<Seq<u8>>)>)
    requires
        forall|i: int| 0 <= i < recs.len() ==> worktree_record_ok(#[trigger] recs[i]),
    ensures
        worktree_records(worktree_fields(recs).push(Seq::<u8>::empty())) == worktree_expected(
            recs,
        ),
    decreases recs.len(),
{
    if recs.len() == 0 {
        let fs = worktree_fields(recs).push(Seq::<u8>::empty());
        assert(fs.len() == 1 && fs[0] == Seq::<u8>::empty());
        assert(fs.drop_first() =~= Seq::<Seq<u8>>::empty());
        assert(bare_field().len() == 4);
        assert(fs[0] != bare_field());
        assert(!starts_record(fs[0]));
        assert(worktree_records(fs) == mark_bare(false, worktree_records(fs.drop_first())));
        assert(worktree_expected(recs) =~= Seq::<(Seq<u8>, bool)>::empty());
    } else {
        let rest = recs.drop_first();
        assert forall|i: int| 0 <= i < rest.len() implies worktree_record_ok(#[trigger] rest[i]) by {
            assert(rest[i] == recs[i + 1]);
        }
        lemma_worktree_records_of_fields(rest);
        let r = recs[0];
        assert(worktree_record_ok(r));
        let attrs = r.1;
        let n = attrs.len();
        let wt = worktree_field_prefix() + r.0;
        let tail = worktree_fields(rest).push(Seq::<u8>::empty());
        let fs = worktree_fields(recs).push(Seq::<u8>::empty());
        assert(fs =~= seq![wt] + attrs + seq![Seq::<u8>::empty()] + tail);
        assert(fs[0] == wt);
        assert(wt.subrange(0, 9) =~= worktree_field_prefix());
        assert(starts_record(wt));
        let body = fs.drop_first();
        assert forall|k: int| 0 <= k < n implies !ends_record(#[trigger] body[k]) by {
            assert(body[k] == attrs[k]);
        }
        assert(body[n as int] == Seq::<u8>::empty());
        lemma_body_end(body, 0, n as int);
        assert(body.subrange(0, body.len() as int) == body);
        assert(body_len(body) == n);
        assert(fs.subrange(1, 1 + n as int) =~= attrs);
        assert(wt.subrange(9, wt.len() as int) =~= r.0);
        let after = fs.subrange(1 + n as int, fs.len() as int);
        assert(after =~= seq![Seq::<u8>::empty()] + tail);
        assert(after[0] == Seq::<u8>::empty());
        assert(after.drop_first() =~= tail);
        assert(after[0] != bare_field());
        assert(bare_field().len() == 4);
        assert(!starts_record(after[0]));
        assert(worktree_records(after) == mark_bare(false, worktree_records(after.drop_first())));
        assert(worktree_records(after) == worktree_records(tail));
        assert(worktree_records(fs) == seq![(r.0, has_bare(attrs))] + worktree_records(after));
        assert(worktree_expected(recs) =~= seq![(r.0, has_bare(attrs))] + worktree_expected(rest));
    }
}

/// `worktree list --porcelain -z` round trip: for any well-formed records
/// (see `worktree_record_ok`), parsing what Git writes yields exactly each
/// path, in order, bare exactly when the record has a `bare` attribute. (The
/// stray-`bare` quirk of [`parse_worktree_list`] never applies to such
/// output.)
pub proof fn theorem_worktree_list_round_trip(recs: Seq<(Seq<u8>, Seq<Seq<u8>>)>)
    requires
        forall|i: int| 0 <= i < recs.len() ==> worktree_record_ok(#[trigger] recs[i]),
    ensures
        worktree_records(nul_fields(nul_terminated(worktree_fields(recs)))) == worktree_expected(
            recs,
        ),
{
    lemma_worktree_fields_nul_free(recs);
    theorem_nul_terminated_fields(worktree_fields(recs));
    lemma_worktree_records_of_fields(recs);
}

} // verus!
