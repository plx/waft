//! Environment layer: which `WAFT_*` variables are policy inputs, and how
//! `WAFT_REPLACE_EXTRA_EXCLUDES` is read as a boolean.
//!
//! # What it decides
//!
//! [`EnvKey::classify`] decides whether an environment variable is one of
//! the eight policy variables (production `layer_from_env_iter` matches the
//! result exhaustively and sets the corresponding layer field; enum values
//! go through the shared [`super::vocab`] parsers, so TOML and the
//! environment accept the same spellings). [`parse_bool_token`] decides the
//! boolean value of `WAFT_REPLACE_EXTRA_EXCLUDES`.
//!
//! # Theorems
//!
//! - `classify` equals `EnvKey::spec_classify` ("the key whose name is
//!   exactly `k`"); `theorem_env_key_table`: the table lists each key once,
//!   names are distinct and start with `WAFT_`, and classification is exact
//!   (case variants, prefixes and suffixes are ignored). `as_str` returns the
//!   name, so the error context production prints is the variable's name.
//! - `parse_bool_token`: `Some(true)` exactly for `1`, `true`, `yes`, `on`;
//!   `Some(false)` exactly for `0`, `false`, `no`, `off` and the empty
//!   string; `None` otherwise. `theorem_bool_tokens_disjoint`: no spelling is
//!   both.
//!
//! # Trust
//!
//! Production trims (`str::trim`, Unicode White_Space) and ASCII-lowercases
//! the raw value before `parse_bool_token`, and keeps the
//! `WAFT_EXTRA_EXCLUDE` comma split, trim and empty filter; the loop over
//! `std::env::vars()` (which panics on a non-UTF-8 variable anywhere in the
//! environment) and its last-occurrence-wins and first-error order also stay
//! in production.

#[allow(unused_imports)]
use vstd::prelude::*;

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

} // verus!
