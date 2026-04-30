//! CLI argument parsing and command dispatch.

use clap::Parser;
use std::path::PathBuf;

use crate::config::{
    BuiltinExcludeSet, CompatProfile, ConfigLayer, CopyStrategy, PolicyResolutionInputs,
    ResolvedPolicy, SymlinkPolicy, WhenMissingWorktreeinclude, WorktreeincludeSemantics,
    discover_project_configs, layer_from_env, load_project_layers, load_user_layer,
    user_config_path,
};
use crate::error::{Error, Result};
use crate::subcommands::{
    CopyArgs, InfoArgs, ListArgs, ValidateArgs, run_copy, run_info, run_list, run_validate,
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
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

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

    /// Resolve the active [`ResolvedPolicy`] from CLI flags, env vars, and
    /// discovered config files.
    pub fn resolve_policy(&self) -> Result<ResolvedPolicy> {
        let cwd = match self.directory.as_deref() {
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

        let user_path = if let Some(p) = self.config.as_deref() {
            Some(p.to_path_buf())
        } else if let Ok(p) = std::env::var("WAFT_CONFIG_PATH") {
            if p.is_empty() {
                user_config_path()
            } else {
                Some(PathBuf::from(p))
            }
        } else {
            user_config_path()
        };

        let user = load_user_layer(user_path.as_deref())?;
        let project_paths = discover_project_configs(&cwd);
        let project = load_project_layers(&project_paths)?;
        let env = layer_from_env()?;
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
        let policy = self.resolve_policy()?;

        match self.command {
            None => {
                let args = CopyArgs {
                    dry_run: false,
                    overwrite: false,
                };
                run_copy(&self, &policy, &args)
            }
            Some(Command::Copy(ref args)) => run_copy(&self, &policy, args),
            Some(Command::List(ref args)) => run_list(&self, &policy, args),
            Some(Command::Info(ref args)) => run_info(&self, &policy, args),
            Some(Command::Validate(ref args)) => run_validate(&self, &policy, args),
        }
    }
}
