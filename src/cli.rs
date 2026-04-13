//! CLI argument parsing and command dispatch.

use clap::Parser;
use std::path::PathBuf;

use crate::error::Result;
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
