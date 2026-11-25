//! Verified kernel: the target a `.git` gitfile points to.
//!
//! # What it decides
//!
//! A linked worktree or submodule checkout has a `.git` *file* holding a
//! `gitdir: <path>` pointer. The boundary probe
//! `DotGitFileTargetExists` reads the file, asks [`gitdir_target`] for the
//! pointer text, and checks whether that path exists. The parse mirrors the
//! production code it replaced exactly:
//!
//! ```text
//! for line in content.lines() {
//!     if let Some(rest) = line.strip_prefix("gitdir:") {
//!         let target = rest.trim();
//!         return if target.is_empty() { None } else { Some(target) };
//!     }
//! }
//! None
//! ```
//!
//! That is: the *first* line (not necessarily the first line of the file)
//! that starts with `gitdir:` decides; its remainder is trimmed of Unicode
//! `White_Space` on both ends; an empty remainder means "no pointer" (later
//! lines are not consulted). This is looser than Git, which requires the file
//! to start with `gitdir: `; the quirk is kept, not fixed.
//!
//! # Theorems
//!
//! - [`gitdir_target`] equals `gitdir_target_spec`, stated over
//!   `line_at`, which models `str::lines` (split at `\n`, and drop one
//!   `\r` before a `\n`).
//! - `lemma_first_tag_exists`: when some line starts with `gitdir:`, a
//!   first one exists, so the specification is total and deterministic.
//! - `lemma_trim_absorbs_trailing_whitespace`: dropping one trailing
//!   whitespace character does not change `trim`, so the `\r` that
//!   `str::lines` strips never matters after trimming.
//!
//! # Trust assumptions
//!
//! - vstd's `char::is_whitespace` specification is Unicode `White_Space`,
//!   the property std's `str::trim` uses.
//! - Production reads the file with `fs::read_to_string` (non-UTF-8 content
//!   is "no pointer", as before) and resolves a relative target against the
//!   gitfile's directory itself.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// Unicode `White_Space` (what `char::is_whitespace` and `str::trim` use).
pub open spec fn ws(c: char) -> bool {
    vstd::std_specs::char::is_white_space(c)
}

/// `str::trim_start`.
pub open spec fn trim_start(x: Seq<char>) -> Seq<char>
    decreases x.len(),
{
    if x.len() > 0 && ws(x[0]) {
        trim_start(x.drop_first())
    } else {
        x
    }
}

/// `str::trim_end`.
pub open spec fn trim_end(x: Seq<char>) -> Seq<char>
    decreases x.len(),
{
    if x.len() > 0 && ws(x.last()) {
        trim_end(x.drop_last())
    } else {
        x
    }
}

/// `str::trim`.
pub open spec fn trim(x: Seq<char>) -> Seq<char> {
    trim_end(trim_start(x))
}

/// `"gitdir:"`.
pub open spec fn gitdir_tag() -> Seq<char> {
    seq!['g', 'i', 't', 'd', 'i', 'r', ':']
}

/// The first `\n` at or after `from`, or the end of `s`.
pub open spec fn line_end(s: Seq<char>, from: int) -> int
    decreases s.len() - from,
{
    if from >= s.len() {
        s.len() as int
    } else if s[from] == '\n' {
        from
    } else {
        line_end(s, from + 1)
    }
}

/// `p` is where a line starts.
pub open spec fn is_line_start(s: Seq<char>, p: int) -> bool {
    p == 0 || (0 < p <= s.len() && s[p - 1] == '\n')
}

/// The line starting at `p` as `str::lines` yields it: up to the next `\n`
/// (exclusive), with one `\r` dropped when the line ends in `\r\n`.
pub open spec fn line_at(s: Seq<char>, p: int) -> Seq<char> {
    let e = line_end(s, p);
    let raw = s.subrange(p, e);
    if e < s.len() && raw.len() > 0 && raw.last() == '\r' {
        raw.drop_last()
    } else {
        raw
    }
}

/// The line starting at `p` starts with `gitdir:`.
pub open spec fn tag_line(s: Seq<char>, p: int) -> bool {
    &&& is_line_start(s, p)
    &&& line_at(s, p).len() >= 7
    &&& line_at(s, p).subrange(0, 7) == gitdir_tag()
}

/// `p` starts the first line beginning with `gitdir:`.
pub open spec fn first_tag_line(s: Seq<char>, p: int) -> bool {
    &&& tag_line(s, p)
    &&& forall|q: int| 0 <= q < p ==> !#[trigger] tag_line(s, q)
}

/// The pointer for the line at `p`: its trimmed remainder after `gitdir:`,
/// or `None` when that is empty.
pub open spec fn pointer_of(s: Seq<char>, p: int) -> Option<Seq<char>> {
    let t = trim(line_at(s, p).skip(7));
    if t.len() == 0 {
        None
    } else {
        Some(t)
    }
}

/// The specification: no line starting with `gitdir:` gives `None`;
/// otherwise the first such line decides.
pub open spec fn gitdir_target_spec(s: Seq<char>, r: Option<Seq<char>>) -> bool {
    &&& (forall|p: int| !#[trigger] tag_line(s, p)) ==> r is None
    &&& forall|p: int| #[trigger] first_tag_line(s, p) ==> r == pointer_of(s, p)
}

/// Properties of `line_end`.
pub proof fn lemma_line_end(s: Seq<char>, from: int)
    requires
        0 <= from <= s.len(),
    ensures
        from <= line_end(s, from) <= s.len(),
        forall|k: int| from <= k < line_end(s, from) ==> s[k] != '\n',
        line_end(s, from) < s.len() ==> s[line_end(s, from)] == '\n',
    decreases s.len() - from,
{
    if from < s.len() && s[from] != '\n' {
        lemma_line_end(s, from + 1);
    }
}

/// `line_end` is the same from any point before the first `\n`.
pub proof fn lemma_line_end_skip(s: Seq<char>, from: int, to: int)
    requires
        0 <= from <= to <= s.len(),
        forall|k: int| from <= k < to ==> s[k] != '\n',
    ensures
        line_end(s, from) == line_end(s, to),
    decreases to - from,
{
    if from < to {
        lemma_line_end_skip(s, from + 1, to);
    }
}

/// Dropping one trailing whitespace character does not change `trim`.
pub proof fn lemma_trim_absorbs_trailing_whitespace(x: Seq<char>)
    requires
        x.len() > 0,
        ws(x.last()),
    ensures
        trim(x.drop_last()) == trim(x),
    decreases x.len(),
{
    if x.len() == 1 {
        assert(x.drop_last() =~= Seq::<char>::empty());
        assert(x.drop_first() =~= Seq::<char>::empty());
    } else if ws(x[0]) {
        let y = x.drop_first();
        assert(y.last() == x.last());
        lemma_trim_absorbs_trailing_whitespace(y);
        assert(x.drop_last().drop_first() =~= y.drop_last());
        assert(x.drop_last()[0] == x[0]);
    } else {
        assert(x.drop_last()[0] == x[0]);
        assert(trim_start(x) == x);
        assert(trim_start(x.drop_last()) == x.drop_last());
    }
}

/// When some line starts with `gitdir:`, a first one exists.
pub proof fn lemma_first_tag_exists(s: Seq<char>, p: int)
    requires
        tag_line(s, p),
    ensures
        exists|q: int| first_tag_line(s, q),
    decreases p,
{
    if exists|q: int| 0 <= q < p && #[trigger] tag_line(s, q) {
        let q = choose|q: int| 0 <= q < p && #[trigger] tag_line(s, q);
        lemma_first_tag_exists(s, q);
    } else {
        assert(first_tag_line(s, p));
    }
}

/// At a line start, the line starts with `gitdir:` exactly when the seven
/// characters there spell it; the pointer is then the trimmed text up to
/// the end of the line.
proof fn lemma_tag_line_bridge(s: Seq<char>, p: int)
    requires
        is_line_start(s, p),
        0 <= p <= s.len(),
    ensures
        tag_line(s, p) <==> (p + 7 <= s.len() && s.subrange(p, p + 7) == gitdir_tag()),
        tag_line(s, p) ==> pointer_of(s, p) == ({
            let t = trim(s.subrange(p + 7, line_end(s, p + 7)));
            if t.len() == 0 {
                None
            } else {
                Some(t)
            }
        }),
{
    lemma_line_end(s, p);
    let e = line_end(s, p);
    let raw = s.subrange(p, e);
    let tag = gitdir_tag();
    if p + 7 <= s.len() && s.subrange(p, p + 7) == tag {
        assert forall|k: int| p <= k < p + 7 implies s[k] != '\n' by {
            assert(s[k] == s.subrange(p, p + 7)[k - p]);
        }
        lemma_line_end_skip(s, p, p + 7);
        assert(e >= p + 7);
        assert(raw.subrange(0, 7) =~= s.subrange(p, p + 7));
        if e < s.len() && raw.len() > 0 && raw.last() == '\r' {
            assert(raw.len() > 7) by {
                if raw.len() == 7 {
                    assert(raw.last() == s.subrange(p, p + 7)[6]);
                }
            }
            assert(raw.drop_last().subrange(0, 7) =~= raw.subrange(0, 7));
            assert(raw.drop_last().skip(7) =~= raw.skip(7).drop_last());
            assert(raw.skip(7).last() == '\r');
            lemma_trim_absorbs_trailing_whitespace(raw.skip(7));
        }
        assert(raw.skip(7) =~= s.subrange(p + 7, line_end(s, p + 7)));
        assert(tag_line(s, p));
    }
    if tag_line(s, p) {
        let line = line_at(s, p);
        assert(line.len() <= raw.len());
        assert(line.subrange(0, 7) =~= raw.subrange(0, 7));
        assert(raw.subrange(0, 7) =~= s.subrange(p, p + 7));
    }
}

/// Whether `cs[p..p + 7]` spells `gitdir:`.
fn has_tag_at(cs: &[char], p: usize) -> (r: bool)
    requires
        p + 7 <= cs@.len(),
    ensures
        r == (cs@.subrange(p as int, p + 7) == gitdir_tag()),
{
    let r = cs[p] == 'g' && cs[p + 1] == 'i' && cs[p + 2] == 't' && cs[p + 3] == 'd' && cs[p + 4]
        == 'i' && cs[p + 5] == 'r' && cs[p + 6] == ':';
    proof {
        let w = cs@.subrange(p as int, p + 7);
        if r {
            assert(w =~= gitdir_tag());
        } else if w == gitdir_tag() {
            assert(w[0] == 'g' && w[1] == 'i' && w[2] == 't' && w[3] == 'd');
            assert(w[4] == 'i' && w[5] == 'r' && w[6] == ':');
        }
    }
    r
}

/// The trimmed text of `cs[from..]` up to the end of its line, or `None`
/// when that is empty.
fn trimmed_rest_of_line(cs: &[char], from: usize) -> (r: Option<String>)
    requires
        from <= cs@.len(),
    ensures
        r matches Some(t) ==> t@ == trim(cs@.subrange(from as int, line_end(cs@, from as int)))
            && t@.len() > 0,
        r is None ==> trim(cs@.subrange(from as int, line_end(cs@, from as int))).len() == 0,
{
    let ghost s = cs@;
    let n = cs.len();
    let mut e = from;
    while e < n && cs[e] != '\n'
        invariant
            s == cs@,
            n == s.len(),
            from <= e <= n,
            line_end(s, from as int) == line_end(s, e as int),
        decreases n - e,
    {
        e += 1;
    }
    let ghost x = s.subrange(from as int, e as int);
    assert(line_end(s, e as int) == e);
    let mut a = from;
    while a < e && cs[a].is_whitespace()
        invariant
            s == cs@,
            from <= a <= e <= n,
            n == s.len(),
            trim_start(x) == trim_start(s.subrange(a as int, e as int)),
        decreases e - a,
    {
        assert(s.subrange(a as int, e as int).drop_first() =~= s.subrange(a + 1, e as int));
        a += 1;
    }
    assert(trim_start(s.subrange(a as int, e as int)) == s.subrange(a as int, e as int));
    let mut b = e;
    while b > a && cs[b - 1].is_whitespace()
        invariant
            s == cs@,
            from <= a <= b <= e <= n,
            n == s.len(),
            trim(x) == trim_end(s.subrange(a as int, b as int)),
        decreases b - a,
    {
        assert(s.subrange(a as int, b as int).drop_last() =~= s.subrange(a as int, b - 1));
        b -= 1;
    }
    assert(trim_end(s.subrange(a as int, b as int)) == s.subrange(a as int, b as int));
    if a == b {
        return None;
    }
    let mut out = String::new();
    let mut k = a;
    while k < b
        invariant
            s == cs@,
            a <= k <= b <= n,
            n == s.len(),
            out@ == s.subrange(a as int, k as int),
        decreases b - k,
    {
        out.push(cs[k]);
        assert(s.subrange(a as int, k + 1) =~= s.subrange(a as int, k as int).push(s[k as int]));
        k += 1;
    }
    Some(out)
}

/// The `gitdir:` pointer text of a gitfile, or `None` when there is none.
pub fn gitdir_target(content: &str) -> (r: Option<String>)
    ensures
        gitdir_target_spec(content@, match r {
            Some(t) => Some(t@),
            None => None,
        }),
{
    let mut cs: Vec<char> = Vec::new();
    for c in it: content.chars()
        invariant
            it.seq() == content@,
            cs@ == content@.subrange(0, it.index() as int),
    {
        cs.push(c);
        assert(content@.subrange(0, it.index() + 1) =~= content@.subrange(
            0,
            it.index() as int,
        ).push(c));
    }
    assert(cs@ =~= content@);
    let ghost s = cs@;
    let n = cs.len();
    let mut p: usize = 0;
    let mut at_line_start = true;
    while p < n
        invariant
            s == cs@,
            s == content@,
            n == s.len(),
            p <= n,
            at_line_start == is_line_start(s, p as int),
            forall|q: int| 0 <= q < p ==> !#[trigger] tag_line(s, q),
        decreases n - p,
    {
        proof {
            if at_line_start {
                lemma_tag_line_bridge(s, p as int);
            }
        }
        if at_line_start && n - p >= 7 && has_tag_at(cs.as_slice(), p) {
            assert(tag_line(s, p as int));
            assert(first_tag_line(s, p as int));
            let r = trimmed_rest_of_line(cs.as_slice(), p + 7);
            proof {
                assert forall|q: int| #[trigger] first_tag_line(s, q) implies q == p by {
                    if q < p {
                        assert(!tag_line(s, q));
                    } else if q > p {
                        assert(!tag_line(s, p as int));
                    }
                }
            }
            return r;
        }
        assert(!tag_line(s, p as int));
        at_line_start = cs[p] == '\n';
        p += 1;
    }
    proof {
        assert forall|q: int| !#[trigger] tag_line(s, q) by {
            if 0 <= q < p {
            } else if q >= 0 && tag_line(s, q) {
                assert(line_at(s, q).len() >= 7);
                lemma_line_end(s, q);
            }
        }
    }
    None
}

} // verus!
