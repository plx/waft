//! CLI argument parsing and command dispatch.

use clap::Parser;
use std::path::PathBuf;

use crate::error::{Error, Result};

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

fn run_list(_cli: &Cli, _args: &ListArgs) -> Result<()> {
    Err(Error::NotImplemented {
        command: "list".to_string(),
    })
}

fn run_info(_cli: &Cli, _args: &InfoArgs) -> Result<()> {
    Err(Error::NotImplemented {
        command: "info".to_string(),
    })
}

fn run_validate(_cli: &Cli, _args: &ValidateArgs) -> Result<()> {
    Err(Error::NotImplemented {
        command: "validate".to_string(),
    })
}
