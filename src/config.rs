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
//! Note: the resolved policy is plumbed through subcommand entrypoints, but
//! does not yet drive runtime behavior. Subsequent PRs wire individual knobs
//! (`when_missing`, `symlink_policy`, exclude filters, semantics engines)
//! into the selector and copy paths.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};

// --- Public enums for each knob ---

/// Top-level compatibility preset.
///
/// Selects a coordinated set of defaults tuned to match a particular tool's
/// observed behavior. The preset is expanded into concrete knob values during
/// policy resolution; explicit knob overrides take precedence.
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
        match self {
            Self::Claude => "claude",
            Self::Git => "git",
            Self::Wt => "wt",
        }
    }

    fn parse(s: &str) -> std::result::Result<Self, String> {
        match s {
            "claude" => Ok(Self::Claude),
            "git" => Ok(Self::Git),
            "wt" => Ok(Self::Wt),
            other => Err(format!(
                "unknown compat profile {other:?}; expected claude|git|wt"
            )),
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
        match self {
            Self::Blank => "blank",
            Self::AllIgnored => "all-ignored",
        }
    }

    fn parse(s: &str) -> std::result::Result<Self, String> {
        match s {
            "blank" => Ok(Self::Blank),
            "all-ignored" => Ok(Self::AllIgnored),
            other => Err(format!(
                "unknown when_missing_worktreeinclude {other:?}; expected blank|all-ignored"
            )),
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
        match self {
            Self::Claude202604 => "claude-2026-04",
            Self::Git => "git",
            Self::Wt039 => "wt-0.39",
        }
    }

    fn parse(s: &str) -> std::result::Result<Self, String> {
        match s {
            "claude-2026-04" => Ok(Self::Claude202604),
            "git" => Ok(Self::Git),
            "wt-0.39" => Ok(Self::Wt039),
            other => Err(format!(
                "unknown worktreeinclude semantics {other:?}; expected claude-2026-04|git|wt-0.39"
            )),
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
        match self {
            Self::Follow => "follow",
            Self::Ignore => "ignore",
            Self::Error => "error",
        }
    }

    fn parse(s: &str) -> std::result::Result<Self, String> {
        match s {
            "follow" => Ok(Self::Follow),
            "ignore" => Ok(Self::Ignore),
            "error" => Ok(Self::Error),
            other => Err(format!(
                "unknown symlink policy {other:?}; expected follow|ignore|error"
            )),
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
        match self {
            Self::None => "none",
            Self::ToolingV1 => "tooling-v1",
        }
    }

    fn parse(s: &str) -> std::result::Result<Self, String> {
        match s {
            "none" => Ok(Self::None),
            "tooling-v1" => Ok(Self::ToolingV1),
            other => Err(format!(
                "unknown builtin exclude set {other:?}; expected none|tooling-v1"
            )),
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
}

impl Default for ResolvedPolicy {
    /// Built-in defaults.
    ///
    /// These are chosen to preserve the behavior that existed before the
    /// modes feature was introduced, NOT the eventual Claude preset. The
    /// default flip lands in the final PR of this feature epic.
    fn default() -> Self {
        Self {
            profile: CompatProfile::Claude,
            when_missing: WhenMissingWorktreeinclude::Blank,
            semantics: WorktreeincludeSemantics::Claude202604,
            // Pre-modes code rejected symlinked .worktreeinclude files;
            // keep that until the symlink policy is wired up.
            symlink_policy: SymlinkPolicy::Error,
            builtin_exclude_set: BuiltinExcludeSet::None,
            extra_excludes: Vec::new(),
        }
    }
}

impl ResolvedPolicy {
    /// Apply a layer's set values on top of `self`.
    ///
    /// Scalar keys overwrite. The `extra_excludes` array appends unless the
    /// layer sets `replace_extra_excludes = true`, in which case the array
    /// is replaced by the layer's values.
    fn apply(&mut self, layer: &ConfigLayer) {
        if let Some(v) = layer.profile {
            self.profile = v;
        }
        if let Some(v) = layer.when_missing {
            self.when_missing = v;
        }
        if let Some(v) = layer.semantics {
            self.semantics = v;
        }
        if let Some(v) = layer.symlink_policy {
            self.symlink_policy = v;
        }
        if let Some(v) = layer.builtin_exclude_set {
            self.builtin_exclude_set = v;
        }
        if layer.replace_extra_excludes == Some(true) {
            self.extra_excludes.clear();
        }
        self.extra_excludes
            .extend(layer.extra_excludes.iter().cloned());
    }

    /// Resolve a policy from an ordered iterator of layers (lowest
    /// precedence first).
    ///
    /// PR1 contract: scalar values overwrite in precedence order; the
    /// `compat.profile` is recorded but does not yet expand into other
    /// knobs. Preset expansion lands in PR2.
    pub fn from_layers<'a, I>(layers: I) -> Self
    where
        I: IntoIterator<Item = &'a ConfigLayer>,
    {
        let mut policy = Self::default();
        for layer in layers {
            policy.apply(layer);
        }
        policy
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

/// Parse a TOML configuration string into a [`ConfigLayer`].
///
/// The `source` argument is used only in error messages.
pub fn parse_toml(source: &str, content: &str) -> Result<ConfigLayer> {
    let raw: RawConfig = toml::from_str(content).map_err(|e| Error::Config {
        message: format!("{source}: {e}"),
    })?;

    if let Some(v) = raw.version
        && v != 1
    {
        return Err(Error::Config {
            message: format!("{source}: unsupported config version {v}; expected 1"),
        });
    }

    let mut layer = ConfigLayer::default();
    let cfg_err = |e: String| Error::Config {
        message: format!("{source}: {e}"),
    };

    if let Some(compat) = raw.compat
        && let Some(s) = compat.profile
    {
        layer.profile = Some(CompatProfile::parse(&s).map_err(cfg_err)?);
    }

    if let Some(wti) = raw.worktreeinclude {
        if let Some(s) = wti.when_missing {
            layer.when_missing = Some(WhenMissingWorktreeinclude::parse(&s).map_err(cfg_err)?);
        }
        if let Some(s) = wti.semantics {
            layer.semantics = Some(WorktreeincludeSemantics::parse(&s).map_err(cfg_err)?);
        }
        if let Some(s) = wti.symlink_policy {
            layer.symlink_policy = Some(SymlinkPolicy::parse(&s).map_err(cfg_err)?);
        }
    }

    if let Some(excl) = raw.exclude {
        if let Some(s) = excl.builtin_set {
            layer.builtin_exclude_set = Some(BuiltinExcludeSet::parse(&s).map_err(cfg_err)?);
        }
        if let Some(extra) = excl.extra {
            layer.extra_excludes = extra;
        }
        layer.replace_extra_excludes = excl.replace_extra;
    }

    Ok(layer)
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
        match k.as_str() {
            "WAFT_COMPAT_PROFILE" => {
                layer.profile = Some(CompatProfile::parse(&v).map_err(|e| Error::Config {
                    message: format!("{ENV_SOURCE} (WAFT_COMPAT_PROFILE): {e}"),
                })?);
            }
            "WAFT_WHEN_MISSING_WORKTREEINCLUDE" => {
                layer.when_missing =
                    Some(
                        WhenMissingWorktreeinclude::parse(&v).map_err(|e| Error::Config {
                            message: format!(
                                "{ENV_SOURCE} (WAFT_WHEN_MISSING_WORKTREEINCLUDE): {e}"
                            ),
                        })?,
                    );
            }
            "WAFT_WORKTREEINCLUDE_SEMANTICS" => {
                layer.semantics =
                    Some(
                        WorktreeincludeSemantics::parse(&v).map_err(|e| Error::Config {
                            message: format!("{ENV_SOURCE} (WAFT_WORKTREEINCLUDE_SEMANTICS): {e}"),
                        })?,
                    );
            }
            "WAFT_WORKTREEINCLUDE_SYMLINK_POLICY" => {
                layer.symlink_policy =
                    Some(SymlinkPolicy::parse(&v).map_err(|e| Error::Config {
                        message: format!("{ENV_SOURCE} (WAFT_WORKTREEINCLUDE_SYMLINK_POLICY): {e}"),
                    })?);
            }
            "WAFT_BUILTIN_EXCLUDE_SET" => {
                layer.builtin_exclude_set =
                    Some(BuiltinExcludeSet::parse(&v).map_err(|e| Error::Config {
                        message: format!("{ENV_SOURCE} (WAFT_BUILTIN_EXCLUDE_SET): {e}"),
                    })?);
            }
            "WAFT_EXTRA_EXCLUDE" => {
                layer.extra_excludes = v
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            "WAFT_REPLACE_EXTRA_EXCLUDES" => {
                layer.replace_extra_excludes =
                    Some(parse_env_bool(&v).map_err(|e| Error::Config {
                        message: format!("{ENV_SOURCE} (WAFT_REPLACE_EXTRA_EXCLUDES): {e}"),
                    })?);
            }
            _ => {}
        }
    }

    Ok(layer)
}

fn parse_env_bool(s: &str) -> std::result::Result<bool, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" | "" => Ok(false),
        other => Err(format!("expected boolean (1|0|true|false), got {other:?}")),
    }
}

// --- File discovery ---

/// User config file location: `$XDG_CONFIG_HOME/waft/config.toml` or
/// `~/.config/waft/config.toml`.
pub fn user_config_path() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        let p = PathBuf::from(xdg).join("waft").join("config.toml");
        return Some(p);
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("waft")
            .join("config.toml"),
    )
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

/// Read and parse a list of project config files into layers, in order.
pub fn load_project_layers(paths: &[PathBuf]) -> Result<Vec<ConfigLayer>> {
    let mut layers = Vec::with_capacity(paths.len());
    for path in paths {
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
    /// Resolve to a final [`ResolvedPolicy`] honoring layer precedence.
    pub fn resolve(&self) -> ResolvedPolicy {
        let mut layers: Vec<&ConfigLayer> = Vec::new();
        layers.push(&self.defaults);
        if let Some(u) = &self.user {
            layers.push(u);
        }
        for p in &self.project {
            layers.push(p);
        }
        layers.push(&self.env);
        layers.push(&self.cli);
        ResolvedPolicy::from_layers(layers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_resolved_policy_matches_baseline() {
        let policy = ResolvedPolicy::default();
        assert_eq!(policy.profile, CompatProfile::Claude);
        assert_eq!(policy.when_missing, WhenMissingWorktreeinclude::Blank);
        assert_eq!(policy.semantics, WorktreeincludeSemantics::Claude202604);
        assert_eq!(policy.symlink_policy, SymlinkPolicy::Error);
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
}
