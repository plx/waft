//! Environment layer: which `WAFT_*` variables are policy inputs, and how
//! `WAFT_REPLACE_EXTRA_EXCLUDES` is read as a boolean.
//!
//! # What it decides
//!
//! [`EnvKey::classify`] decides whether an environment variable is one of
//! the eight policy variables (production `layer_from_env_iter` matches the
//! result exhaustively and sets the corresponding layer field; enum values
//! go through the shared [`super::vocab`] parsers, so TOML and the
//! environment accept the same spellings). [`parse_env_bool`] decides the
//! boolean value of `WAFT_REPLACE_EXTRA_EXCLUDES`, and [`parse_exclude_list`]
//! turns `WAFT_EXTRA_EXCLUDE` into globs.
//!
//! # Theorems
//!
//! - `classify` equals `EnvKey::spec_classify` ("the key whose name is
//!   exactly `k`"); `theorem_env_key_table`: the table lists each key once,
//!   names are distinct and start with `WAFT_`, and classification is exact
//!   (case variants, prefixes and suffixes are ignored). `as_str` returns the
//!   name, so the error context production prints is the variable's name.
//! - `parse_env_bool` decides `parse_bool_token` on
//!   `ascii_lowercase(trim(v))` (`str::trim` then `str::to_ascii_lowercase`);
//!   `parse_bool_token`: `Some(true)` exactly for `1`, `true`, `yes`, `on`;
//!   `Some(false)` exactly for `0`, `false`, `no`, `off` and the empty
//!   string; `None` otherwise. `theorem_bool_tokens_disjoint`: no spelling is
//!   both. So the value is read case-insensitively (ASCII) and ignoring
//!   surrounding whitespace, unlike the enum-valued variables, which are
//!   exact.
//! - `parse_exclude_list` equals `exclude_list`: the pieces between commas
//!   (`split_commas`, which is `str::split(',')`), each trimmed of Unicode
//!   `White_Space` at both ends (`trim`, which is `str::trim`), without the
//!   empty ones, in order. `theorem_exclude_list_globs`: every glob is
//!   non-empty, starts and ends with a non-whitespace character, contains no
//!   comma, and is a contiguous part of the value. (Quirk: a comma can
//!   therefore not be expressed, so `*.{log,tmp}` becomes `*.{log` and
//!   `tmp}`.)
//!
//! # Trust
//!
//! vstd's `White_Space` table for `char::is_whitespace` (the 25 code points
//! of the Unicode property `str::trim` uses) and its `Chars`, `Vec` and
//! `String::push` specifications, and `char::to_ascii_lowercase`. The error
//! message for an invalid boolean is built in production from
//! `str::trim` and `str::to_ascii_lowercase` of the value; the loop over
//! `std::env::vars()`
//! (which panics on a non-UTF-8 variable anywhere in the environment) and its
//! last-occurrence-wins and first-error order stay in production.

#[allow(unused_imports)]
use vstd::prelude::*;
#[cfg(verus_keep_ghost)]
use vstd::std_specs::char::{is_white_space, to_ascii_lowercase};

verus! {

/// A recognized policy environment variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvKey {
    /// `WAFT_COMPAT_PROFILE`.
    CompatProfile,
    /// `WAFT_WHEN_MISSING_WORKTREEINCLUDE`.
    WhenMissingWorktreeinclude,
    /// `WAFT_WORKTREEINCLUDE_SEMANTICS`.
    WorktreeincludeSemantics,
    /// `WAFT_WORKTREEINCLUDE_SYMLINK_POLICY`.
    WorktreeincludeSymlinkPolicy,
    /// `WAFT_BUILTIN_EXCLUDE_SET`.
    BuiltinExcludeSet,
    /// `WAFT_EXTRA_EXCLUDE`.
    ExtraExclude,
    /// `WAFT_REPLACE_EXTRA_EXCLUDES`.
    ReplaceExtraExcludes,
    /// `WAFT_COPY_STRATEGY`.
    CopyStrategy,
}

impl EnvKey {
    /// Every recognized variable, in declaration order.
    pub open spec fn all() -> Seq<EnvKey> {
        seq![
            EnvKey::CompatProfile,
            EnvKey::WhenMissingWorktreeinclude,
            EnvKey::WorktreeincludeSemantics,
            EnvKey::WorktreeincludeSymlinkPolicy,
            EnvKey::BuiltinExcludeSet,
            EnvKey::ExtraExclude,
            EnvKey::ReplaceExtraExcludes,
            EnvKey::CopyStrategy,
        ]
    }

    /// The variable's exact name.
    pub open spec fn name(self) -> Seq<char> {
        match self {
            EnvKey::CompatProfile => "WAFT_COMPAT_PROFILE"@,
            EnvKey::WhenMissingWorktreeinclude => "WAFT_WHEN_MISSING_WORKTREEINCLUDE"@,
            EnvKey::WorktreeincludeSemantics => "WAFT_WORKTREEINCLUDE_SEMANTICS"@,
            EnvKey::WorktreeincludeSymlinkPolicy => "WAFT_WORKTREEINCLUDE_SYMLINK_POLICY"@,
            EnvKey::BuiltinExcludeSet => "WAFT_BUILTIN_EXCLUDE_SET"@,
            EnvKey::ExtraExclude => "WAFT_EXTRA_EXCLUDE"@,
            EnvKey::ReplaceExtraExcludes => "WAFT_REPLACE_EXTRA_EXCLUDES"@,
            EnvKey::CopyStrategy => "WAFT_COPY_STRATEGY"@,
        }
    }

    /// The recognized variable named exactly `k`, if any.
    pub open spec fn spec_classify(k: Seq<char>) -> Option<EnvKey> {
        if exists|x: EnvKey| x.name() == k {
            Some(choose|x: EnvKey| x.name() == k)
        } else {
            None
        }
    }

    /// The variable's name.
    pub fn as_str(self) -> (s: &'static str)
        ensures
            s@ == self.name(),
    {
        match self {
            EnvKey::CompatProfile => "WAFT_COMPAT_PROFILE",
            EnvKey::WhenMissingWorktreeinclude => "WAFT_WHEN_MISSING_WORKTREEINCLUDE",
            EnvKey::WorktreeincludeSemantics => "WAFT_WORKTREEINCLUDE_SEMANTICS",
            EnvKey::WorktreeincludeSymlinkPolicy => "WAFT_WORKTREEINCLUDE_SYMLINK_POLICY",
            EnvKey::BuiltinExcludeSet => "WAFT_BUILTIN_EXCLUDE_SET",
            EnvKey::ExtraExclude => "WAFT_EXTRA_EXCLUDE",
            EnvKey::ReplaceExtraExcludes => "WAFT_REPLACE_EXTRA_EXCLUDES",
            EnvKey::CopyStrategy => "WAFT_COPY_STRATEGY",
        }
    }

    /// Classifies an environment variable name (case-sensitive, exact).
    pub fn classify(k: &str) -> (r: Option<EnvKey>)
        ensures
            r == EnvKey::spec_classify(k@),
    {
        proof {
            theorem_env_key_table();
        }
        if k == "WAFT_COMPAT_PROFILE" {
            assert(k@ == EnvKey::CompatProfile.name());
            Some(EnvKey::CompatProfile)
        } else if k == "WAFT_WHEN_MISSING_WORKTREEINCLUDE" {
            assert(k@ == EnvKey::WhenMissingWorktreeinclude.name());
            Some(EnvKey::WhenMissingWorktreeinclude)
        } else if k == "WAFT_WORKTREEINCLUDE_SEMANTICS" {
            assert(k@ == EnvKey::WorktreeincludeSemantics.name());
            Some(EnvKey::WorktreeincludeSemantics)
        } else if k == "WAFT_WORKTREEINCLUDE_SYMLINK_POLICY" {
            assert(k@ == EnvKey::WorktreeincludeSymlinkPolicy.name());
            Some(EnvKey::WorktreeincludeSymlinkPolicy)
        } else if k == "WAFT_BUILTIN_EXCLUDE_SET" {
            assert(k@ == EnvKey::BuiltinExcludeSet.name());
            Some(EnvKey::BuiltinExcludeSet)
        } else if k == "WAFT_EXTRA_EXCLUDE" {
            assert(k@ == EnvKey::ExtraExclude.name());
            Some(EnvKey::ExtraExclude)
        } else if k == "WAFT_REPLACE_EXTRA_EXCLUDES" {
            assert(k@ == EnvKey::ReplaceExtraExcludes.name());
            Some(EnvKey::ReplaceExtraExcludes)
        } else if k == "WAFT_COPY_STRATEGY" {
            assert(k@ == EnvKey::CopyStrategy.name());
            Some(EnvKey::CopyStrategy)
        } else {
            assert forall|x: EnvKey| x.name() != k@ by {
                match x {
                    EnvKey::CompatProfile => {},
                    EnvKey::WhenMissingWorktreeinclude => {},
                    EnvKey::WorktreeincludeSemantics => {},
                    EnvKey::WorktreeincludeSymlinkPolicy => {},
                    EnvKey::BuiltinExcludeSet => {},
                    EnvKey::ExtraExclude => {},
                    EnvKey::ReplaceExtraExcludes => {},
                    EnvKey::CopyStrategy => {},
                }
            }
            None
        }
    }
}

/// The key table: `all()` lists every key once, names are distinct and all
/// start with `WAFT_`, and classification is exact (a name is recognized
/// only when it equals a key's name, so case variants such as
/// `waft_compat_profile`, prefixes and suffixes are ignored).
pub proof fn theorem_env_key_table()
    ensures
        forall|x: EnvKey| EnvKey::all().contains(x),
        EnvKey::all().no_duplicates(),
        forall|x: EnvKey, y: EnvKey| x.name() == y.name() ==> x == y,
        forall|x: EnvKey| #[trigger] x.name().len() > 5 && x.name().take(5) == "WAFT_"@,
        forall|x: EnvKey| EnvKey::spec_classify(#[trigger] x.name()) == Some(x),
        forall|k: Seq<char>|
            (#[trigger] EnvKey::spec_classify(k)) matches Some(x) ==> x.name() == k,
{
    reveal_strlit("WAFT_");
    reveal_strlit("WAFT_COMPAT_PROFILE");
    reveal_strlit("WAFT_WHEN_MISSING_WORKTREEINCLUDE");
    reveal_strlit("WAFT_WORKTREEINCLUDE_SEMANTICS");
    reveal_strlit("WAFT_WORKTREEINCLUDE_SYMLINK_POLICY");
    reveal_strlit("WAFT_BUILTIN_EXCLUDE_SET");
    reveal_strlit("WAFT_EXTRA_EXCLUDE");
    reveal_strlit("WAFT_REPLACE_EXTRA_EXCLUDES");
    reveal_strlit("WAFT_COPY_STRATEGY");
    let all = EnvKey::all();
    assert forall|x: EnvKey| all.contains(x) by {
        match x {
            EnvKey::CompatProfile => assert(all[0] == x),
            EnvKey::WhenMissingWorktreeinclude => assert(all[1] == x),
            EnvKey::WorktreeincludeSemantics => assert(all[2] == x),
            EnvKey::WorktreeincludeSymlinkPolicy => assert(all[3] == x),
            EnvKey::BuiltinExcludeSet => assert(all[4] == x),
            EnvKey::ExtraExclude => assert(all[5] == x),
            EnvKey::ReplaceExtraExcludes => assert(all[6] == x),
            EnvKey::CopyStrategy => assert(all[7] == x),
        }
    }
    assert forall|x: EnvKey| #[trigger] x.name().len() > 5 && x.name().take(5) == "WAFT_"@ by {
        assert(x.name().take(5) =~= "WAFT_"@);
    }
    // Distinct names differ in length or at one of a few positions.
    assert forall|x: EnvKey, y: EnvKey| x.name() == y.name() implies x == y by {
        if x != y {
            assert(x.name().len() != y.name().len() || x.name()[5] != y.name()[5]
                || x.name()[6] != y.name()[6] || x.name()[13] != y.name()[13]);
        }
    }
}

/// Spellings `WAFT_REPLACE_EXTRA_EXCLUDES` reads as `true`, after trimming
/// and ASCII lowercasing.
pub open spec fn is_true_token(t: Seq<char>) -> bool {
    t == "1"@ || t == "true"@ || t == "yes"@ || t == "on"@
}

/// Spellings read as `false`; the empty string is one of them.
pub open spec fn is_false_token(t: Seq<char>) -> bool {
    t == "0"@ || t == "false"@ || t == "no"@ || t == "off"@ || t == ""@
}

/// Decides a boolean from an already trimmed and ASCII-lowercased value.
pub fn parse_bool_token(t: &str) -> (r: Option<bool>)
    ensures
        r == Some(true) <==> is_true_token(t@),
        r == Some(false) <==> is_false_token(t@),
        r is None <==> !is_true_token(t@) && !is_false_token(t@),
{
    proof {
        theorem_bool_tokens_disjoint();
        reveal_strlit("");
        assert(t@.len() == 0 <==> t@ =~= ""@);
    }
    if t == "1" || t == "true" || t == "yes" || t == "on" {
        Some(true)
    } else if t == "0" || t == "false" || t == "no" || t == "off" || t.is_empty() {
        Some(false)
    } else {
        None
    }
}

/// No spelling is both a `true` and a `false` token.
pub proof fn theorem_bool_tokens_disjoint()
    ensures
        forall|t: Seq<char>| !(#[trigger] is_true_token(t) && is_false_token(t)),
{
    reveal_strlit("1");
    reveal_strlit("true");
    reveal_strlit("yes");
    reveal_strlit("on");
    reveal_strlit("0");
    reveal_strlit("false");
    reveal_strlit("no");
    reveal_strlit("off");
    reveal_strlit("");
    assert forall|t: Seq<char>| !(#[trigger] is_true_token(t) && is_false_token(t)) by {
        if is_true_token(t) && is_false_token(t) {
            assert(t.len() == 0 || t[0] == '1' || t[0] == 't' || t[0] == 'y' || t[0] == 'o');
            assert(t.len() == 0 || t[0] == '0' || t[0] == 'f' || t[0] == 'n' || t[0] == 'o');
            if t.len() > 0 && t[0] == 'o' {
                assert(t == "on"@ && t == "off"@);
            }
        }
    }
}


// ---------------------------------------------------------------------------
// WAFT_EXTRA_EXCLUDE

/// The pieces of `s` between commas (`str::split(',')`): always at least
/// one piece, and an empty piece before, between or after commas.
pub open spec fn split_commas(s: Seq<char>) -> Seq<Seq<char>>
    decreases s.len(),
{
    if s.len() == 0 {
        seq![Seq::empty()]
    } else {
        let prev = split_commas(s.drop_last());
        if s.last() == ',' {
            prev.push(Seq::empty())
        } else {
            prev.update(prev.len() - 1, prev.last().push(s.last()))
        }
    }
}

/// `s` without leading Unicode `White_Space` (`str::trim_start`).
pub open spec fn trim_start(s: Seq<char>) -> Seq<char>
    decreases s.len(),
{
    if s.len() > 0 && is_white_space(s[0]) {
        trim_start(s.drop_first())
    } else {
        s
    }
}

/// `s` without trailing Unicode `White_Space` (`str::trim_end`).
pub open spec fn trim_end(s: Seq<char>) -> Seq<char>
    decreases s.len(),
{
    if s.len() > 0 && is_white_space(s.last()) {
        trim_end(s.drop_last())
    } else {
        s
    }
}

/// `s` without leading or trailing Unicode `White_Space` (`str::trim`).
pub open spec fn trim(s: Seq<char>) -> Seq<char> {
    trim_end(trim_start(s))
}

/// The trimmed pieces that are not empty, in order.
pub open spec fn nonempty_trimmed(pieces: Seq<Seq<char>>) -> Seq<Seq<char>>
    decreases pieces.len(),
{
    if pieces.len() == 0 {
        Seq::empty()
    } else {
        let rest = nonempty_trimmed(pieces.drop_last());
        if trim(pieces.last()).len() > 0 {
            rest.push(trim(pieces.last()))
        } else {
            rest
        }
    }
}

/// The globs `WAFT_EXTRA_EXCLUDE` denotes: the comma-separated pieces,
/// trimmed, without the empty ones (`v.split(',').map(str::trim)
/// .filter(|s| !s.is_empty())`).
pub open spec fn exclude_list(v: Seq<char>) -> Seq<Seq<char>> {
    nonempty_trimmed(split_commas(v))
}

/// The character views of `v`.
pub open spec fn views(v: Seq<String>) -> Seq<Seq<char>> {
    v.map_values(|s: String| s@)
}

proof fn lemma_split_commas_nonempty(s: Seq<char>)
    ensures
        split_commas(s).len() >= 1,
    decreases s.len(),
{
    if s.len() > 0 {
        lemma_split_commas_nonempty(s.drop_last());
    }
}

/// The bounds of `trim(chars)` within `chars`.
fn trim_range(chars: &[char]) -> (r: (usize, usize))
    ensures
        r.0 <= r.1 <= chars@.len(),
        trim(chars@) == chars@.subrange(r.0 as int, r.1 as int),
{
    let n = chars.len();
    let mut a: usize = 0;
    proof {
        assert(chars@.subrange(0, n as int) =~= chars@);
    }
    while a < n && chars[a].is_whitespace()
        invariant
            a <= n,
            n == chars@.len(),
            trim_start(chars@) == trim_start(chars@.subrange(a as int, n as int)),
        decreases n - a,
    {
        proof {
            assert(chars@.subrange(a as int, n as int).drop_first() =~= chars@.subrange(
                a + 1,
                n as int,
            ));
        }
        a += 1;
    }
    let ghost started = chars@.subrange(a as int, n as int);
    proof {
        assert(trim_start(started) == started);
    }
    let mut b: usize = n;
    while b > a && chars[b - 1].is_whitespace()
        invariant
            a <= b <= n,
            n == chars@.len(),
            trim_end(started) == trim_end(chars@.subrange(a as int, b as int)),
            started == chars@.subrange(a as int, n as int),
        decreases b - a,
    {
        proof {
            assert(chars@.subrange(a as int, b as int).drop_last() =~= chars@.subrange(
                a as int,
                b - 1,
            ));
        }
        b -= 1;
    }
    proof {
        assert(trim_end(chars@.subrange(a as int, b as int)) == chars@.subrange(a as int, b as int));
    }
    (a, b)
}

/// Appends `trim(piece)` to `out` when it is not empty.
fn push_trimmed(out: &mut Vec<String>, piece: &[char])
    ensures
        views(final(out)@) == if trim(piece@).len() > 0 {
            views(old(out)@).push(trim(piece@))
        } else {
            views(old(out)@)
        },
{
    let (a, b) = trim_range(piece);
    if a < b {
        let mut t = String::new();
        let mut k: usize = a;
        while k < b
            invariant
                a <= k <= b <= piece@.len(),
                t@ == piece@.subrange(a as int, k as int),
            decreases b - k,
        {
            t.push(piece[k]);
            proof {
                assert(piece@.subrange(a as int, k + 1) =~= piece@.subrange(a as int, k as int).push(
                    piece@[k as int],
                ));
            }
            k += 1;
        }
        let ghost before = out@;
        out.push(t);
        proof {
            assert(views(out@) =~= views(before).push(t@));
        }
    }
}

/// `s` with ASCII letters lowercased (`str::to_ascii_lowercase`).
pub open spec fn ascii_lowercase(s: Seq<char>) -> Seq<char> {
    s.map_values(|c: char| to_ascii_lowercase(c))
}

/// Reads `WAFT_REPLACE_EXTRA_EXCLUDES`: the token decision on the value
/// trimmed of Unicode whitespace and ASCII-lowercased.
pub fn parse_env_bool(v: &str) -> (r: Option<bool>)
    ensures
        r == Some(true) <==> is_true_token(ascii_lowercase(trim(v@))),
        r == Some(false) <==> is_false_token(ascii_lowercase(trim(v@))),
        r is None <==> !is_true_token(ascii_lowercase(trim(v@))) && !is_false_token(
            ascii_lowercase(trim(v@)),
        ),
{
    let mut chars: Vec<char> = Vec::new();
    for c in it: v.chars()
        invariant
            it.seq() == v@,
            chars@ == v@.subrange(0, it.index() as int),
    {
        proof {
            assert(v@.subrange(0, it.index() + 1) == v@.subrange(0, it.index() as int).push(c));
        }
        chars.push(c);
    }
    proof {
        assert(v@.subrange(0, v@.len() as int) =~= v@);
    }
    let (a, b) = trim_range(chars.as_slice());
    let mut lowered = String::new();
    let mut k: usize = a;
    while k < b
        invariant
            a <= k <= b <= chars@.len(),
            lowered@ == ascii_lowercase(chars@.subrange(a as int, k as int)),
        decreases b - k,
    {
        lowered.push(chars[k].to_ascii_lowercase());
        proof {
            assert(ascii_lowercase(chars@.subrange(a as int, k + 1)) =~= ascii_lowercase(
                chars@.subrange(a as int, k as int),
            ).push(to_ascii_lowercase(chars@[k as int])));
        }
        k += 1;
    }
    parse_bool_token(lowered.as_str())
}

/// Parses `WAFT_EXTRA_EXCLUDE`: split on commas, trim each piece, drop the
/// empty ones.
pub fn parse_exclude_list(v: &str) -> (r: Vec<String>)
    ensures
        views(r@) == exclude_list(v@),
{
    let mut out: Vec<String> = Vec::new();
    let mut cur: Vec<char> = Vec::new();
    let ghost mut done: Seq<Seq<char>> = Seq::empty();
    proof {
        assert(views(out@) =~= Seq::<Seq<char>>::empty());
        assert(v@.subrange(0, 0) =~= Seq::<char>::empty());
        assert(split_commas(Seq::<char>::empty()) =~= done.push(cur@));
    }
    for c in it: v.chars()
        invariant
            it.seq() == v@,
            split_commas(v@.subrange(0, it.index() as int)) == done.push(cur@),
            views(out@) == nonempty_trimmed(done),
    {
        let ghost prefix = v@.subrange(0, it.index() as int);
        proof {
            assert(v@.subrange(0, it.index() + 1) == prefix.push(c));
            assert(prefix.push(c).drop_last() =~= prefix);
            lemma_split_commas_nonempty(prefix);
        }
        if c == ',' {
            push_trimmed(&mut out, cur.as_slice());
            proof {
                assert(done.push(cur@).drop_last() =~= done);
                done = done.push(cur@);
            }
            cur = Vec::new();
            proof {
                assert(split_commas(prefix.push(c)) == split_commas(prefix).push(Seq::<char>::empty()));
                assert(split_commas(prefix.push(c)) =~= done.push(cur@));
            }
        } else {
            let ghost old_cur = cur@;
            cur.push(c);
            proof {
                let prev = done.push(old_cur);
                assert(split_commas(prefix.push(c)) == prev.update(prev.len() - 1, prev.last().push(c)));
                assert(prev.update(prev.len() - 1, prev.last().push(c)) =~= done.push(cur@));
            }
        }
    }
    push_trimmed(&mut out, cur.as_slice());
    proof {
        assert(v@.subrange(0, v@.len() as int) =~= v@);
        assert(done.push(cur@).drop_last() =~= done);
    }
    out
}


proof fn lemma_trim_start(s: Seq<char>) -> (a: int)
    ensures
        0 <= a <= s.len(),
        trim_start(s) == s.subrange(a, s.len() as int),
        trim_start(s).len() > 0 ==> !is_white_space(trim_start(s)[0]),
    decreases s.len(),
{
    if s.len() > 0 && is_white_space(s[0]) {
        let a = lemma_trim_start(s.drop_first());
        assert(s.drop_first().subrange(a, s.drop_first().len() as int) =~= s.subrange(
            a + 1,
            s.len() as int,
        ));
        a + 1
    } else {
        assert(s.subrange(0, s.len() as int) =~= s);
        0
    }
}

proof fn lemma_trim_end(s: Seq<char>) -> (b: int)
    ensures
        0 <= b <= s.len(),
        trim_end(s) == s.subrange(0, b),
        trim_end(s).len() > 0 ==> !is_white_space(trim_end(s).last()),
    decreases s.len(),
{
    if s.len() > 0 && is_white_space(s.last()) {
        let b = lemma_trim_end(s.drop_last());
        assert(s.drop_last().subrange(0, b) =~= s.subrange(0, b));
        b
    } else {
        assert(s.subrange(0, s.len() as int) =~= s);
        s.len() as int
    }
}

/// A non-empty trim starts and ends with a non-whitespace character and is
/// a contiguous part of the original.
proof fn lemma_trim(s: Seq<char>) -> (ab: (int, int))
    ensures
        0 <= ab.0 <= ab.1 <= s.len(),
        trim(s) == s.subrange(ab.0, ab.1),
        trim(s).len() > 0 ==> !is_white_space(trim(s)[0]) && !is_white_space(trim(s).last()),
{
    let a = lemma_trim_start(s);
    let t = trim_start(s);
    let b = lemma_trim_end(t);
    assert(t.subrange(0, b) =~= s.subrange(a, a + b));
    if trim(s).len() > 0 {
        assert(trim(s)[0] == t[0]);
    }
    (a, a + b)
}

proof fn lemma_split_commas_no_comma(s: Seq<char>, i: int, k: int)
    requires
        0 <= i < split_commas(s).len(),
        0 <= k < split_commas(s)[i].len(),
    ensures
        split_commas(s)[i][k] != ',',
    decreases s.len(),
{
    if s.len() > 0 {
        let prev = split_commas(s.drop_last());
        lemma_split_commas_nonempty(s.drop_last());
        if s.last() == ',' {
            if i < prev.len() {
                lemma_split_commas_no_comma(s.drop_last(), i, k);
            }
        } else {
            if i < prev.len() - 1 {
                lemma_split_commas_no_comma(s.drop_last(), i, k);
            } else if k < prev.last().len() {
                lemma_split_commas_no_comma(s.drop_last(), i, k);
            }
        }
    }
}

proof fn lemma_nonempty_trimmed(ps: Seq<Seq<char>>, j: int) -> (i: int)
    requires
        0 <= j < nonempty_trimmed(ps).len(),
    ensures
        0 <= i < ps.len(),
        nonempty_trimmed(ps)[j] == trim(ps[i]),
        trim(ps[i]).len() > 0,
    decreases ps.len(),
{
    let rest = nonempty_trimmed(ps.drop_last());
    if j < rest.len() {
        let i = lemma_nonempty_trimmed(ps.drop_last(), j);
        assert(ps.drop_last()[i] == ps[i]);
        i
    } else {
        (ps.len() - 1) as int
    }
}

/// Every glob parsed from `WAFT_EXTRA_EXCLUDE` is non-empty, starts and
/// ends with a non-whitespace character, contains no comma, and is a
/// contiguous part of the variable's value.
pub proof fn theorem_exclude_list_globs(v: Seq<char>, j: int)
    requires
        0 <= j < exclude_list(v).len(),
    ensures
        ({
            let g = exclude_list(v)[j];
            &&& g.len() > 0
            &&& !is_white_space(g[0])
            &&& !is_white_space(g.last())
            &&& forall|k: int| 0 <= k < g.len() ==> g[k] != ','
            &&& exists|a: int|
                0 <= a && a + g.len() <= v.len() && #[trigger] v.subrange(a, a + g.len()) == g
        }),
{
    let ps = split_commas(v);
    let i = lemma_nonempty_trimmed(ps, j);
    let g = exclude_list(v)[j];
    let (a, b) = lemma_trim(ps[i]);
    assert forall|k: int| 0 <= k < g.len() implies g[k] != ',' by {
        assert(g[k] == ps[i][a + k]);
        lemma_split_commas_no_comma(v, i, a + k);
    }
    let (start, _) = lemma_split_commas_piece_in(v, i);
    assert(v.subrange(start + a, start + a + g.len()) =~= g) by {
        assert forall|k: int| 0 <= k < g.len() implies v.subrange(start + a, start + a + g.len())[k]
            == g[k] by {
            assert(g[k] == ps[i][a + k]);
        }
    }
}

/// Each comma-separated piece is a contiguous part of the input.
proof fn lemma_split_commas_piece_in(s: Seq<char>, i: int) -> (r: (int, int))
    requires
        0 <= i < split_commas(s).len(),
    ensures
        0 <= r.0 <= r.1 <= s.len(),
        split_commas(s)[i] == s.subrange(r.0, r.1),
        i == split_commas(s).len() - 1 ==> r.1 == s.len(),
    decreases s.len(),
{
    if s.len() == 0 {
        (0, 0)
    } else {
        let p = s.drop_last();
        let prev = split_commas(p);
        lemma_split_commas_nonempty(p);
        if s.last() == ',' {
            if i < prev.len() {
                let (x, y) = lemma_split_commas_piece_in(p, i);
                assert(p.subrange(x, y) =~= s.subrange(x, y));
                (x, y)
            } else {
                assert(s.subrange(s.len() as int, s.len() as int) =~= Seq::<char>::empty());
                (s.len() as int, s.len() as int)
            }
        } else {
            let (x, y) = lemma_split_commas_piece_in(p, i);
            if i < prev.len() - 1 {
                assert(p.subrange(x, y) =~= s.subrange(x, y));
                (x, y)
            } else {
                assert(p.subrange(x, y).push(s.last()) =~= s.subrange(x, y + 1));
                (x, y + 1)
            }
        }
    }
}

} // verus!
