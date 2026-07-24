//! CLI argument parsing and command dispatch.

use clap::Parser;
use std::path::{Path, PathBuf};

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
use crate::path::RepoRelPath;
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
            replace_extra_excludes: if self.replace_extra_excludes {
                Some(true)
            } else {
                None
            },
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
        let git = default_git_backend();
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
        match &self.command {
            None | Some(Command::Copy(_)) => CommandKind::Copy,
            Some(Command::List(_)) => CommandKind::List,
            Some(Command::Info(_)) => CommandKind::Info,
            Some(Command::Validate(_)) => CommandKind::Validate,
        }
    }

    fn resolve_policy_for_context(
        &self,
        ctx: &RepoContext,
        git: &dyn GitBackend,
    ) -> Result<ResolvedPolicy> {
        let invocation_dir = self.effective_directory()?;

        let user = if self.isolated {
            None
        } else {
            let user_path = if let Some(p) = self.config.as_deref() {
                Some(resolve_from(p, &invocation_dir))
            } else if let Ok(p) = std::env::var("WAFT_CONFIG_PATH") {
                if p.is_empty() {
                    user_config_path()
                } else {
                    Some(resolve_from(Path::new(&p), &invocation_dir))
                }
            } else {
                user_config_path()
            };
            load_user_layer(user_path.as_deref())?
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
        let env = if self.isolated {
            ConfigLayer::default()
        } else {
            layer_from_env()?
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
        let git = default_git_backend();
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
        ctx.known_worktrees
            .iter()
            .filter_map(|worktree| {
                path.strip_prefix(worktree)
                    .ok()
                    .map(|relative| (worktree.components().count(), relative))
            })
            // Worktrees should not overlap, but choosing the deepest match
            // makes the mapping deterministic if they do.
            .max_by_key(|(depth, _)| *depth)
            .map(|(_, relative)| ctx.source_root.join(relative))
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
