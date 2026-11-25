//! Policy vocabulary: the exact strings each policy knob accepts and prints.
//!
//! Each enum mirrors a production enum in `src/config.rs` variant for
//! variant (production keeps its own copies because they derive
//! `clap::ValueEnum`, which cannot be implemented on this crate's types).
//! For every enum this module defines:
//!
//! - `name`: the canonical spelling (the specification);
//! - `all`: every variant in declaration order;
//! - `as_str`: proven to return `name`;
//! - `parse`: proven to equal `spec_parse`, the declarative "the unique
//!   variant whose spelling is `s`, if any" (for [`CopyStrategy`], "whose
//!   accepted spellings include `s`");
//! - `EXPECTED`: the `a|b|c` hint used in "unknown ..." error messages,
//!   proven equal to the canonical names of `all()` joined with `|`.
//!
//! `theorem_*_vocabulary` proves, per enum, that `all()` lists every variant
//! exactly once, that `name` is injective, that parsing is exact (accepted
//! strings are canonical names or the listed aliases: case-sensitive,
//! untrimmed, no prefixes), and the round trip `spec_parse(x.name()) ==
//! Some(x)`. Since `as_str` returns `name` and `parse` equals `spec_parse`,
//! `parse(x.as_str()) == Some(x)` holds for the shipped code.
//!
//! Trust: vstd's specification of `&str == &str` (view equality) and the
//! string-literal axioms; production's variant-for-variant conversions.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// Joins spellings with `|`, as the error-message hints list them.
pub open spec fn join_with_bar(names: Seq<Seq<char>>) -> Seq<char>
    decreases names.len(),
{
    if names.len() == 0 {
        Seq::empty()
    } else if names.len() == 1 {
        names[0]
    } else {
        join_with_bar(names.drop_last()) + seq!['|'] + names.last()
    }
}

// ---------------------------------------------------------------------------
// CompatProfile

/// Top-level compatibility preset (mirror of production `CompatProfile`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatProfile {
    /// Match Claude Code out-of-the-box behavior.
    Claude,
    /// Match Git per-directory exclude semantics.
    Git,
    /// Match worktrunk's observed behavior.
    Wt,
}

impl CompatProfile {
    /// The `|`-separated list of accepted spellings used in error messages.
    pub const EXPECTED: &'static str = "claude|git|wt";

    /// Every variant, in declaration order.
    pub open spec fn all() -> Seq<CompatProfile> {
        seq![CompatProfile::Claude, CompatProfile::Git, CompatProfile::Wt]
    }

    /// Canonical spelling.
    pub open spec fn name(self) -> Seq<char> {
        match self {
            CompatProfile::Claude => "claude"@,
            CompatProfile::Git => "git"@,
            CompatProfile::Wt => "wt"@,
        }
    }

    /// The variant spelled exactly `s`, if any.
    pub open spec fn spec_parse(s: Seq<char>) -> Option<CompatProfile> {
        if exists|p: CompatProfile| p.name() == s {
            Some(choose|p: CompatProfile| p.name() == s)
        } else {
            None
        }
    }

    /// Stable string identifier.
    pub fn as_str(self) -> (s: &'static str)
        ensures
            s@ == self.name(),
    {
        match self {
            CompatProfile::Claude => "claude",
            CompatProfile::Git => "git",
            CompatProfile::Wt => "wt",
        }
    }

    /// Parses a canonical spelling.
    pub fn parse(s: &str) -> (r: Option<CompatProfile>)
        ensures
            r == CompatProfile::spec_parse(s@),
    {
        proof {
            theorem_profile_vocabulary();
        }
        if s == "claude" {
            assert(s@ == CompatProfile::Claude.name());
            Some(CompatProfile::Claude)
        } else if s == "git" {
            assert(s@ == CompatProfile::Git.name());
            Some(CompatProfile::Git)
        } else if s == "wt" {
            assert(s@ == CompatProfile::Wt.name());
            Some(CompatProfile::Wt)
        } else {
            assert forall|p: CompatProfile| p.name() != s@ by {
                match p {
                    CompatProfile::Claude => {},
                    CompatProfile::Git => {},
                    CompatProfile::Wt => {},
                }
            }
            None
        }
    }
}

/// `CompatProfile` vocabulary facts (see the module documentation).
pub proof fn theorem_profile_vocabulary()
    ensures
        forall|p: CompatProfile| CompatProfile::all().contains(p),
        CompatProfile::all().no_duplicates(),
        forall|p: CompatProfile, q: CompatProfile| p.name() == q.name() ==> p == q,
        forall|p: CompatProfile| CompatProfile::spec_parse(#[trigger] p.name()) == Some(p),
        forall|s: Seq<char>|
            (#[trigger] CompatProfile::spec_parse(s)) matches Some(p) ==> p.name() == s,
        CompatProfile::EXPECTED@ == join_with_bar(
            CompatProfile::all().map_values(|p: CompatProfile| p.name()),
        ),
{
    reveal_strlit("claude");
    reveal_strlit("git");
    reveal_strlit("wt");
    reveal_strlit("claude|git|wt");
    let all = CompatProfile::all();
    assert(all[0] == CompatProfile::Claude);
    assert(all[1] == CompatProfile::Git);
    assert(all[2] == CompatProfile::Wt);
    assert forall|p: CompatProfile| all.contains(p) by {
        match p {
            CompatProfile::Claude => assert(all[0] == p),
            CompatProfile::Git => assert(all[1] == p),
            CompatProfile::Wt => assert(all[2] == p),
        }
    }
    assert forall|p: CompatProfile, q: CompatProfile| p.name() == q.name() implies p == q by {
        if p != q {
            assert(p.name()[0] != q.name()[0] || p.name().len() != q.name().len());
        }
    }
    let names = all.map_values(|p: CompatProfile| p.name());
    assert(names =~= seq!["claude"@, "git"@, "wt"@]);
    reveal_with_fuel(join_with_bar, 3);
    assert(join_with_bar(names) =~= "claude|git|wt"@);
}

// ---------------------------------------------------------------------------
// WhenMissingWorktreeinclude

/// Behavior when no `.worktreeinclude` file exists (mirror of production
/// `WhenMissingWorktreeinclude`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhenMissingWorktreeinclude {
    /// Select nothing.
    Blank,
    /// Treat all git-ignored paths as selected.
    AllIgnored,
}

impl WhenMissingWorktreeinclude {
    /// The `|`-separated list of accepted spellings used in error messages.
    pub const EXPECTED: &'static str = "blank|all-ignored";

    /// Every variant, in declaration order.
    pub open spec fn all() -> Seq<WhenMissingWorktreeinclude> {
        seq![WhenMissingWorktreeinclude::Blank, WhenMissingWorktreeinclude::AllIgnored]
    }

    /// Canonical spelling.
    pub open spec fn name(self) -> Seq<char> {
        match self {
            WhenMissingWorktreeinclude::Blank => "blank"@,
            WhenMissingWorktreeinclude::AllIgnored => "all-ignored"@,
        }
    }

    /// The variant spelled exactly `s`, if any.
    pub open spec fn spec_parse(s: Seq<char>) -> Option<WhenMissingWorktreeinclude> {
        if exists|x: WhenMissingWorktreeinclude| x.name() == s {
            Some(choose|x: WhenMissingWorktreeinclude| x.name() == s)
        } else {
            None
        }
    }

    /// Stable string identifier.
    pub fn as_str(self) -> (s: &'static str)
        ensures
            s@ == self.name(),
    {
        match self {
            WhenMissingWorktreeinclude::Blank => "blank",
            WhenMissingWorktreeinclude::AllIgnored => "all-ignored",
        }
    }

    /// Parses a canonical spelling.
    pub fn parse(s: &str) -> (r: Option<WhenMissingWorktreeinclude>)
        ensures
            r == WhenMissingWorktreeinclude::spec_parse(s@),
    {
        proof {
            theorem_when_missing_vocabulary();
        }
        if s == "blank" {
            assert(s@ == WhenMissingWorktreeinclude::Blank.name());
            Some(WhenMissingWorktreeinclude::Blank)
        } else if s == "all-ignored" {
            assert(s@ == WhenMissingWorktreeinclude::AllIgnored.name());
            Some(WhenMissingWorktreeinclude::AllIgnored)
        } else {
            assert forall|x: WhenMissingWorktreeinclude| x.name() != s@ by {
                match x {
                    WhenMissingWorktreeinclude::Blank => {},
                    WhenMissingWorktreeinclude::AllIgnored => {},
                }
            }
            None
        }
    }
}

/// `WhenMissingWorktreeinclude` vocabulary facts (see the module documentation).
pub proof fn theorem_when_missing_vocabulary()
    ensures
        forall|x: WhenMissingWorktreeinclude| WhenMissingWorktreeinclude::all().contains(x),
        WhenMissingWorktreeinclude::all().no_duplicates(),
        forall|x: WhenMissingWorktreeinclude, y: WhenMissingWorktreeinclude| x.name() == y.name() ==> x == y,
        forall|x: WhenMissingWorktreeinclude| WhenMissingWorktreeinclude::spec_parse(#[trigger] x.name()) == Some(x),
        forall|s: Seq<char>|
            (#[trigger] WhenMissingWorktreeinclude::spec_parse(s)) matches Some(x) ==> x.name() == s,
        WhenMissingWorktreeinclude::EXPECTED@ == join_with_bar(
            WhenMissingWorktreeinclude::all().map_values(|x: WhenMissingWorktreeinclude| x.name()),
        ),
{
    reveal_strlit("blank");
    reveal_strlit("all-ignored");
    reveal_strlit("blank|all-ignored");
    let all = WhenMissingWorktreeinclude::all();
    assert(all[0] == WhenMissingWorktreeinclude::Blank);
    assert(all[1] == WhenMissingWorktreeinclude::AllIgnored);
    assert forall|x: WhenMissingWorktreeinclude| all.contains(x) by {
        match x {
            WhenMissingWorktreeinclude::Blank => assert(all[0] == x),
            WhenMissingWorktreeinclude::AllIgnored => assert(all[1] == x),
        }
    }
    assert forall|x: WhenMissingWorktreeinclude, y: WhenMissingWorktreeinclude| x.name() == y.name() implies x == y by {
        if x != y {
            assert(x.name()[0] != y.name()[0]);
        }
    }
    let names = all.map_values(|x: WhenMissingWorktreeinclude| x.name());
    assert(names =~= seq!["blank"@, "all-ignored"@]);
    reveal_with_fuel(join_with_bar, 2);
    assert(join_with_bar(names) =~= "blank|all-ignored"@);
}

// ---------------------------------------------------------------------------
// WorktreeincludeSemantics

/// Matcher semantics for `.worktreeinclude` evaluation (mirror of production
/// `WorktreeincludeSemantics`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeincludeSemantics {
    /// Match observed Claude behavior as of 2026-04.
    Claude202604,
    /// Match Git per-directory exclude semantics.
    Git,
    /// Match observed worktrunk 0.39 behavior.
    Wt039,
}

impl WorktreeincludeSemantics {
    /// The `|`-separated list of accepted spellings used in error messages.
    pub const EXPECTED: &'static str = "claude-2026-04|git|wt-0.39";

    /// Every variant, in declaration order.
    pub open spec fn all() -> Seq<WorktreeincludeSemantics> {
        seq![WorktreeincludeSemantics::Claude202604, WorktreeincludeSemantics::Git, WorktreeincludeSemantics::Wt039]
    }

    /// Canonical spelling.
    pub open spec fn name(self) -> Seq<char> {
        match self {
            WorktreeincludeSemantics::Claude202604 => "claude-2026-04"@,
            WorktreeincludeSemantics::Git => "git"@,
            WorktreeincludeSemantics::Wt039 => "wt-0.39"@,
        }
    }

    /// The variant spelled exactly `s`, if any.
    pub open spec fn spec_parse(s: Seq<char>) -> Option<WorktreeincludeSemantics> {
        if exists|x: WorktreeincludeSemantics| x.name() == s {
            Some(choose|x: WorktreeincludeSemantics| x.name() == s)
        } else {
            None
        }
    }

    /// Stable string identifier.
    pub fn as_str(self) -> (s: &'static str)
        ensures
            s@ == self.name(),
    {
        match self {
            WorktreeincludeSemantics::Claude202604 => "claude-2026-04",
            WorktreeincludeSemantics::Git => "git",
            WorktreeincludeSemantics::Wt039 => "wt-0.39",
        }
    }

    /// Parses a canonical spelling.
    pub fn parse(s: &str) -> (r: Option<WorktreeincludeSemantics>)
        ensures
            r == WorktreeincludeSemantics::spec_parse(s@),
    {
        proof {
            theorem_semantics_vocabulary();
        }
        if s == "claude-2026-04" {
            assert(s@ == WorktreeincludeSemantics::Claude202604.name());
            Some(WorktreeincludeSemantics::Claude202604)
        } else if s == "git" {
            assert(s@ == WorktreeincludeSemantics::Git.name());
            Some(WorktreeincludeSemantics::Git)
        } else if s == "wt-0.39" {
            assert(s@ == WorktreeincludeSemantics::Wt039.name());
            Some(WorktreeincludeSemantics::Wt039)
        } else {
            assert forall|x: WorktreeincludeSemantics| x.name() != s@ by {
                match x {
                    WorktreeincludeSemantics::Claude202604 => {},
                    WorktreeincludeSemantics::Git => {},
                    WorktreeincludeSemantics::Wt039 => {},
                }
            }
            None
        }
    }
}

/// `WorktreeincludeSemantics` vocabulary facts (see the module documentation).
pub proof fn theorem_semantics_vocabulary()
    ensures
        forall|x: WorktreeincludeSemantics| WorktreeincludeSemantics::all().contains(x),
        WorktreeincludeSemantics::all().no_duplicates(),
        forall|x: WorktreeincludeSemantics, y: WorktreeincludeSemantics| x.name() == y.name() ==> x == y,
        forall|x: WorktreeincludeSemantics| WorktreeincludeSemantics::spec_parse(#[trigger] x.name()) == Some(x),
        forall|s: Seq<char>|
            (#[trigger] WorktreeincludeSemantics::spec_parse(s)) matches Some(x) ==> x.name() == s,
        WorktreeincludeSemantics::EXPECTED@ == join_with_bar(
            WorktreeincludeSemantics::all().map_values(|x: WorktreeincludeSemantics| x.name()),
        ),
{
    reveal_strlit("claude-2026-04");
    reveal_strlit("git");
    reveal_strlit("wt-0.39");
    reveal_strlit("claude-2026-04|git|wt-0.39");
    let all = WorktreeincludeSemantics::all();
    assert(all[0] == WorktreeincludeSemantics::Claude202604);
    assert(all[1] == WorktreeincludeSemantics::Git);
    assert(all[2] == WorktreeincludeSemantics::Wt039);
    assert forall|x: WorktreeincludeSemantics| all.contains(x) by {
        match x {
            WorktreeincludeSemantics::Claude202604 => assert(all[0] == x),
            WorktreeincludeSemantics::Git => assert(all[1] == x),
            WorktreeincludeSemantics::Wt039 => assert(all[2] == x),
        }
    }
    assert forall|x: WorktreeincludeSemantics, y: WorktreeincludeSemantics| x.name() == y.name() implies x == y by {
        if x != y {
            assert(x.name()[0] != y.name()[0]);
        }
    }
    let names = all.map_values(|x: WorktreeincludeSemantics| x.name());
    assert(names =~= seq!["claude-2026-04"@, "git"@, "wt-0.39"@]);
    reveal_with_fuel(join_with_bar, 3);
    assert(join_with_bar(names) =~= "claude-2026-04|git|wt-0.39"@);
}

// ---------------------------------------------------------------------------
// SymlinkPolicy

/// Policy for symlinked `.worktreeinclude` files (mirror of production
/// `SymlinkPolicy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymlinkPolicy {
    /// Follow the symlink target as a rule file.
    Follow,
    /// Ignore symlinked rule files.
    Ignore,
    /// Fail on a symlinked rule file.
    Error,
}

impl SymlinkPolicy {
    /// The `|`-separated list of accepted spellings used in error messages.
    pub const EXPECTED: &'static str = "follow|ignore|error";

    /// Every variant, in declaration order.
    pub open spec fn all() -> Seq<SymlinkPolicy> {
        seq![SymlinkPolicy::Follow, SymlinkPolicy::Ignore, SymlinkPolicy::Error]
    }

    /// Canonical spelling.
    pub open spec fn name(self) -> Seq<char> {
        match self {
            SymlinkPolicy::Follow => "follow"@,
            SymlinkPolicy::Ignore => "ignore"@,
            SymlinkPolicy::Error => "error"@,
        }
    }

    /// The variant spelled exactly `s`, if any.
    pub open spec fn spec_parse(s: Seq<char>) -> Option<SymlinkPolicy> {
        if exists|x: SymlinkPolicy| x.name() == s {
            Some(choose|x: SymlinkPolicy| x.name() == s)
        } else {
            None
        }
    }

    /// Stable string identifier.
    pub fn as_str(self) -> (s: &'static str)
        ensures
            s@ == self.name(),
    {
        match self {
            SymlinkPolicy::Follow => "follow",
            SymlinkPolicy::Ignore => "ignore",
            SymlinkPolicy::Error => "error",
        }
    }

    /// Parses a canonical spelling.
    pub fn parse(s: &str) -> (r: Option<SymlinkPolicy>)
        ensures
            r == SymlinkPolicy::spec_parse(s@),
    {
        proof {
            theorem_symlink_vocabulary();
        }
        if s == "follow" {
            assert(s@ == SymlinkPolicy::Follow.name());
            Some(SymlinkPolicy::Follow)
        } else if s == "ignore" {
            assert(s@ == SymlinkPolicy::Ignore.name());
            Some(SymlinkPolicy::Ignore)
        } else if s == "error" {
            assert(s@ == SymlinkPolicy::Error.name());
            Some(SymlinkPolicy::Error)
        } else {
            assert forall|x: SymlinkPolicy| x.name() != s@ by {
                match x {
                    SymlinkPolicy::Follow => {},
                    SymlinkPolicy::Ignore => {},
                    SymlinkPolicy::Error => {},
                }
            }
            None
        }
    }
}

/// `SymlinkPolicy` vocabulary facts (see the module documentation).
pub proof fn theorem_symlink_vocabulary()
    ensures
        forall|x: SymlinkPolicy| SymlinkPolicy::all().contains(x),
        SymlinkPolicy::all().no_duplicates(),
        forall|x: SymlinkPolicy, y: SymlinkPolicy| x.name() == y.name() ==> x == y,
        forall|x: SymlinkPolicy| SymlinkPolicy::spec_parse(#[trigger] x.name()) == Some(x),
        forall|s: Seq<char>|
            (#[trigger] SymlinkPolicy::spec_parse(s)) matches Some(x) ==> x.name() == s,
        SymlinkPolicy::EXPECTED@ == join_with_bar(
            SymlinkPolicy::all().map_values(|x: SymlinkPolicy| x.name()),
        ),
{
    reveal_strlit("follow");
    reveal_strlit("ignore");
    reveal_strlit("error");
    reveal_strlit("follow|ignore|error");
    let all = SymlinkPolicy::all();
    assert(all[0] == SymlinkPolicy::Follow);
    assert(all[1] == SymlinkPolicy::Ignore);
    assert(all[2] == SymlinkPolicy::Error);
    assert forall|x: SymlinkPolicy| all.contains(x) by {
        match x {
            SymlinkPolicy::Follow => assert(all[0] == x),
            SymlinkPolicy::Ignore => assert(all[1] == x),
            SymlinkPolicy::Error => assert(all[2] == x),
        }
    }
    assert forall|x: SymlinkPolicy, y: SymlinkPolicy| x.name() == y.name() implies x == y by {
        if x != y {
            assert(x.name()[0] != y.name()[0]);
        }
    }
    let names = all.map_values(|x: SymlinkPolicy| x.name());
    assert(names =~= seq!["follow"@, "ignore"@, "error"@]);
    reveal_with_fuel(join_with_bar, 3);
    assert(join_with_bar(names) =~= "follow|ignore|error"@);
}

// ---------------------------------------------------------------------------
// BuiltinExcludeSet

/// Built-in exclusion set selector (mirror of production `BuiltinExcludeSet`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinExcludeSet {
    /// No built-in excludes.
    None,
    /// The curated `tooling-v1` exclusion list.
    ToolingV1,
}

impl BuiltinExcludeSet {
    /// The `|`-separated list of accepted spellings used in error messages.
    pub const EXPECTED: &'static str = "none|tooling-v1";

    /// Every variant, in declaration order.
    pub open spec fn all() -> Seq<BuiltinExcludeSet> {
        seq![BuiltinExcludeSet::None, BuiltinExcludeSet::ToolingV1]
    }

    /// Canonical spelling.
    pub open spec fn name(self) -> Seq<char> {
        match self {
            BuiltinExcludeSet::None => "none"@,
            BuiltinExcludeSet::ToolingV1 => "tooling-v1"@,
        }
    }

    /// The variant spelled exactly `s`, if any.
    pub open spec fn spec_parse(s: Seq<char>) -> Option<BuiltinExcludeSet> {
        if exists|x: BuiltinExcludeSet| x.name() == s {
            Some(choose|x: BuiltinExcludeSet| x.name() == s)
        } else {
            None
        }
    }

    /// Stable string identifier.
    pub fn as_str(self) -> (s: &'static str)
        ensures
            s@ == self.name(),
    {
        match self {
            BuiltinExcludeSet::None => "none",
            BuiltinExcludeSet::ToolingV1 => "tooling-v1",
        }
    }

    /// Parses a canonical spelling.
    pub fn parse(s: &str) -> (r: Option<BuiltinExcludeSet>)
        ensures
            r == BuiltinExcludeSet::spec_parse(s@),
    {
        proof {
            theorem_builtin_set_vocabulary();
        }
        if s == "none" {
            assert(s@ == BuiltinExcludeSet::None.name());
            Some(BuiltinExcludeSet::None)
        } else if s == "tooling-v1" {
            assert(s@ == BuiltinExcludeSet::ToolingV1.name());
            Some(BuiltinExcludeSet::ToolingV1)
        } else {
            assert forall|x: BuiltinExcludeSet| x.name() != s@ by {
                match x {
                    BuiltinExcludeSet::None => {},
                    BuiltinExcludeSet::ToolingV1 => {},
                }
            }
            None
        }
    }
}

/// `BuiltinExcludeSet` vocabulary facts (see the module documentation).
pub proof fn theorem_builtin_set_vocabulary()
    ensures
        forall|x: BuiltinExcludeSet| BuiltinExcludeSet::all().contains(x),
        BuiltinExcludeSet::all().no_duplicates(),
        forall|x: BuiltinExcludeSet, y: BuiltinExcludeSet| x.name() == y.name() ==> x == y,
        forall|x: BuiltinExcludeSet| BuiltinExcludeSet::spec_parse(#[trigger] x.name()) == Some(x),
        forall|s: Seq<char>|
            (#[trigger] BuiltinExcludeSet::spec_parse(s)) matches Some(x) ==> x.name() == s,
        BuiltinExcludeSet::EXPECTED@ == join_with_bar(
            BuiltinExcludeSet::all().map_values(|x: BuiltinExcludeSet| x.name()),
        ),
{
    reveal_strlit("none");
    reveal_strlit("tooling-v1");
    reveal_strlit("none|tooling-v1");
    let all = BuiltinExcludeSet::all();
    assert(all[0] == BuiltinExcludeSet::None);
    assert(all[1] == BuiltinExcludeSet::ToolingV1);
    assert forall|x: BuiltinExcludeSet| all.contains(x) by {
        match x {
            BuiltinExcludeSet::None => assert(all[0] == x),
            BuiltinExcludeSet::ToolingV1 => assert(all[1] == x),
        }
    }
    assert forall|x: BuiltinExcludeSet, y: BuiltinExcludeSet| x.name() == y.name() implies x == y by {
        if x != y {
            assert(x.name()[0] != y.name()[0]);
        }
    }
    let names = all.map_values(|x: BuiltinExcludeSet| x.name());
    assert(names =~= seq!["none"@, "tooling-v1"@]);
    reveal_with_fuel(join_with_bar, 2);
    assert(join_with_bar(names) =~= "none|tooling-v1"@);
}

// ---------------------------------------------------------------------------
// CopyStrategy

/// Strategy for placing copied file content (mirror of production
/// `CopyStrategy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyStrategy {
    /// The platform default.
    Auto,
    /// Always a plain byte-for-byte copy.
    SimpleCopy,
    /// Attempt a reflink (copy-on-write) copy, falling back to a plain copy.
    CowCopy,
}

impl CopyStrategy {
    /// The `|`-separated list of canonical spellings used in error messages
    /// (aliases are accepted but not advertised).
    pub const EXPECTED: &'static str = "auto|simple-copy|cow-copy";

    /// Every variant, in declaration order.
    pub open spec fn all() -> Seq<CopyStrategy> {
        seq![CopyStrategy::Auto, CopyStrategy::SimpleCopy, CopyStrategy::CowCopy]
    }

    /// Canonical spelling.
    pub open spec fn name(self) -> Seq<char> {
        match self {
            CopyStrategy::Auto => "auto"@,
            CopyStrategy::SimpleCopy => "simple-copy"@,
            CopyStrategy::CowCopy => "cow-copy"@,
        }
    }

    /// Additional accepted spellings (TOML and `WAFT_COPY_STRATEGY` only;
    /// the CLI flag uses clap's derived names).
    pub open spec fn aliases(self) -> Seq<Seq<char>> {
        match self {
            CopyStrategy::Auto => seq!["default"@],
            CopyStrategy::SimpleCopy => seq!["simple"@],
            CopyStrategy::CowCopy => seq!["cow"@, "reflink"@],
        }
    }

    /// Whether `s` is one of this variant's spellings.
    pub open spec fn accepts(self, s: Seq<char>) -> bool {
        s == self.name() || self.aliases().contains(s)
    }

    /// The variant that accepts `s`, if any.
    pub open spec fn spec_parse(s: Seq<char>) -> Option<CopyStrategy> {
        if exists|x: CopyStrategy| x.accepts(s) {
            Some(choose|x: CopyStrategy| x.accepts(s))
        } else {
            None
        }
    }

    /// Stable string identifier.
    pub fn as_str(self) -> (s: &'static str)
        ensures
            s@ == self.name(),
    {
        match self {
            CopyStrategy::Auto => "auto",
            CopyStrategy::SimpleCopy => "simple-copy",
            CopyStrategy::CowCopy => "cow-copy",
        }
    }

    /// Parses a canonical spelling or alias.
    pub fn parse(s: &str) -> (r: Option<CopyStrategy>)
        ensures
            r == CopyStrategy::spec_parse(s@),
    {
        proof {
            theorem_copy_strategy_vocabulary();
            reveal_strlit("default");
            reveal_strlit("simple");
            reveal_strlit("cow");
            reveal_strlit("reflink");
        }
        if s == "auto" || s == "default" {
            assert(CopyStrategy::Auto.aliases()[0] == "default"@);
            assert(CopyStrategy::Auto.accepts(s@));
            Some(CopyStrategy::Auto)
        } else if s == "simple-copy" || s == "simple" {
            assert(CopyStrategy::SimpleCopy.aliases()[0] == "simple"@);
            assert(CopyStrategy::SimpleCopy.accepts(s@));
            Some(CopyStrategy::SimpleCopy)
        } else if s == "cow-copy" || s == "cow" || s == "reflink" {
            assert(CopyStrategy::CowCopy.aliases()[0] == "cow"@);
            assert(CopyStrategy::CowCopy.aliases()[1] == "reflink"@);
            assert(CopyStrategy::CowCopy.accepts(s@));
            Some(CopyStrategy::CowCopy)
        } else {
            assert forall|x: CopyStrategy| !x.accepts(s@) by {
                match x {
                    CopyStrategy::Auto => {},
                    CopyStrategy::SimpleCopy => {},
                    CopyStrategy::CowCopy => {
                        assert(x.aliases() =~= seq!["cow"@, "reflink"@]);
                    },
                }
            }
            None
        }
    }
}

/// `CopyStrategy` vocabulary facts: as for the other enums, plus the accepted
/// spellings of distinct variants are disjoint, so every accepted string
/// (canonical or alias) identifies exactly one variant.
pub proof fn theorem_copy_strategy_vocabulary()
    ensures
        forall|x: CopyStrategy| CopyStrategy::all().contains(x),
        CopyStrategy::all().no_duplicates(),
        forall|x: CopyStrategy, y: CopyStrategy| x.name() == y.name() ==> x == y,
        forall|x: CopyStrategy, y: CopyStrategy, s: Seq<char>|
            #[trigger] x.accepts(s) && #[trigger] y.accepts(s) ==> x == y,
        forall|x: CopyStrategy| CopyStrategy::spec_parse(#[trigger] x.name()) == Some(x),
        forall|x: CopyStrategy, s: Seq<char>|
            #[trigger] x.aliases().contains(s) ==> CopyStrategy::spec_parse(s) == Some(x),
        forall|s: Seq<char>|
            (#[trigger] CopyStrategy::spec_parse(s)) matches Some(x) ==> x.accepts(s),
        CopyStrategy::EXPECTED@ == join_with_bar(
            CopyStrategy::all().map_values(|x: CopyStrategy| x.name()),
        ),
{
    reveal_strlit("auto");
    reveal_strlit("simple-copy");
    reveal_strlit("cow-copy");
    reveal_strlit("default");
    reveal_strlit("simple");
    reveal_strlit("cow");
    reveal_strlit("reflink");
    reveal_strlit("auto|simple-copy|cow-copy");
    let all = CopyStrategy::all();
    assert(all[0] == CopyStrategy::Auto);
    assert(all[1] == CopyStrategy::SimpleCopy);
    assert(all[2] == CopyStrategy::CowCopy);
    assert forall|x: CopyStrategy| all.contains(x) by {
        match x {
            CopyStrategy::Auto => assert(all[0] == x),
            CopyStrategy::SimpleCopy => assert(all[1] == x),
            CopyStrategy::CowCopy => assert(all[2] == x),
        }
    }
    // Every spelling of a variant starts with a character no spelling of
    // another variant starts with.
    assert forall|x: CopyStrategy, s: Seq<char>| #[trigger] x.accepts(s) implies s.len() > 0 && (
    match x {
        CopyStrategy::Auto => s[0] == 'a' || s[0] == 'd',
        CopyStrategy::SimpleCopy => s[0] == 's',
        CopyStrategy::CowCopy => s[0] == 'c' || s[0] == 'r',
    }) by {
        if x.aliases().contains(s) {
            let i = choose|i: int| 0 <= i < x.aliases().len() && x.aliases()[i] == s;
            if x is CowCopy {
                assert(i == 0 || i == 1);
            }
        }
    }
    assert forall|x: CopyStrategy, y: CopyStrategy, s: Seq<char>|
        #[trigger] x.accepts(s) && #[trigger] y.accepts(s) implies x == y by {}
    assert forall|x: CopyStrategy| x.accepts(#[trigger] x.name()) by {}
    assert forall|x: CopyStrategy, y: CopyStrategy| x.name() == y.name() implies x == y by {
        assert(x.accepts(x.name()));
        assert(y.accepts(y.name()));
    }
    assert forall|x: CopyStrategy, s: Seq<char>| #[trigger] x.aliases().contains(s) implies
        CopyStrategy::spec_parse(s) == Some(x) by {
        assert(x.accepts(s));
    }
    let names = all.map_values(|x: CopyStrategy| x.name());
    assert(names =~= seq!["auto"@, "simple-copy"@, "cow-copy"@]);
    reveal_with_fuel(join_with_bar, 3);
    assert(join_with_bar(names) =~= "auto|simple-copy|cow-copy"@);
}

/// The alias table, stated literally: `default` is `Auto`, `simple` is
/// `SimpleCopy`, and `cow` and `reflink` are `CowCopy`.
pub proof fn theorem_copy_strategy_aliases()
    ensures
        CopyStrategy::spec_parse("default"@) == Some(CopyStrategy::Auto),
        CopyStrategy::spec_parse("simple"@) == Some(CopyStrategy::SimpleCopy),
        CopyStrategy::spec_parse("cow"@) == Some(CopyStrategy::CowCopy),
        CopyStrategy::spec_parse("reflink"@) == Some(CopyStrategy::CowCopy),
{
    theorem_copy_strategy_vocabulary();
    assert(CopyStrategy::Auto.aliases().contains("default"@)) by {
        assert(CopyStrategy::Auto.aliases()[0] == "default"@);
    }
    assert(CopyStrategy::SimpleCopy.aliases().contains("simple"@)) by {
        assert(CopyStrategy::SimpleCopy.aliases()[0] == "simple"@);
    }
    assert(CopyStrategy::CowCopy.aliases().contains("cow"@)) by {
        assert(CopyStrategy::CowCopy.aliases()[0] == "cow"@);
    }
    assert(CopyStrategy::CowCopy.aliases().contains("reflink"@)) by {
        assert(CopyStrategy::CowCopy.aliases()[1] == "reflink"@);
    }
}
} // verus!
