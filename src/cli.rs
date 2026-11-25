//! CLI argument parsing and command dispatch.

use clap::Parser;
use std::path::{Path, PathBuf};
use waft_verified::context::cli_policy;
use waft_verified::path::model::Comp;
use waft_verified::worktree_map;

use crate::config::{
    BuiltinExcludeSet, CompatProfile, ConfigLayer, CopyStrategy, PolicyResolutionInputs,
    ResolvedPolicy, SymlinkPolicy, WhenMissingWorktreeinclude, WorktreeincludeSemantics,
    discover_project_configs_in_repo, layer_from_env, load_project_layers, load_user_layer,
    user_config_path,
};
use crate::context::{self, CommandKind};
use crate::error::{Error, Result};
use crate::git::{GitBackend, default_git_backend};
use crate::model::RepoContext;
use crate::path::{RepoRelPath, kernel_path_components};
use crate::subcommands::{
    CopyArgs, InfoArgs, ListArgs, ValidateArgs, run_copy_with_context, run_info_with_context,
    run_list_with_context, run_validate_with_context,
};

/// waft — copy .worktreeinclude-selected ignored files between Git worktrees.
#[derive(Debug, Parser)]
#[command(name = "waft", version, about, long_about = None)]
pub struct Cli {
    /// Source (main) worktree path.
    #[arg(long, global = true)]
    pub source: Option<PathBuf>,

    /// Destination (linked) worktree path.
    #[arg(long, global = true)]
    pub dest: Option<PathBuf>,

    /// Operate as if started in PATH.
    #[arg(short = 'C', global = true, value_name = "PATH")]
    pub directory: Option<PathBuf>,

    /// Suppress non-error output.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Increase output verbosity.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Compat profile preset (claude|git|wt).
    #[arg(long, global = true, value_name = "PROFILE")]
    pub compat_profile: Option<CompatProfile>,

    /// Behavior when no .worktreeinclude file exists.
    #[arg(long, global = true, value_name = "MODE")]
    pub when_missing_worktreeinclude: Option<WhenMissingWorktreeinclude>,

    /// Worktreeinclude matcher semantics.
    #[arg(long, global = true, value_name = "MODE")]
    pub worktreeinclude_semantics: Option<WorktreeincludeSemantics>,

    /// Symlinked .worktreeinclude policy.
    #[arg(long, global = true, value_name = "POLICY")]
    pub worktreeinclude_symlink_policy: Option<SymlinkPolicy>,

    /// Built-in exclude set.
    #[arg(long, global = true, value_name = "SET")]
    pub builtin_exclude_set: Option<BuiltinExcludeSet>,

    /// Extra exclude glob (repeatable).
    #[arg(long = "extra-exclude", global = true, value_name = "GLOB")]
    pub extra_exclude: Vec<String>,

    /// Replace extra excludes inherited from lower-precedence layers.
    #[arg(long, global = true)]
    pub replace_extra_excludes: bool,

    /// File copy strategy (auto|simple-copy|cow-copy).
    #[arg(long, global = true, value_name = "STRATEGY")]
    pub copy_strategy: Option<CopyStrategy>,

    /// Path to an explicit config file (overrides user config discovery).
    #[arg(long, global = true, value_name = "PATH", conflicts_with = "isolated")]
    pub config: Option<PathBuf>,

    /// Ignore user config and WAFT_* policy environment variables.
    #[arg(long, global = true)]
    pub isolated: bool,

    /// Subcommand to run. If omitted, defaults to `copy`.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Available subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Copy eligible files from source to destination (default command).
    Copy(CopyArgs),

    /// List eligible files.
    List(ListArgs),

    /// Show detailed status for one or more paths.
    Info(InfoArgs),

    /// Validate .worktreeinclude and Git ignore files.
    Validate(ValidateArgs),
}

impl Cli {
    /// Build a [`ConfigLayer`] from this CLI's flag-provided values.
    pub fn cli_layer(&self) -> ConfigLayer {
        ConfigLayer {
            profile: self.compat_profile,
            when_missing: self.when_missing_worktreeinclude,
            semantics: self.worktreeinclude_semantics,
            symlink_policy: self.worktreeinclude_symlink_policy,
            builtin_exclude_set: self.builtin_exclude_set,
            extra_excludes: self.extra_exclude.clone(),
            // Verified: `Some(true)` or `None`, never `Some(false)`, so the
            // CLI layer cannot cancel a lower layer's replace.
            replace_extra_excludes: cli_policy::cli_replace_extra_excludes(
                self.replace_extra_excludes,
            ),
            copy_strategy: self.copy_strategy,
        }
    }

    /// Resolve the active [`ResolvedPolicy`] for this command's source
    /// repository.
    ///
    /// Project config discovery is performed only after Git context
    /// resolution. This keeps an unrelated process working directory, or a
    /// destination worktree on another branch, from changing source
    /// selection.
    pub fn resolve_policy(&self) -> Result<ResolvedPolicy> {
        let git = default_git_backend()?;
        let ctx = context::resolve_context(
            git.as_ref(),
            self.source.as_deref(),
            self.dest.as_deref(),
            self.directory.as_deref(),
            self.command_kind(),
        )?;
        self.resolve_policy_for_context(&ctx, git.as_ref())
    }

    /// Return the effective working directory after applying `-C`.
    pub(crate) fn effective_directory(&self) -> Result<PathBuf> {
        let path = match self.directory.as_deref() {
            Some(dir) if dir.is_absolute() => dir.to_path_buf(),
            Some(dir) => std::env::current_dir()
                .map_err(|e| Error::Io {
                    context: "getting current directory".to_string(),
                    source: e,
                })?
                .join(dir),
            None => std::env::current_dir().map_err(|e| Error::Io {
                context: "getting current directory".to_string(),
                source: e,
            })?,
        };

        // Context resolution requires this path to exist. Canonicalizing here
        // also lets us compare `-C` paths containing `..` or symlinked parent
        // components with Git's normalized worktree paths.
        Ok(std::fs::canonicalize(&path).unwrap_or(path))
    }

    /// Normalize a user-supplied path against the invocation directory and
    /// map a linked-worktree path to the equivalent source-worktree path.
    pub(crate) fn normalize_source_path(
        &self,
        path: &Path,
        ctx: &RepoContext,
    ) -> Result<RepoRelPath> {
        let input = if path.is_absolute() {
            let normalized = canonicalize_parent(path);
            self.map_worktree_path_to_source(&normalized, ctx)
                .unwrap_or(normalized)
        } else {
            let invocation_dir = self.effective_directory()?;
            self.map_worktree_path_to_source(&invocation_dir, ctx)
                .unwrap_or(invocation_dir)
                .join(path)
        };
        RepoRelPath::normalize(&input, &ctx.source_root)
    }

    fn command_kind(&self) -> CommandKind {
        let subcommand = match &self.command {
            None => cli_policy::Subcommand::None,
            Some(Command::Copy(_)) => cli_policy::Subcommand::Copy,
            Some(Command::List(_)) => cli_policy::Subcommand::List,
            Some(Command::Info(_)) => cli_policy::Subcommand::Info,
            Some(Command::Validate(_)) => cli_policy::Subcommand::Validate,
        };
        // Verified: no subcommand resolves the context for `copy`, matching
        // `dispatch`, which runs `copy` by default.
        context::command_kind_from_kernel(cli_policy::command_kind(subcommand))
    }

    fn resolve_policy_for_context(
        &self,
        ctx: &RepoContext,
        git: &dyn GitBackend,
    ) -> Result<ResolvedPolicy> {
        let invocation_dir = self.effective_directory()?;

        // Verified precedence: `--isolated`, then `--config`, then a non-empty
        // Unicode `WAFT_CONFIG_PATH`, then the default location. The kernel
        // reads the variable itself, through `ProcessConfigPath`, and proves it
        // does so exactly once when neither flag decides and never otherwise.
        let user = match cli_policy::user_config_source(
            self.isolated,
            self.config.as_deref(),
            &mut ProcessConfigPath,
        ) {
            cli_policy::UserConfigSource::Skipped => None,
            cli_policy::UserConfigSource::Flag(p) => {
                load_user_layer(Some(resolve_from(p, &invocation_dir).as_path()))?
            }
            cli_policy::UserConfigSource::Env(p) => {
                load_user_layer(Some(resolve_from(Path::new(&p), &invocation_dir).as_path()))?
            }
            cli_policy::UserConfigSource::DefaultLocation => {
                load_user_layer(user_config_path().as_deref())?
            }
        };

        let project_dir = self.source_view_directory(ctx)?;
        let gitlinks = git.gitlinks(&ctx.source_root)?;
        let project_paths = discover_project_configs_in_repo(
            &ctx.source_root,
            project_dir.as_path(),
            &gitlinks,
            ctx.core_ignore_case,
        );
        let project = load_project_layers(&project_paths)?;
        let env = if cli_policy::policy_env_layer_applies(self.isolated) {
            layer_from_env()?
        } else {
            ConfigLayer::default()
        };
        let cli = self.cli_layer();

        let inputs = PolicyResolutionInputs {
            defaults: ConfigLayer::default(),
            user,
            project,
            env,
            cli,
        };
        Ok(inputs.resolve())
    }

    /// Dispatch the parsed CLI to the appropriate command handler.
    pub fn dispatch(self) -> Result<()> {
        let git = default_git_backend()?;
        let ctx = context::resolve_context(
            git.as_ref(),
            self.source.as_deref(),
            self.dest.as_deref(),
            self.directory.as_deref(),
            self.command_kind(),
        )?;
        let policy = self.resolve_policy_for_context(&ctx, git.as_ref())?;

        match &self.command {
            None => {
                let args = CopyArgs {
                    dry_run: false,
                    overwrite: false,
                };
                run_copy_with_context(&self, &policy, &args, &ctx, git.as_ref())
            }
            Some(Command::Copy(args)) => {
                run_copy_with_context(&self, &policy, args, &ctx, git.as_ref())
            }
            Some(Command::List(args)) => {
                run_list_with_context(&self, &policy, args, &ctx, git.as_ref())
            }
            Some(Command::Info(args)) => {
                run_info_with_context(&self, &policy, args, &ctx, git.as_ref())
            }
            Some(Command::Validate(args)) => {
                run_validate_with_context(&self, &policy, args, &ctx, git.as_ref())
            }
        }
    }

    fn source_view_directory(&self, ctx: &RepoContext) -> Result<PathBuf> {
        let invocation_dir = self.effective_directory()?;
        Ok(self
            .map_worktree_path_to_source(&invocation_dir, ctx)
            .unwrap_or_else(|| ctx.source_root.clone()))
    }

    fn map_worktree_path_to_source(&self, path: &Path, ctx: &RepoContext) -> Option<PathBuf> {
        let path = crate::path::without_windows_verbatim_prefix(path);
        let worktrees: Vec<PathBuf> = ctx
            .known_worktrees
            .iter()
            .map(|worktree| crate::path::without_windows_verbatim_prefix(worktree))
            .collect();
        // Both sides are lowered with the conversion `RepoRelPath::normalize`
        // uses, so the kernel compares components with the path model's
        // verified `comp_eq` (`Component` equality, which `strip_prefix`
        // compares).
        let worktree_components: Vec<Vec<Comp<'_>>> = worktrees
            .iter()
            .map(|worktree| kernel_path_components(worktree))
            .collect();
        // Worktrees should not overlap, but choosing the deepest match makes
        // the mapping deterministic if they do. The verified kernel picks it
        // (the last of equally deep matches, as `max_by_key` did) and proves
        // the stripped components do not depend on the worktree order.
        let chosen = worktree_map::deepest_worktree_prefix(
            &kernel_path_components(&path),
            &worktree_components,
        )?;
        // The kernel proved `worktrees[chosen]`'s components prefix `path`'s,
        // so `strip_prefix` succeeds; it yields the remainder as std spells it.
        let relative = path.strip_prefix(&worktrees[chosen]).ok()?;
        Some(ctx.source_root.join(relative))
    }
}

/// Process-environment reader of `WAFT_CONFIG_PATH` for the verified
/// user-config choice, which decides whether to call it.
struct ProcessConfigPath;

impl cli_policy::ConfigPathReader<String> for ProcessConfigPath {
    /// Classify `std::env::var("WAFT_CONFIG_PATH")` (trusted: exhaustive over
    /// `VarError`, with `empty` equal to `value.is_empty()`).
    fn read_config_path_var(&mut self) -> cli_policy::ConfigPathVar<String> {
        match std::env::var("WAFT_CONFIG_PATH") {
            Ok(value) => {
                let empty = value.is_empty();
                cli_policy::ConfigPathVar::Set { value, empty }
            }
            Err(std::env::VarError::NotPresent) => cli_policy::ConfigPathVar::Unset,
            Err(std::env::VarError::NotUnicode(_)) => cli_policy::ConfigPathVar::NotUnicode,
        }
    }
}

fn resolve_from(path: &Path, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn canonicalize_parent(path: &Path) -> PathBuf {
    let Some(parent) = path.parent() else {
        return path.to_path_buf();
    };
    let Some(file_name) = path.file_name() else {
        return path.to_path_buf();
    };

    let mut cursor = parent;
    let mut missing = Vec::new();
    loop {
        if let Ok(canonical) = std::fs::canonicalize(cursor) {
            let mut result = canonical;
            for component in missing.iter().rev() {
                result.push(component);
            }
            result.push(file_name);
            return result;
        }
        let Some(name) = cursor.file_name() else {
            return path.to_path_buf();
        };
        missing.push(name.to_os_string());
        let Some(next) = cursor.parent() else {
            return path.to_path_buf();
        };
        cursor = next;
    }
}

#[cfg(test)]
mod mapping_tests {
    use super::*;

    /// The mapping as written before the verified kernel, kept as a test
    /// oracle for differential checks.
    fn reference_map(path: &Path, ctx: &RepoContext) -> Option<PathBuf> {
        let path = crate::path::without_windows_verbatim_prefix(path);
        ctx.known_worktrees
            .iter()
            .filter_map(|worktree| {
                let worktree = crate::path::without_windows_verbatim_prefix(worktree);
                path.strip_prefix(&worktree)
                    .ok()
                    .map(|relative| (worktree.components().count(), relative.to_path_buf()))
            })
            .max_by_key(|(depth, _)| *depth)
            .map(|(_, relative)| ctx.source_root.join(relative))
    }

    fn context(known: &[&str]) -> RepoContext {
        RepoContext {
            source_root: PathBuf::from("/src/main"),
            dest_root: None,
            main_worktree: PathBuf::from("/src/main"),
            known_worktrees: known.iter().map(PathBuf::from).collect(),
            core_ignore_case: false,
        }
    }

    #[test]
    fn mapping_matches_reference_on_overlapping_and_reordered_worktrees() {
        let cli = Cli::parse_from(["waft"]);
        let lists: &[&[&str]] = &[
            &[],
            &["/src/main", "/wt/linked"],
            &["/wt/linked", "/src/main"],
            &["/src/main", "/wt", "/wt/linked", "/wt/linked/nested"],
            &["/wt/linked/nested", "/wt/linked", "/wt", "/src/main"],
            &["/wt/linked/", "/wt/./linked", "/wt//linked", "/src/main"],
            &["/src/main", "/src/main", "/wt/linked", "/wt/linked"],
            &["/", "/src/main"],
            &["wt/linked", "/src/main"],
        ];
        let paths = [
            "/wt/linked",
            "/wt/linked/",
            "/wt/linked/a/b.txt",
            "/wt/linked/./a//b.txt",
            "/wt/linked/nested/x",
            "/wt/linkedx/a",
            "/wt/other",
            "/src/main/sub/file",
            "/src/main",
            "/elsewhere/file",
            "wt/linked/rel",
            "/wt/linked/../escape",
        ];
        for known in lists {
            let ctx = context(known);
            for path in paths {
                let path = Path::new(path);
                assert_eq!(
                    cli.map_worktree_path_to_source(path, &ctx),
                    reference_map(path, &ctx),
                    "path {path:?} with worktrees {known:?}"
                );
            }
        }
    }

    /// The kernel's component prefix over the shared lowering
    /// (`kernel_path_components`) is exactly `Path::strip_prefix`, which the
    /// mapping re-runs on the chosen worktree.
    #[test]
    fn kernel_component_prefix_agrees_with_strip_prefix() {
        let samples = [
            "/a/b", "/a/b/", "/a/./b", "a/b", "./a/b", "../a", "/", "", ".", "..", "/a/b/..",
            "/a/bb", "/ab", "a", "/a//b",
        ];
        for left in samples {
            for right in samples {
                let left_components = kernel_path_components(Path::new(left));
                let right_components = kernel_path_components(Path::new(right));
                assert_eq!(
                    waft_verified::path::containment::component_prefix(
                        &right_components,
                        &left_components
                    ),
                    Path::new(left).strip_prefix(right).is_ok(),
                    "{left:?} strip {right:?}"
                );
            }
        }
    }

    /// Windows spellings, including prefix components (drive letters in
    /// either case, UNC and device prefixes, verbatim forms, drive-relative
    /// paths), map exactly as the pre-kernel `strip_prefix` oracle does. On
    /// other platforms these are ordinary names with backslashes.
    #[test]
    fn mapping_matches_reference_on_windows_spellings() {
        let cli = Cli::parse_from(["waft"]);
        let lists: &[&[&str]] = &[
            &[r"C:\wt\linked", r"C:\src\main"],
            &[r"c:\wt\linked", r"C:\wt", r"C:\src\main"],
            &[r"\\server\share\wt", r"\\server\share"],
            &[r"\\SERVER\share\wt", r"\\server/share\wt\nested"],
            &[r"\\?\C:\wt\linked", r"\\?\UNC\server\share\wt"],
            &[r"\\.\COM1", r"C:wt", r"C:"],
            &[r"\\?\D:\wt", r"D:\wt", r"\\?\D:"],
        ];
        let paths = [
            r"C:\wt\linked\a\b.txt",
            r"c:\wt\linked",
            r"C:\wt\linkedx",
            r"C:/wt/linked/a",
            r"D:\wt\linked\a",
            r"\\server\share\wt\a",
            r"\\server\share",
            r"\\server/share\wt\nested\x",
            r"\\SERVER\share\wt\a",
            r"\\?\C:\wt\linked\nested",
            r"\\?\UNC\server\share\wt\a",
            r"\\.\COM1\x",
            r"C:wt\x",
            r"C:x",
            r"\\?\D:\wt\x",
            r"D:\wt\x",
        ];
        for known in lists {
            let ctx = context(known);
            for path in paths {
                let path = Path::new(path);
                assert_eq!(
                    cli.map_worktree_path_to_source(path, &ctx),
                    reference_map(path, &ctx),
                    "path {path:?} with worktrees {known:?}"
                );
            }
        }
    }

    #[test]
    fn command_kind_defaults_to_copy_and_follows_subcommands() {
        let cases: [(&[&str], CommandKind); 5] = [
            (&["waft"], CommandKind::Copy),
            (&["waft", "copy"], CommandKind::Copy),
            (&["waft", "list"], CommandKind::List),
            (&["waft", "info", "x"], CommandKind::Info),
            (&["waft", "validate"], CommandKind::Validate),
        ];
        for (args, expected) in cases {
            assert_eq!(Cli::parse_from(args).command_kind(), expected, "{args:?}");
        }
    }

    /// Answers `WAFT_CONFIG_PATH` reads from a fixed case and counts them.
    struct ScriptedConfigPath {
        case: usize,
        reads: usize,
    }

    impl cli_policy::ConfigPathReader<String> for ScriptedConfigPath {
        fn read_config_path_var(&mut self) -> cli_policy::ConfigPathVar<String> {
            self.reads += 1;
            match self.case {
                0 => cli_policy::ConfigPathVar::Unset,
                1 => cli_policy::ConfigPathVar::NotUnicode,
                2 => cli_policy::ConfigPathVar::Set {
                    value: String::new(),
                    empty: true,
                },
                _ => cli_policy::ConfigPathVar::Set {
                    value: "env.toml".to_string(),
                    empty: false,
                },
            }
        }
    }

    /// The user-config choice follows `--isolated` > `--config` >
    /// non-empty Unicode `WAFT_CONFIG_PATH` > default, and reads the
    /// variable exactly once when neither flag decides, never otherwise.
    #[test]
    fn user_config_source_reads_variable_only_when_no_flag_decides() {
        for isolated in [false, true] {
            for flag in [None, Some(Path::new("flag.toml"))] {
                for case in 0..4 {
                    let mut reader = ScriptedConfigPath { case, reads: 0 };
                    let source = cli_policy::user_config_source(isolated, flag, &mut reader);
                    let got = match source {
                        cli_policy::UserConfigSource::Skipped => "skipped".to_string(),
                        cli_policy::UserConfigSource::Flag(p) => format!("flag {}", p.display()),
                        cli_policy::UserConfigSource::Env(s) => format!("env {s}"),
                        cli_policy::UserConfigSource::DefaultLocation => "default".to_string(),
                    };
                    let expected = if isolated {
                        "skipped"
                    } else if flag.is_some() {
                        "flag flag.toml"
                    } else if case == 3 {
                        "env env.toml"
                    } else {
                        "default"
                    };
                    let context = format!("isolated {isolated} flag {flag:?} case {case}");
                    assert_eq!(got, expected, "{context}");
                    assert_eq!(
                        reader.reads,
                        usize::from(!isolated && flag.is_none()),
                        "{context}"
                    );
                }
            }
        }
    }

    #[test]
    fn cli_layer_replace_flag_is_true_or_absent() {
        assert_eq!(
            Cli::parse_from(["waft"]).cli_layer().replace_extra_excludes,
            None
        );
        assert_eq!(
            Cli::parse_from(["waft", "--replace-extra-excludes"])
                .cli_layer()
                .replace_extra_excludes,
            Some(true)
        );
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn maps_verbatim_linked_worktree_path_to_source() {
        let cli = Cli::parse_from(["waft"]);
        let ctx = RepoContext {
            source_root: PathBuf::from(r"C:\repos\main"),
            dest_root: None,
            main_worktree: PathBuf::from(r"C:\repos\main"),
            known_worktrees: vec![
                PathBuf::from(r"C:\repos\main"),
                PathBuf::from(r"C:\worktrees\linked"),
            ],
            core_ignore_case: true,
        };

        let mapped =
            cli.map_worktree_path_to_source(Path::new(r"\\?\C:\worktrees\linked\nested"), &ctx);

        assert_eq!(mapped, Some(PathBuf::from(r"C:\repos\main\nested")));
    }
}
