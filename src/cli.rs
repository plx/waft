//! CLI argument parsing and command dispatch.

use clap::Parser;
use std::path::PathBuf;

use crate::context::{self, CommandKind};
use crate::error::{Error, Result};
use crate::git::{GitBackend, GitCli};
use crate::model::ValidationSeverity;
use crate::validate;
use crate::worktreeinclude;

/// wiff — copy .worktreeinclude-selected ignored files between Git worktrees.
#[derive(Debug, Parser)]
#[command(name = "wiff", version, about, long_about = None)]
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

/// Arguments for the copy command.
#[derive(Debug, clap::Args)]
pub struct CopyArgs {
    /// Show what would be done without making changes.
    #[arg(short = 'n', long)]
    pub dry_run: bool,

    /// Allow overwriting existing untracked destination files.
    #[arg(long)]
    pub overwrite: bool,
}

/// Arguments for the list command.
#[derive(Debug, clap::Args)]
pub struct ListArgs {}

/// Arguments for the info command.
#[derive(Debug, clap::Args)]
pub struct InfoArgs {
    /// Paths to inspect.
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,
}

/// Arguments for the validate command.
#[derive(Debug, clap::Args)]
pub struct ValidateArgs {}

impl Cli {
    /// Dispatch the parsed CLI to the appropriate command handler.
    pub fn dispatch(self) -> Result<()> {
        match self.command {
            None => {
                // Default to copy with default args
                let args = CopyArgs {
                    dry_run: false,
                    overwrite: false,
                };
                run_copy(&self, &args)
            }
            Some(Command::Copy(ref args)) => run_copy(&self, args),
            Some(Command::List(ref args)) => run_list(&self, args),
            Some(Command::Info(ref args)) => run_info(&self, args),
            Some(Command::Validate(ref args)) => run_validate(&self, args),
        }
    }
}

fn run_copy(_cli: &Cli, _args: &CopyArgs) -> Result<()> {
    Err(Error::NotImplemented {
        command: "copy".to_string(),
    })
}

fn run_list(cli: &Cli, _args: &ListArgs) -> Result<()> {
    let git = GitCli::new();

    // Resolve context
    let ctx = context::resolve_context(
        &git,
        cli.source.as_deref(),
        cli.dest.as_deref(),
        cli.directory.as_deref(),
        CommandKind::List,
    )?;

    // Validate
    let report = validate::validate(&ctx);
    if report.has_errors() {
        for issue in &report.issues {
            if matches!(issue.severity, ValidationSeverity::Error) {
                eprintln!("error: {}: {}", issue.file.display(), issue.message);
            }
        }
        return Err(Error::Validation {
            error_count: report.error_count(),
        });
    }

    // Print warnings
    if !cli.quiet {
        for issue in &report.issues {
            if matches!(issue.severity, ValidationSeverity::Warning) {
                eprintln!("warning: {}: {}", issue.file.display(), issue.message);
            }
        }
    }

    // Enumerate worktreeinclude candidates
    let candidates = git.list_worktreeinclude_candidates(&ctx.source_root)?;

    if candidates.is_empty() {
        return Ok(());
    }

    // Batch check-ignore to find which candidates are actually git-ignored
    let ignore_results = git.check_ignore(&ctx.source_root, &candidates)?;

    // Keep only paths with real ignore hits
    let mut eligible: Vec<String> = ignore_results
        .into_iter()
        .filter(|r| r.match_info.is_some())
        .map(|r| r.path.as_str().to_string())
        .collect();

    // Sort lexically
    eligible.sort();

    // Output
    for path in &eligible {
        if cli.verbose > 0 {
            // Verbose mode: include worktreeinclude explanation
            let wti = worktreeinclude::explain(&ctx.source_root, path, false, ctx.core_ignore_case);
            println!("{path}\t{wti:?}");
        } else {
            println!("{path}");
        }
    }

    Ok(())
}

fn run_info(_cli: &Cli, _args: &InfoArgs) -> Result<()> {
    Err(Error::NotImplemented {
        command: "info".to_string(),
    })
}

fn run_validate(cli: &Cli, _args: &ValidateArgs) -> Result<()> {
    let git = GitCli::new();

    let ctx = context::resolve_context(
        &git,
        cli.source.as_deref(),
        cli.dest.as_deref(),
        cli.directory.as_deref(),
        CommandKind::Validate,
    )?;

    let report = validate::validate(&ctx);

    for issue in &report.issues {
        let severity = match issue.severity {
            ValidationSeverity::Warning => "warning",
            ValidationSeverity::Error => "error",
        };
        let location = if let Some(line) = issue.line {
            format!("{}:{}", issue.file.display(), line)
        } else {
            issue.file.display().to_string()
        };
        eprintln!("{severity}: {location}: {}", issue.message);
    }

    if report.has_errors() {
        Err(Error::Validation {
            error_count: report.error_count(),
        })
    } else {
        if !cli.quiet {
            eprintln!("validation passed");
        }
        Ok(())
    }
}
