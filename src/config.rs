//! Configuration model, discovery, and policy resolution.
//!
//! Layers (lowest precedence first, last writer wins for scalars):
//!
//! 1. Built-in defaults
//! 2. User config (`~/.config/waft/config.toml`)
//! 3. Project configs (`.waft.toml` from repo root to cwd; deeper wins)
//! 4. Environment variables (`WAFT_*`)
//! 5. CLI flags
//!
//! Array keys (currently `exclude.extra`) append across layers unless the
//! same layer sets `replace_extra = true`, in which case that layer's value
//! replaces accumulated values from lower-precedence layers.
//!
//! Project configuration discovery is bounded to the resolved source
//! repository. `--isolated` omits the user and environment policy layers for
//! managed, reproducible invocation.
//!
//! The decisions in this module (the accepted spellings, presets, layer
//! merge and order, config-file acceptance, the environment key table and
//! boolean tokens, the user config base variable, and which directories
//! discovery searches) are made by the Verus-verified kernels in
//! `waft_verified::policy`; this module converts its types to the kernels'
//! and performs the I/O and error formatting around them.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use waft_verified::policy::{discovery, env_layer, merge, raw, user_source, vocab};

use crate::error::{Error, Result};

// --- Public enums for each knob ---

/// Top-level compatibility preset.
///
/// Selects a coordinated set of defaults tuned to match a particular tool's
/// observed behavior. The preset is expanded into concrete knob values at the
/// layer where it is selected; explicit knobs in that layer or a later layer
/// take precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, clap::ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum CompatProfile {
    /// Match Claude Code out-of-the-box behavior.
    Claude,
    /// Match Git per-directory exclude semantics.
    Git,
    /// Match worktrunk's observed behavior (`wt-0.39` snapshot).
    Wt,
}

impl CompatProfile {
    /// Stable string identifier for this profile.
    pub fn as_str(self) -> &'static str {
        vocab::CompatProfile::from(self).as_str()
    }

    /// Whether this profile's preset selects files with no `.worktreeinclude`
    /// present.
    ///
    /// Only the preset is consulted, not the resolved policy: a later layer
    /// can set `when_missing = blank` on top of any profile. That distinction
    /// is what the empty-selection note needs, because naming a profile as the
    /// reason a selection came back empty is only fair when the profile itself
    /// is what blanks it. The verified kernel proves this holds exactly for
    /// `wt`, and that a resolved `blank` under such a profile was set
    /// explicitly by a layer at or after the last profile selection
    /// (`policy::merge::theorem_attribution`).
    pub fn selects_without_rule_file(self) -> bool {
        merge::selects_without_rule_file(self.into())
    }

    fn parse(s: &str) -> std::result::Result<Self, String> {
        vocab::CompatProfile::parse(s)
            .map(Self::from)
            .ok_or_else(|| Self::unknown_value(s))
    }

    fn unknown_value(s: &str) -> String {
        format!(
            "unknown compat profile {s:?}; expected {}",
            vocab::CompatProfile::EXPECTED
        )
    }
}

impl From<CompatProfile> for vocab::CompatProfile {
    fn from(value: CompatProfile) -> Self {
        match value {
            CompatProfile::Claude => Self::Claude,
            CompatProfile::Git => Self::Git,
            CompatProfile::Wt => Self::Wt,
        }
    }
}

impl From<vocab::CompatProfile> for CompatProfile {
    fn from(value: vocab::CompatProfile) -> Self {
        match value {
            vocab::CompatProfile::Claude => Self::Claude,
            vocab::CompatProfile::Git => Self::Git,
            vocab::CompatProfile::Wt => Self::Wt,
        }
    }
}

/// Behavior when no `.worktreeinclude` file exists anywhere relevant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, clap::ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum WhenMissingWorktreeinclude {
    /// Select nothing.
    Blank,
    /// Treat all git-ignored paths as selected.
    AllIgnored,
}

impl WhenMissingWorktreeinclude {
    /// Stable string identifier.
    pub fn as_str(self) -> &'static str {
        vocab::WhenMissingWorktreeinclude::from(self).as_str()
    }

    fn parse(s: &str) -> std::result::Result<Self, String> {
        vocab::WhenMissingWorktreeinclude::parse(s)
            .map(Self::from)
            .ok_or_else(|| Self::unknown_value(s))
    }

    fn unknown_value(s: &str) -> String {
        format!(
            "unknown when_missing_worktreeinclude {s:?}; expected {}",
            vocab::WhenMissingWorktreeinclude::EXPECTED
        )
    }
}

impl From<WhenMissingWorktreeinclude> for vocab::WhenMissingWorktreeinclude {
    fn from(value: WhenMissingWorktreeinclude) -> Self {
        match value {
            WhenMissingWorktreeinclude::Blank => Self::Blank,
            WhenMissingWorktreeinclude::AllIgnored => Self::AllIgnored,
        }
    }
}

impl From<vocab::WhenMissingWorktreeinclude> for WhenMissingWorktreeinclude {
    fn from(value: vocab::WhenMissingWorktreeinclude) -> Self {
        match value {
            vocab::WhenMissingWorktreeinclude::Blank => Self::Blank,
            vocab::WhenMissingWorktreeinclude::AllIgnored => Self::AllIgnored,
        }
    }
}

/// Matcher semantics profile for `.worktreeinclude` evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, clap::ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum WorktreeincludeSemantics {
    /// Match observed Claude behavior as of 2026-04.
    #[clap(name = "claude-2026-04")]
    Claude202604,
    /// Match Git per-directory exclude semantics.
    Git,
    /// Match observed worktrunk 0.39 behavior.
    #[clap(name = "wt-0.39")]
    Wt039,
}

impl WorktreeincludeSemantics {
    /// Stable string identifier.
    pub fn as_str(self) -> &'static str {
        vocab::WorktreeincludeSemantics::from(self).as_str()
    }

    fn parse(s: &str) -> std::result::Result<Self, String> {
        vocab::WorktreeincludeSemantics::parse(s)
            .map(Self::from)
            .ok_or_else(|| Self::unknown_value(s))
    }

    fn unknown_value(s: &str) -> String {
        format!(
            "unknown worktreeinclude semantics {s:?}; expected {}",
            vocab::WorktreeincludeSemantics::EXPECTED
        )
    }
}

impl From<WorktreeincludeSemantics> for vocab::WorktreeincludeSemantics {
    fn from(value: WorktreeincludeSemantics) -> Self {
        match value {
            WorktreeincludeSemantics::Claude202604 => Self::Claude202604,
            WorktreeincludeSemantics::Git => Self::Git,
            WorktreeincludeSemantics::Wt039 => Self::Wt039,
        }
    }
}

impl From<vocab::WorktreeincludeSemantics> for WorktreeincludeSemantics {
    fn from(value: vocab::WorktreeincludeSemantics) -> Self {
        match value {
            vocab::WorktreeincludeSemantics::Claude202604 => Self::Claude202604,
            vocab::WorktreeincludeSemantics::Git => Self::Git,
            vocab::WorktreeincludeSemantics::Wt039 => Self::Wt039,
        }
    }
}

/// Policy for handling symlinked `.worktreeinclude` files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, clap::ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum SymlinkPolicy {
    /// Follow the symlink target as a rule file.
    Follow,
    /// Ignore symlinked rule files (treat as if absent).
    Ignore,
    /// Fail validation/copy on encountering a symlinked rule file.
    Error,
}

impl SymlinkPolicy {
    /// Stable string identifier.
    pub fn as_str(self) -> &'static str {
        vocab::SymlinkPolicy::from(self).as_str()
    }

    fn parse(s: &str) -> std::result::Result<Self, String> {
        vocab::SymlinkPolicy::parse(s)
            .map(Self::from)
            .ok_or_else(|| Self::unknown_value(s))
    }

    fn unknown_value(s: &str) -> String {
        format!(
            "unknown symlink policy {s:?}; expected {}",
            vocab::SymlinkPolicy::EXPECTED
        )
    }
}

impl From<SymlinkPolicy> for vocab::SymlinkPolicy {
    fn from(value: SymlinkPolicy) -> Self {
        match value {
            SymlinkPolicy::Follow => Self::Follow,
            SymlinkPolicy::Ignore => Self::Ignore,
            SymlinkPolicy::Error => Self::Error,
        }
    }
}

impl From<vocab::SymlinkPolicy> for SymlinkPolicy {
    fn from(value: vocab::SymlinkPolicy) -> Self {
        match value {
            vocab::SymlinkPolicy::Follow => Self::Follow,
            vocab::SymlinkPolicy::Ignore => Self::Ignore,
            vocab::SymlinkPolicy::Error => Self::Error,
        }
    }
}

/// Built-in exclusion set selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, clap::ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum BuiltinExcludeSet {
    /// No built-in excludes.
    None,
    /// Apply the curated `tooling-v1` exclusion list.
    #[clap(name = "tooling-v1")]
    ToolingV1,
}

impl BuiltinExcludeSet {
    /// Stable string identifier.
    pub fn as_str(self) -> &'static str {
        vocab::BuiltinExcludeSet::from(self).as_str()
    }

    fn parse(s: &str) -> std::result::Result<Self, String> {
        vocab::BuiltinExcludeSet::parse(s)
            .map(Self::from)
            .ok_or_else(|| Self::unknown_value(s))
    }

    fn unknown_value(s: &str) -> String {
        format!(
            "unknown builtin exclude set {s:?}; expected {}",
            vocab::BuiltinExcludeSet::EXPECTED
        )
    }
}

impl From<BuiltinExcludeSet> for vocab::BuiltinExcludeSet {
    fn from(value: BuiltinExcludeSet) -> Self {
        match value {
            BuiltinExcludeSet::None => Self::None,
            BuiltinExcludeSet::ToolingV1 => Self::ToolingV1,
        }
    }
}

impl From<vocab::BuiltinExcludeSet> for BuiltinExcludeSet {
    fn from(value: vocab::BuiltinExcludeSet) -> Self {
        match value {
            vocab::BuiltinExcludeSet::None => Self::None,
            vocab::BuiltinExcludeSet::ToolingV1 => Self::ToolingV1,
        }
    }
}

/// Strategy for placing copied file content at the destination.
///
/// Reflink (a.k.a. copy-on-write) is supported on APFS (macOS), Btrfs/XFS
/// (Linux), and ReFS (Windows Server). When supported, the new file shares
/// on-disk extents with the source until either side is modified, making the
/// copy near-instant and storage-cheap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, clap::ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum CopyStrategy {
    /// Use the platform default: attempt reflink on macOS, plain copy elsewhere.
    Auto,
    /// Always perform a plain byte-for-byte copy, even where reflinks are available.
    SimpleCopy,
    /// Attempt a reflink (COW) copy; fall back to a plain copy if unsupported.
    CowCopy,
}

impl CopyStrategy {
    /// Stable string identifier.
    pub fn as_str(self) -> &'static str {
        vocab::CopyStrategy::from(self).as_str()
    }

    /// Parses a canonical name or one of the aliases `default`, `simple`,
    /// `cow` and `reflink` (TOML and `WAFT_COPY_STRATEGY` only).
    fn parse(s: &str) -> std::result::Result<Self, String> {
        vocab::CopyStrategy::parse(s)
            .map(Self::from)
            .ok_or_else(|| Self::unknown_value(s))
    }

    fn unknown_value(s: &str) -> String {
        format!(
            "unknown copy strategy {s:?}; expected {}",
            vocab::CopyStrategy::EXPECTED
        )
    }
}

impl From<CopyStrategy> for vocab::CopyStrategy {
    fn from(value: CopyStrategy) -> Self {
        match value {
            CopyStrategy::Auto => Self::Auto,
            CopyStrategy::SimpleCopy => Self::SimpleCopy,
            CopyStrategy::CowCopy => Self::CowCopy,
        }
    }
}

impl From<vocab::CopyStrategy> for CopyStrategy {
    fn from(value: vocab::CopyStrategy) -> Self {
        match value {
            vocab::CopyStrategy::Auto => Self::Auto,
            vocab::CopyStrategy::SimpleCopy => Self::SimpleCopy,
            vocab::CopyStrategy::CowCopy => Self::CowCopy,
        }
    }
}

// --- Layered configuration ---

/// One layer of configuration with all keys optional.
///
/// Each source (defaults, user file, project files, env, CLI) produces a
/// `ConfigLayer`. Layers are merged in precedence order to produce a
/// [`ResolvedPolicy`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigLayer {
    /// Compat profile preset.
    pub profile: Option<CompatProfile>,
    /// Behavior when no `.worktreeinclude` file exists.
    pub when_missing: Option<WhenMissingWorktreeinclude>,
    /// Matcher semantics.
    pub semantics: Option<WorktreeincludeSemantics>,
    /// Symlink handling policy.
    pub symlink_policy: Option<SymlinkPolicy>,
    /// Built-in exclude set selector.
    pub builtin_exclude_set: Option<BuiltinExcludeSet>,
    /// Extra exclude globs introduced by this layer.
    pub extra_excludes: Vec<String>,
    /// Whether this layer's `extra_excludes` should replace accumulated values.
    pub replace_extra_excludes: Option<bool>,
    /// Strategy for placing file content at the destination during copy.
    pub copy_strategy: Option<CopyStrategy>,
}

/// Fully resolved policy after merging all layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPolicy {
    /// Selected compat profile.
    pub profile: CompatProfile,
    /// Behavior when no `.worktreeinclude` file exists.
    pub when_missing: WhenMissingWorktreeinclude,
    /// Matcher semantics.
    pub semantics: WorktreeincludeSemantics,
    /// Symlink handling policy.
    pub symlink_policy: SymlinkPolicy,
    /// Built-in exclude set selector.
    pub builtin_exclude_set: BuiltinExcludeSet,
    /// Extra exclude globs after layer merging.
    pub extra_excludes: Vec<String>,
    /// Strategy for placing file content at the destination.
    pub copy_strategy: CopyStrategy,
}

impl ConfigLayer {
    /// Lower this layer to the verified merge kernel's view of it.
    ///
    /// The destructuring is exhaustive so a new field fails to compile until
    /// it is mapped.
    fn kernel_view(&self) -> merge::LayerView<'_> {
        let Self {
            profile,
            when_missing,
            semantics,
            symlink_policy,
            builtin_exclude_set,
            extra_excludes,
            replace_extra_excludes,
            copy_strategy,
        } = self;
        merge::LayerView {
            knobs: merge::KnobLayer {
                profile: profile.map(Into::into),
                when_missing: when_missing.map(Into::into),
                semantics: semantics.map(Into::into),
                symlink_policy: symlink_policy.map(Into::into),
                builtin_exclude_set: builtin_exclude_set.map(Into::into),
                copy_strategy: copy_strategy.map(Into::into),
                replace_extra: *replace_extra_excludes,
            },
            extras: extra_excludes,
        }
    }

    /// Lift kernel knobs plus this layer's extra excludes to a layer
    /// (exhaustive).
    fn from_kernel(knobs: merge::KnobLayer, extra_excludes: Vec<String>) -> Self {
        let merge::KnobLayer {
            profile,
            when_missing,
            semantics,
            symlink_policy,
            builtin_exclude_set,
            copy_strategy,
            replace_extra,
        } = knobs;
        Self {
            profile: profile.map(Into::into),
            when_missing: when_missing.map(Into::into),
            semantics: semantics.map(Into::into),
            symlink_policy: symlink_policy.map(Into::into),
            builtin_exclude_set: builtin_exclude_set.map(Into::into),
            extra_excludes,
            replace_extra_excludes: replace_extra,
            copy_strategy: copy_strategy.map(Into::into),
        }
    }
}

impl Default for ResolvedPolicy {
    /// Resolve to the default profile's preset.
    ///
    /// The default profile is `claude`, so the OOTB experience matches
    /// observed Claude Code behavior. Users who want the previous
    /// per-directory Git semantics can set `--compat-profile git` (or
    /// `compat.profile = "git"` in `.waft.toml`).
    fn default() -> Self {
        Self::from_kernel(merge::Resolution {
            knobs: merge::default_knobs(),
            extra_excludes: Vec::new(),
        })
    }
}

impl ResolvedPolicy {
    /// Resolve a policy from an ordered iterator of layers (lowest
    /// precedence first).
    ///
    /// The resolution rule is:
    ///
    /// - Start from the default profile's coordinated preset.
    /// - Process layers from lowest to highest precedence.
    /// - Selecting a profile resets every profile-owned knob to that preset.
    /// - Explicit knobs in the same layer are then applied, followed by later
    ///   layers. A lower-layer knob therefore cannot silently modify a
    ///   profile selected at a higher layer.
    /// - `extra_excludes` accumulate across layers; a layer's
    ///   `replace_extra_excludes = true` clears accumulated values before
    ///   appending the layer's own values.
    ///
    /// The decision is made by the verified kernel
    /// `waft_verified::policy::merge::resolve_layers`, which is proven equal
    /// to a closed form (see that module's documentation).
    pub fn from_layers<'a, I>(layers: I) -> Self
    where
        I: IntoIterator<Item = &'a ConfigLayer>,
    {
        let views: Vec<merge::LayerView<'a>> =
            layers.into_iter().map(ConfigLayer::kernel_view).collect();
        Self::from_kernel(merge::resolve_layers(&views))
    }

    /// Lift a kernel resolution back to the production types (exhaustive).
    fn from_kernel(resolution: merge::Resolution) -> Self {
        let merge::Resolution {
            knobs,
            extra_excludes,
        } = resolution;
        let merge::ResolvedKnobs {
            profile,
            when_missing,
            semantics,
            symlink_policy,
            builtin_exclude_set,
            copy_strategy,
        } = knobs;
        Self {
            profile: profile.into(),
            when_missing: when_missing.into(),
            semantics: semantics.into(),
            symlink_policy: symlink_policy.into(),
            builtin_exclude_set: builtin_exclude_set.into(),
            extra_excludes,
            copy_strategy: copy_strategy.into(),
        }
    }
}

// --- TOML parsing ---

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    compat: Option<RawCompat>,
    #[serde(default)]
    worktreeinclude: Option<RawWorktreeinclude>,
    #[serde(default)]
    exclude: Option<RawExclude>,
    #[serde(default)]
    copy: Option<RawCopy>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCompat {
    #[serde(default)]
    profile: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWorktreeinclude {
    #[serde(default)]
    when_missing: Option<String>,
    #[serde(default)]
    semantics: Option<String>,
    #[serde(default)]
    symlink_policy: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExclude {
    #[serde(default)]
    builtin_set: Option<String>,
    #[serde(default)]
    extra: Option<Vec<String>>,
    #[serde(default)]
    replace_extra: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCopy {
    #[serde(default)]
    strategy: Option<String>,
}

/// Parse a TOML configuration string into a [`ConfigLayer`].
///
/// The `source` argument is used only in error messages.
pub fn parse_toml(source: &str, content: &str) -> Result<ConfigLayer> {
    let raw: RawConfig = toml::from_str(content).map_err(|e| Error::Config {
        message: format!("{source}: {e}"),
    })?;

    // An absent section behaves exactly like an empty one.
    let RawConfig {
        version,
        compat,
        worktreeinclude,
        exclude,
        copy,
    } = raw;
    let RawCompat { profile } = compat.unwrap_or_default();
    let RawWorktreeinclude {
        when_missing,
        semantics,
        symlink_policy,
    } = worktreeinclude.unwrap_or_default();
    let RawExclude {
        builtin_set,
        extra,
        replace_extra,
    } = exclude.unwrap_or_default();
    let RawCopy { strategy } = copy.unwrap_or_default();

    // The verified kernel applies the version gate, checks the fields in a
    // fixed order, and parses each present field.
    let knobs = raw::layer_from_raw(raw::RawKnobs {
        version,
        profile: profile.as_deref(),
        when_missing: when_missing.as_deref(),
        semantics: semantics.as_deref(),
        symlink_policy: symlink_policy.as_deref(),
        builtin_exclude_set: builtin_set.as_deref(),
        copy_strategy: strategy.as_deref(),
        replace_extra,
    })
    .map_err(|e| Error::Config {
        message: match e {
            raw::RawError::UnsupportedVersion(v) => {
                format!("{source}: unsupported config version {v}; expected 1")
            }
            raw::RawError::UnknownValue(field, s) => {
                format!("{source}: {}", unknown_value_message(field, s))
            }
        },
    })?;

    Ok(ConfigLayer::from_kernel(knobs, extra.unwrap_or_default()))
}

/// The "unknown ..." message for a rejected config-file value.
fn unknown_value_message(field: raw::RawField, value: &str) -> String {
    match field {
        raw::RawField::CompatProfile => CompatProfile::unknown_value(value),
        raw::RawField::WhenMissing => WhenMissingWorktreeinclude::unknown_value(value),
        raw::RawField::Semantics => WorktreeincludeSemantics::unknown_value(value),
        raw::RawField::SymlinkPolicy => SymlinkPolicy::unknown_value(value),
        raw::RawField::BuiltinExcludeSet => BuiltinExcludeSet::unknown_value(value),
        raw::RawField::CopyStrategy => CopyStrategy::unknown_value(value),
    }
}

// --- Environment variable layer ---

/// Source of an environment variable for diagnostics.
#[allow(dead_code)]
const ENV_SOURCE: &str = "environment";

/// Build a [`ConfigLayer`] from `WAFT_*` environment variables.
pub fn layer_from_env() -> Result<ConfigLayer> {
    layer_from_env_iter(std::env::vars())
}

/// Variant that reads from an explicit iterator of `(key, value)` pairs;
/// used by tests to avoid global env mutation.
pub fn layer_from_env_iter<I>(vars: I) -> Result<ConfigLayer>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut layer = ConfigLayer::default();

    for (k, v) in vars {
        // The verified key table recognizes exactly the eight names below
        // (case-sensitive); every other variable is ignored.
        let Some(key) = env_layer::EnvKey::classify(&k) else {
            continue;
        };
        // `key.as_str()` is proven equal to `k` here.
        let invalid = |e: String| Error::Config {
            message: format!("{ENV_SOURCE} ({}): {e}", key.as_str()),
        };
        match key {
            env_layer::EnvKey::CompatProfile => {
                layer.profile = Some(CompatProfile::parse(&v).map_err(invalid)?);
            }
            env_layer::EnvKey::WhenMissingWorktreeinclude => {
                layer.when_missing = Some(WhenMissingWorktreeinclude::parse(&v).map_err(invalid)?);
            }
            env_layer::EnvKey::WorktreeincludeSemantics => {
                layer.semantics = Some(WorktreeincludeSemantics::parse(&v).map_err(invalid)?);
            }
            env_layer::EnvKey::WorktreeincludeSymlinkPolicy => {
                layer.symlink_policy = Some(SymlinkPolicy::parse(&v).map_err(invalid)?);
            }
            env_layer::EnvKey::BuiltinExcludeSet => {
                layer.builtin_exclude_set = Some(BuiltinExcludeSet::parse(&v).map_err(invalid)?);
            }
            env_layer::EnvKey::ExtraExclude => {
                layer.extra_excludes = v
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            env_layer::EnvKey::ReplaceExtraExcludes => {
                layer.replace_extra_excludes = Some(parse_env_bool(&v).map_err(invalid)?);
            }
            env_layer::EnvKey::CopyStrategy => {
                layer.copy_strategy = Some(CopyStrategy::parse(&v).map_err(invalid)?);
            }
        }
    }

    Ok(layer)
}

/// Read a boolean environment value: trimmed and ASCII-lowercased here, then
/// decided by the verified token table.
fn parse_env_bool(s: &str) -> std::result::Result<bool, String> {
    let normalized = s.trim().to_ascii_lowercase();
    env_layer::parse_bool_token(&normalized)
        .ok_or_else(|| format!("expected boolean (1|0|true|false), got {normalized:?}"))
}

// --- File discovery ---

/// User config file location: `$XDG_CONFIG_HOME/waft/config.toml` or
/// `~/.config/waft/config.toml`.
///
/// The verified kernel `policy::user_source::select_config_home` queries
/// `XDG_CONFIG_HOME`, `HOME` and `USERPROFILE` in that order, stopping at the
/// first one that is set.
pub fn user_config_path() -> Option<PathBuf> {
    let mut lookup = ProcessConfigHome { value: None };
    let var = user_source::select_config_home(&mut lookup)?;
    // The kernel proves its last query was `var` and found it set, so
    // `value` holds that variable's value.
    let base = PathBuf::from(lookup.value?);
    Some(match var {
        user_source::ConfigHomeVar::XdgConfigHome => base.join("waft").join("config.toml"),
        user_source::ConfigHomeVar::Home | user_source::ConfigHomeVar::UserProfile => {
            base.join(".config").join("waft").join("config.toml")
        }
    })
}

/// Process-environment oracle for [`user_config_path`]: remembers the value
/// of the most recently queried variable.
struct ProcessConfigHome {
    value: Option<std::ffi::OsString>,
}

impl user_source::EnvLookup for ProcessConfigHome {
    fn is_set(&mut self, var: user_source::ConfigHomeVar) -> bool {
        self.value = std::env::var_os(var.as_str());
        self.value.is_some()
    }
}

/// Read and parse the user config layer if the file exists.
pub fn load_user_layer(path: Option<&Path>) -> Result<Option<ConfigLayer>> {
    let path = match path {
        Some(p) => p,
        None => return Ok(None),
    };
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path).map_err(|e| Error::Io {
        context: format!("reading user config {}", path.display()),
        source: e,
    })?;
    let layer = parse_toml(&path.display().to_string(), &content)?;
    Ok(Some(layer))
}

/// Walk upward from `cwd` looking for `.waft.toml` project configs, stopping
/// at (but including) the first directory that contains a `.git` entry.
///
/// Returned paths are ordered from outermost (closest to repo root) to
/// innermost (closest to cwd). Apply them in this order so deeper layers
/// override shallower ones.
pub fn discover_project_configs(cwd: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut current = Some(cwd);

    while let Some(dir) = current {
        let candidate = dir.join(".waft.toml");
        if candidate.is_file() {
            found.push(candidate);
        }

        let git_marker = dir.join(".git");
        if git_marker.exists() {
            break;
        }

        current = dir.parent();
    }

    // Reverse so outermost (shallowest) comes first.
    found.reverse();
    found
}

/// Discover project configs within one resolved repository.
///
/// `repo_root` is always included in the search. When `cwd` is inside that
/// repository, configs are then considered from the root down to `cwd`.
/// Discovery is bounded by `repo_root`, so a config from an unrelated process
/// working directory can never affect the resolved repository. If the path
/// crosses into a nested Git checkout or a registered `gitlink`, that
/// boundary and its descendants are excluded from discovery.
///
/// Returned paths are ordered from outermost to innermost.
pub fn discover_project_configs_in_repo(
    repo_root: &Path,
    cwd: &Path,
    gitlinks: &HashSet<String>,
    case_insensitive: bool,
) -> Vec<PathBuf> {
    let mut walk = ConfigSearchWalk {
        repo_root,
        gitlinks,
        case_insensitive,
        components: Vec::new(),
        current: repo_root.to_path_buf(),
        directories: vec![repo_root.to_path_buf()],
    };

    if let Ok(relative) = cwd.strip_prefix(repo_root) {
        walk.components = relative.components().collect();
        let kinds: Vec<discovery::ComponentKind> =
            walk.components.iter().map(component_kind).collect();
        // The verified kernel decides which components are descended into
        // and searched: names only, in order, up to the first `..`/root/
        // prefix component or nested boundary, whichever comes first.
        discovery::walk_toward_invocation(&kinds, &mut walk);
    }

    walk.directories
        .into_iter()
        .map(|dir| dir.join(".waft.toml"))
        // Keep symlinks in the discovered set so loading can reject them
        // explicitly, including dangling symlinks.
        .filter(|path| {
            std::fs::symlink_metadata(path).is_ok_and(|metadata| {
                let file_type = metadata.file_type();
                file_type.is_file() || file_type.is_symlink()
            })
        })
        .collect()
}

/// Classify a relative path component for the discovery kernel.
fn component_kind(component: &Component<'_>) -> discovery::ComponentKind {
    match component {
        Component::CurDir => discovery::ComponentKind::CurDir,
        Component::Normal(_) => discovery::ComponentKind::Normal,
        // Callers pass an absolute, normalized source-view path. If that
        // invariant is violated, the search stops here.
        Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
            discovery::ComponentKind::Stop
        }
    }
}

/// Production side of [`discovery::walk_toward_invocation`]: tracks the
/// directory reached so far and probes it for nested-checkout boundaries.
struct ConfigSearchWalk<'a> {
    repo_root: &'a Path,
    gitlinks: &'a HashSet<String>,
    case_insensitive: bool,
    components: Vec<Component<'a>>,
    current: PathBuf,
    directories: Vec<PathBuf>,
}

impl discovery::DiscoveryWalk for ConfigSearchWalk<'_> {
    fn enter(&mut self, i: usize) -> bool {
        // The kernel proves it only enters in-bounds `Normal` components.
        if let Some(Component::Normal(part)) = self.components.get(i) {
            self.current.push(part);
        }
        // Do not let a nested checkout's branch-local config influence
        // selection in the outer source repository.
        let is_gitlink = crate::path::RepoRelPath::normalize(&self.current, self.repo_root)
            .ok()
            .is_some_and(|relative| {
                self.gitlinks.iter().any(|gitlink| {
                    crate::git::repo_paths_equivalent(
                        relative.as_str(),
                        gitlink,
                        crate::walk::boundary_case_folds(self.case_insensitive),
                    )
                })
            });
        self.current.join(".git").exists() || is_gitlink
    }

    fn search(&mut self, _i: usize) {
        self.directories.push(self.current.clone());
    }
}

/// Read and parse a list of project config files into layers, in order.
pub fn load_project_layers(paths: &[PathBuf]) -> Result<Vec<ConfigLayer>> {
    let mut layers = Vec::with_capacity(paths.len());
    for path in paths {
        let metadata = std::fs::symlink_metadata(path).map_err(|e| Error::Io {
            context: format!("inspecting project config {}", path.display()),
            source: e,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(Error::Config {
                message: format!(
                    "project config {} is a symlink; source project configs must be regular files",
                    path.display()
                ),
            });
        }
        if !metadata.file_type().is_file() {
            return Err(Error::Config {
                message: format!("project config {} is not a regular file", path.display()),
            });
        }
        let content = std::fs::read_to_string(path).map_err(|e| Error::Io {
            context: format!("reading project config {}", path.display()),
            source: e,
        })?;
        let layer = parse_toml(&path.display().to_string(), &content)?;
        layers.push(layer);
    }
    Ok(layers)
}

// --- Top-level resolution ---

/// Inputs needed to resolve a [`ResolvedPolicy`] from all sources.
#[derive(Debug, Default)]
pub struct PolicyResolutionInputs {
    /// Built-in defaults layer (usually `ConfigLayer::default()`).
    pub defaults: ConfigLayer,
    /// User config layer (already parsed), if any.
    pub user: Option<ConfigLayer>,
    /// Project config layers, in the order they should be applied
    /// (outermost first, innermost last).
    pub project: Vec<ConfigLayer>,
    /// Environment variable layer.
    pub env: ConfigLayer,
    /// CLI flag layer.
    pub cli: ConfigLayer,
}

impl PolicyResolutionInputs {
    /// Resolve to a final [`ResolvedPolicy`] honoring layer precedence:
    /// defaults, user, project layers (outermost first), environment, CLI.
    ///
    /// The ordering and the merge are the verified kernel
    /// `waft_verified::policy::merge::resolve_inputs`.
    pub fn resolve(&self) -> ResolvedPolicy {
        let Self {
            defaults,
            user,
            project,
            env,
            cli,
        } = self;
        let project: Vec<merge::LayerView<'_>> =
            project.iter().map(ConfigLayer::kernel_view).collect();
        ResolvedPolicy::from_kernel(merge::resolve_inputs(
            defaults.kernel_view(),
            user.as_ref().map(ConfigLayer::kernel_view),
            &project,
            env.kernel_view(),
            cli.kernel_view(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_resolved_policy_matches_claude_preset() {
        let policy = ResolvedPolicy::default();
        assert_eq!(policy.profile, CompatProfile::Claude);
        assert_eq!(policy.when_missing, WhenMissingWorktreeinclude::Blank);
        assert_eq!(policy.semantics, WorktreeincludeSemantics::Claude202604);
        assert_eq!(policy.symlink_policy, SymlinkPolicy::Follow);
        assert_eq!(policy.builtin_exclude_set, BuiltinExcludeSet::None);
        assert!(policy.extra_excludes.is_empty());
    }

    #[test]
    fn parse_full_toml_roundtrips() {
        let content = r#"
version = 1

[compat]
profile = "git"

[worktreeinclude]
when_missing = "all-ignored"
semantics = "wt-0.39"
symlink_policy = "ignore"

[exclude]
builtin_set = "tooling-v1"
extra = ["build/", "logs/*.log"]
replace_extra = true
"#;
        let layer = parse_toml("inline", content).unwrap();
        assert_eq!(layer.profile, Some(CompatProfile::Git));
        assert_eq!(
            layer.when_missing,
            Some(WhenMissingWorktreeinclude::AllIgnored)
        );
        assert_eq!(layer.semantics, Some(WorktreeincludeSemantics::Wt039));
        assert_eq!(layer.symlink_policy, Some(SymlinkPolicy::Ignore));
        assert_eq!(
            layer.builtin_exclude_set,
            Some(BuiltinExcludeSet::ToolingV1)
        );
        assert_eq!(
            layer.extra_excludes,
            vec!["build/".to_string(), "logs/*.log".to_string()]
        );
        assert_eq!(layer.replace_extra_excludes, Some(true));
    }

    #[test]
    fn parse_rejects_unknown_top_level_key() {
        let err = parse_toml("inline", "bogus = 1\n").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("bogus"), "msg: {msg}");
    }

    #[test]
    fn parse_rejects_unknown_section_key() {
        let err = parse_toml(
            "inline",
            r#"
[worktreeinclude]
mystery = "x"
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("mystery"));
    }

    #[test]
    fn parse_rejects_bad_enum_value() {
        let err = parse_toml(
            "inline",
            r#"
[compat]
profile = "rainbow"
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("rainbow"));
    }

    #[test]
    fn parse_rejects_unsupported_version() {
        let err = parse_toml("inline", "version = 2\n").unwrap_err();
        assert!(err.to_string().contains("version"));
    }

    #[test]
    fn empty_toml_yields_empty_layer() {
        let layer = parse_toml("inline", "").unwrap();
        assert_eq!(layer, ConfigLayer::default());
    }

    #[test]
    fn env_layer_basic() {
        let env = vec![
            ("WAFT_COMPAT_PROFILE".to_string(), "git".to_string()),
            (
                "WAFT_WHEN_MISSING_WORKTREEINCLUDE".to_string(),
                "all-ignored".to_string(),
            ),
            ("WAFT_EXTRA_EXCLUDE".to_string(), "a, b ,c".to_string()),
            (
                "WAFT_REPLACE_EXTRA_EXCLUDES".to_string(),
                "true".to_string(),
            ),
            ("UNRELATED".to_string(), "ignored".to_string()),
        ];
        let layer = layer_from_env_iter(env).unwrap();
        assert_eq!(layer.profile, Some(CompatProfile::Git));
        assert_eq!(
            layer.when_missing,
            Some(WhenMissingWorktreeinclude::AllIgnored)
        );
        assert_eq!(layer.extra_excludes, vec!["a", "b", "c"]);
        assert_eq!(layer.replace_extra_excludes, Some(true));
    }

    #[test]
    fn env_layer_rejects_bad_enum() {
        let env = vec![(
            "WAFT_WORKTREEINCLUDE_SEMANTICS".to_string(),
            "huh".to_string(),
        )];
        assert!(layer_from_env_iter(env).is_err());
    }

    #[test]
    fn env_layer_replace_extra_false_variants() {
        for s in ["0", "false", "FALSE", "off", "no", ""] {
            let env = vec![("WAFT_REPLACE_EXTRA_EXCLUDES".to_string(), s.to_string())];
            let layer = layer_from_env_iter(env).unwrap();
            assert_eq!(
                layer.replace_extra_excludes,
                Some(false),
                "input {s:?} should parse as false"
            );
        }
    }

    #[test]
    fn layer_application_overwrites_scalars_and_appends_extras() {
        let lower = ConfigLayer {
            profile: Some(CompatProfile::Git),
            extra_excludes: vec!["a".into()],
            ..ConfigLayer::default()
        };
        let upper = ConfigLayer {
            profile: Some(CompatProfile::Wt),
            extra_excludes: vec!["b".into()],
            ..ConfigLayer::default()
        };
        let policy = ResolvedPolicy::from_layers([&lower, &upper]);
        assert_eq!(policy.profile, CompatProfile::Wt);
        assert_eq!(
            policy.extra_excludes,
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn replace_extra_clears_lower_layers() {
        let lower = ConfigLayer {
            extra_excludes: vec!["a".into(), "b".into()],
            ..ConfigLayer::default()
        };
        let upper = ConfigLayer {
            extra_excludes: vec!["c".into()],
            replace_extra_excludes: Some(true),
            ..ConfigLayer::default()
        };
        let policy = ResolvedPolicy::from_layers([&lower, &upper]);
        assert_eq!(policy.extra_excludes, vec!["c".to_string()]);
    }

    #[test]
    fn discover_project_configs_walks_to_git_root() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let sub = root.join("a/b/c");
        std::fs::create_dir_all(&sub).unwrap();
        // Mark root with a .git directory
        std::fs::create_dir_all(root.join(".git")).unwrap();
        // Root and inner waft configs
        std::fs::write(root.join(".waft.toml"), "version = 1\n").unwrap();
        std::fs::write(sub.join(".waft.toml"), "version = 1\n").unwrap();

        let found = discover_project_configs(&sub);
        assert_eq!(found.len(), 2);
        // Outermost first
        assert_eq!(found[0], root.join(".waft.toml"));
        assert_eq!(found[1], sub.join(".waft.toml"));
    }

    #[test]
    fn discover_project_configs_no_files_no_repo() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("isolated");
        std::fs::create_dir_all(&dir).unwrap();
        let found = discover_project_configs(&dir);
        assert!(found.is_empty());
    }

    #[test]
    fn in_repo_discovery_is_bounded_to_resolved_source() {
        let tmp = tempfile::TempDir::new().unwrap();
        let source = tmp.path().join("source");
        let unrelated = tmp.path().join("unrelated");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&unrelated).unwrap();
        std::fs::write(source.join(".waft.toml"), "version = 1\n").unwrap();
        std::fs::write(unrelated.join(".waft.toml"), "version = 1\n").unwrap();

        assert_eq!(
            discover_project_configs_in_repo(&source, &unrelated, &HashSet::new(), false),
            vec![source.join(".waft.toml")]
        );
    }

    #[test]
    fn in_repo_discovery_stops_before_nested_checkout() {
        let tmp = tempfile::TempDir::new().unwrap();
        let source = tmp.path();
        let nested = source.join("vendor/dependency");
        std::fs::create_dir_all(nested.join(".git")).unwrap();
        std::fs::write(source.join(".waft.toml"), "version = 1\n").unwrap();
        std::fs::write(nested.join(".waft.toml"), "version = 1\n").unwrap();

        assert_eq!(
            discover_project_configs_in_repo(source, &nested, &HashSet::new(), false),
            vec![source.join(".waft.toml")]
        );
    }

    #[test]
    fn in_repo_discovery_stops_before_registered_gitlink() {
        let tmp = tempfile::TempDir::new().unwrap();
        let source = tmp.path();
        let gitlink = source.join("vendor/dependency");
        std::fs::create_dir_all(&gitlink).unwrap();
        std::fs::write(source.join(".waft.toml"), "version = 1\n").unwrap();
        std::fs::write(gitlink.join(".waft.toml"), "version = 1\n").unwrap();
        let gitlinks = HashSet::from(["vendor/dependency".to_string()]);

        assert_eq!(
            discover_project_configs_in_repo(source, &gitlink, &gitlinks, false),
            vec![source.join(".waft.toml")]
        );
    }

    #[test]
    fn in_repo_discovery_matches_gitlinks_with_unicode_case_folding() {
        let tmp = tempfile::TempDir::new().unwrap();
        let source = tmp.path();
        let gitlink = source.join("vendor/Äddon");
        std::fs::create_dir_all(&gitlink).unwrap();
        std::fs::write(source.join(".waft.toml"), "version = 1\n").unwrap();
        std::fs::write(gitlink.join(".waft.toml"), "version = 1\n").unwrap();
        let gitlinks = HashSet::from(["vendor/äddon".to_string()]);

        assert_eq!(
            discover_project_configs_in_repo(source, &gitlink, &gitlinks, true),
            vec![source.join(".waft.toml")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn project_config_symlink_is_discovered_then_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let target = tmp.path().join("external.toml");
        let config = tmp.path().join(".waft.toml");
        std::fs::write(&target, "[compat]\nprofile = \"wt\"\n").unwrap();
        std::os::unix::fs::symlink(&target, &config).unwrap();

        let configs =
            discover_project_configs_in_repo(tmp.path(), tmp.path(), &HashSet::new(), false);
        assert_eq!(configs, vec![config.clone()]);
        let error = load_project_layers(&configs).unwrap_err();
        assert!(error.to_string().contains("is a symlink"));
    }

    // --- Glue between production types and the verified policy kernel ---

    #[test]
    fn enum_conversions_round_trip_and_match_clap_names() {
        use clap::ValueEnum;

        for &v in CompatProfile::value_variants() {
            assert_eq!(CompatProfile::from(vocab::CompatProfile::from(v)), v);
            assert_eq!(v.to_possible_value().unwrap().get_name(), v.as_str());
            assert_eq!(CompatProfile::parse(v.as_str()), Ok(v));
        }
        for &v in WhenMissingWorktreeinclude::value_variants() {
            assert_eq!(
                WhenMissingWorktreeinclude::from(vocab::WhenMissingWorktreeinclude::from(v)),
                v
            );
            assert_eq!(v.to_possible_value().unwrap().get_name(), v.as_str());
            assert_eq!(WhenMissingWorktreeinclude::parse(v.as_str()), Ok(v));
        }
        for &v in WorktreeincludeSemantics::value_variants() {
            assert_eq!(
                WorktreeincludeSemantics::from(vocab::WorktreeincludeSemantics::from(v)),
                v
            );
            assert_eq!(v.to_possible_value().unwrap().get_name(), v.as_str());
            assert_eq!(WorktreeincludeSemantics::parse(v.as_str()), Ok(v));
        }
        for &v in SymlinkPolicy::value_variants() {
            assert_eq!(SymlinkPolicy::from(vocab::SymlinkPolicy::from(v)), v);
            assert_eq!(v.to_possible_value().unwrap().get_name(), v.as_str());
            assert_eq!(SymlinkPolicy::parse(v.as_str()), Ok(v));
        }
        for &v in BuiltinExcludeSet::value_variants() {
            assert_eq!(
                BuiltinExcludeSet::from(vocab::BuiltinExcludeSet::from(v)),
                v
            );
            assert_eq!(v.to_possible_value().unwrap().get_name(), v.as_str());
            assert_eq!(BuiltinExcludeSet::parse(v.as_str()), Ok(v));
        }
        for &v in CopyStrategy::value_variants() {
            assert_eq!(CopyStrategy::from(vocab::CopyStrategy::from(v)), v);
            assert_eq!(v.to_possible_value().unwrap().get_name(), v.as_str());
            assert_eq!(CopyStrategy::parse(v.as_str()), Ok(v));
        }
    }

    #[test]
    fn unknown_value_messages_are_unchanged() {
        assert_eq!(
            CompatProfile::parse("Git").unwrap_err(),
            "unknown compat profile \"Git\"; expected claude|git|wt"
        );
        assert_eq!(
            WhenMissingWorktreeinclude::parse(" blank").unwrap_err(),
            "unknown when_missing_worktreeinclude \" blank\"; expected blank|all-ignored"
        );
        assert_eq!(
            WorktreeincludeSemantics::parse("wt").unwrap_err(),
            "unknown worktreeinclude semantics \"wt\"; expected claude-2026-04|git|wt-0.39"
        );
        assert_eq!(
            SymlinkPolicy::parse("").unwrap_err(),
            "unknown symlink policy \"\"; expected follow|ignore|error"
        );
        assert_eq!(
            BuiltinExcludeSet::parse("tooling").unwrap_err(),
            "unknown builtin exclude set \"tooling\"; expected none|tooling-v1"
        );
        assert_eq!(
            CopyStrategy::parse("Reflink").unwrap_err(),
            "unknown copy strategy \"Reflink\"; expected auto|simple-copy|cow-copy"
        );
        for (alias, strategy) in [
            ("default", CopyStrategy::Auto),
            ("simple", CopyStrategy::SimpleCopy),
            ("cow", CopyStrategy::CowCopy),
            ("reflink", CopyStrategy::CowCopy),
        ] {
            assert_eq!(CopyStrategy::parse(alias), Ok(strategy));
        }
    }

    #[test]
    fn config_layer_round_trips_through_kernel_view() {
        let layer = ConfigLayer {
            profile: Some(CompatProfile::Wt),
            when_missing: Some(WhenMissingWorktreeinclude::Blank),
            semantics: None,
            symlink_policy: Some(SymlinkPolicy::Error),
            builtin_exclude_set: Some(BuiltinExcludeSet::None),
            extra_excludes: vec!["a".into(), "b/".into()],
            replace_extra_excludes: Some(false),
            copy_strategy: Some(CopyStrategy::CowCopy),
        };
        let view = layer.kernel_view();
        assert_eq!(view.extras, layer.extra_excludes.as_slice());
        assert_eq!(
            ConfigLayer::from_kernel(view.knobs, view.extras.to_vec()),
            layer
        );
    }

    #[test]
    fn env_key_table_matches_variable_names() {
        for name in [
            "WAFT_COMPAT_PROFILE",
            "WAFT_WHEN_MISSING_WORKTREEINCLUDE",
            "WAFT_WORKTREEINCLUDE_SEMANTICS",
            "WAFT_WORKTREEINCLUDE_SYMLINK_POLICY",
            "WAFT_BUILTIN_EXCLUDE_SET",
            "WAFT_EXTRA_EXCLUDE",
            "WAFT_REPLACE_EXTRA_EXCLUDES",
            "WAFT_COPY_STRATEGY",
        ] {
            let key = env_layer::EnvKey::classify(name).expect(name);
            assert_eq!(key.as_str(), name);
        }
        for name in ["waft_compat_profile", "WAFT_COMPAT_PROFILE_", "WAFT_", ""] {
            assert_eq!(env_layer::EnvKey::classify(name), None, "{name:?}");
        }
    }

    #[test]
    fn env_bool_error_echoes_trimmed_lowercased_value() {
        assert_eq!(parse_env_bool(" YES "), Ok(true));
        assert_eq!(parse_env_bool("\tOff\n"), Ok(false));
        assert_eq!(
            parse_env_bool("  Maybe "),
            Err("expected boolean (1|0|true|false), got \"maybe\"".to_string())
        );
    }

    #[test]
    fn in_repo_discovery_stops_at_parent_dir_component() {
        // Pins current behavior for a non-normalized invocation path: the
        // directories before the `..` are still searched.
        let tmp = tempfile::TempDir::new().unwrap();
        let source = tmp.path();
        std::fs::create_dir_all(source.join("a")).unwrap();
        std::fs::create_dir_all(source.join("b")).unwrap();
        std::fs::write(source.join(".waft.toml"), "version = 1\n").unwrap();
        std::fs::write(source.join("a/.waft.toml"), "version = 1\n").unwrap();
        std::fs::write(source.join("b/.waft.toml"), "version = 1\n").unwrap();

        assert_eq!(
            discover_project_configs_in_repo(
                source,
                &source.join("a/../b"),
                &HashSet::new(),
                false
            ),
            vec![source.join(".waft.toml"), source.join("a/.waft.toml")]
        );
    }

    /// The pre-kernel `ResolvedPolicy::from_layers`, kept as a differential
    /// oracle for the verified merge.
    fn reference_from_layers(layers: &[ConfigLayer]) -> ResolvedPolicy {
        let mut resolved = ResolvedPolicy {
            profile: CompatProfile::Claude,
            when_missing: WhenMissingWorktreeinclude::Blank,
            semantics: WorktreeincludeSemantics::Claude202604,
            symlink_policy: SymlinkPolicy::Follow,
            builtin_exclude_set: BuiltinExcludeSet::None,
            extra_excludes: Vec::new(),
            copy_strategy: CopyStrategy::Auto,
        };
        for layer in layers {
            if let Some(v) = layer.profile {
                let (when_missing, semantics, symlink_policy, builtin_exclude_set) = match v {
                    CompatProfile::Claude => (
                        WhenMissingWorktreeinclude::Blank,
                        WorktreeincludeSemantics::Claude202604,
                        SymlinkPolicy::Follow,
                        BuiltinExcludeSet::None,
                    ),
                    CompatProfile::Git => (
                        WhenMissingWorktreeinclude::Blank,
                        WorktreeincludeSemantics::Git,
                        SymlinkPolicy::Ignore,
                        BuiltinExcludeSet::None,
                    ),
                    CompatProfile::Wt => (
                        WhenMissingWorktreeinclude::AllIgnored,
                        WorktreeincludeSemantics::Wt039,
                        SymlinkPolicy::Follow,
                        BuiltinExcludeSet::ToolingV1,
                    ),
                };
                resolved.profile = v;
                resolved.when_missing = when_missing;
                resolved.semantics = semantics;
                resolved.symlink_policy = symlink_policy;
                resolved.builtin_exclude_set = builtin_exclude_set;
            }
            if let Some(v) = layer.when_missing {
                resolved.when_missing = v;
            }
            if let Some(v) = layer.semantics {
                resolved.semantics = v;
            }
            if let Some(v) = layer.symlink_policy {
                resolved.symlink_policy = v;
            }
            if let Some(v) = layer.builtin_exclude_set {
                resolved.builtin_exclude_set = v;
            }
            if let Some(v) = layer.copy_strategy {
                resolved.copy_strategy = v;
            }
            if layer.replace_extra_excludes == Some(true) {
                resolved.extra_excludes.clear();
            }
            resolved
                .extra_excludes
                .extend(layer.extra_excludes.iter().cloned());
        }
        resolved
    }

    fn pick<T: Copy + std::fmt::Debug + 'static>(
        values: &'static [T],
    ) -> impl proptest::strategy::Strategy<Value = Option<T>> {
        proptest::option::of(proptest::sample::select(values))
    }

    fn arb_layer() -> impl proptest::strategy::Strategy<Value = ConfigLayer> {
        use proptest::prelude::*;

        (
            pick(&[CompatProfile::Claude, CompatProfile::Git, CompatProfile::Wt]),
            pick(&[
                WhenMissingWorktreeinclude::Blank,
                WhenMissingWorktreeinclude::AllIgnored,
            ]),
            pick(&[
                WorktreeincludeSemantics::Claude202604,
                WorktreeincludeSemantics::Git,
                WorktreeincludeSemantics::Wt039,
            ]),
            pick(&[
                SymlinkPolicy::Follow,
                SymlinkPolicy::Ignore,
                SymlinkPolicy::Error,
            ]),
            pick(&[BuiltinExcludeSet::None, BuiltinExcludeSet::ToolingV1]),
            proptest::collection::vec("[a-c]{0,2}", 0..3),
            proptest::option::of(any::<bool>()),
            pick(&[
                CopyStrategy::Auto,
                CopyStrategy::SimpleCopy,
                CopyStrategy::CowCopy,
            ]),
        )
            .prop_map(
                |(
                    profile,
                    when_missing,
                    semantics,
                    symlink_policy,
                    builtin_exclude_set,
                    extra_excludes,
                    replace_extra_excludes,
                    copy_strategy,
                )| ConfigLayer {
                    profile,
                    when_missing,
                    semantics,
                    symlink_policy,
                    builtin_exclude_set,
                    extra_excludes,
                    replace_extra_excludes,
                    copy_strategy,
                },
            )
    }

    proptest::proptest! {
        #[test]
        fn verified_merge_matches_reference(
            layers in proptest::collection::vec(arb_layer(), 0..7),
        ) {
            proptest::prop_assert_eq!(
                ResolvedPolicy::from_layers(&layers),
                reference_from_layers(&layers)
            );
        }

        #[test]
        fn verified_resolve_matches_reference_order(
            defaults in arb_layer(),
            user in proptest::option::of(arb_layer()),
            project in proptest::collection::vec(arb_layer(), 0..3),
            env in arb_layer(),
            cli in arb_layer(),
        ) {
            let mut ordered = vec![defaults.clone()];
            ordered.extend(user.clone());
            ordered.extend(project.iter().cloned());
            ordered.push(env.clone());
            ordered.push(cli.clone());
            let inputs = PolicyResolutionInputs { defaults, user, project, env, cli };
            proptest::prop_assert_eq!(inputs.resolve(), reference_from_layers(&ordered));
        }
    }
}
