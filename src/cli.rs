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

fn run_info(cli: &Cli, args: &InfoArgs) -> Result<()> {
    let git = GitCli::new();

    let ctx = context::resolve_context(
        &git,
        cli.source.as_deref(),
        cli.dest.as_deref(),
        cli.directory.as_deref(),
        CommandKind::Info,
    )?;

    // Normalize all input paths to repo-relative
    let mut rel_paths = Vec::new();
    for path in &args.paths {
        let rp = crate::path::RepoRelPath::normalize(path, &ctx.source_root)?;
        rel_paths.push(rp);
    }

    // Check tracked status for all paths
    let tracked_set = git.tracked_paths(&ctx.source_root, &rel_paths)?;

    // Check ignore status for untracked paths
    let untracked: Vec<_> = rel_paths
        .iter()
        .filter(|p| !tracked_set.contains(*p))
        .cloned()
        .collect();
    let ignore_results = git.check_ignore(&ctx.source_root, &untracked)?;

    // Build a map from path to ignore info
    let mut ignore_map = std::collections::HashMap::new();
    for record in &ignore_results {
        ignore_map.insert(record.path.as_str().to_string(), record);
    }

    // Print info for each path
    for rp in &rel_paths {
        let abs_path = rp.to_path(&ctx.source_root);
        let source_exists = abs_path.exists();
        let source_kind = if !source_exists {
            "missing"
        } else if abs_path.is_file() {
            "file"
        } else if abs_path.is_dir() {
            "directory"
        } else if abs_path.is_symlink() {
            "symlink"
        } else {
            "other"
        };

        let is_tracked = tracked_set.contains(rp);

        // Git ignore status
        let gitignore_str = if is_tracked {
            "tracked".to_string()
        } else if let Some(record) = ignore_map.get(rp.as_str()) {
            if let Some(ref info) = record.match_info {
                format!(
                    "ignored ({}:{}: {})",
                    info.source_file.display(),
                    info.line,
                    info.pattern
                )
            } else {
                "not ignored".to_string()
            }
        } else {
            "not ignored".to_string()
        };

        // Worktreeinclude status
        let wti = worktreeinclude::explain(
            &ctx.source_root,
            rp.as_str(),
            abs_path.is_dir(),
            ctx.core_ignore_case,
        );
        let wti_str = match &wti {
            crate::model::WorktreeincludeStatus::Included {
                file,
                line,
                pattern,
            } => format!("included ({}:{}: {})", file.display(), line, pattern),
            crate::model::WorktreeincludeStatus::ExcludedByNegation {
                file,
                line,
                pattern,
            } => format!("excluded ({}:{}: {})", file.display(), line, pattern),
            crate::model::WorktreeincludeStatus::NoMatch => "no match".to_string(),
        };

        // Eligibility
        let is_ignored = !is_tracked
            && ignore_map
                .get(rp.as_str())
                .map(|r| r.match_info.is_some())
                .unwrap_or(false);
        let is_included = matches!(wti, crate::model::WorktreeincludeStatus::Included { .. });
        let eligible =
            source_exists && abs_path.is_file() && !is_tracked && is_ignored && is_included;

        println!("path: {rp}");
        println!(
            "source_exists: {}",
            if source_exists { "yes" } else { "no" }
        );
        println!("source_kind: {source_kind}");
        println!("tracked: {}", if is_tracked { "yes" } else { "no" });
        println!("gitignore: {gitignore_str}");
        println!("worktreeinclude: {wti_str}");
        println!("eligible_to_copy: {}", if eligible { "yes" } else { "no" });

        // Destination info if available
        if let Some(ref dest_root) = ctx.dest_root {
            let dest_path = rp.to_path(dest_root);
            if dest_path.exists() {
                if dest_path.is_file() {
                    // Check if identical
                    if source_exists && abs_path.is_file() {
                        let src_bytes = std::fs::read(&abs_path).ok();
                        let dst_bytes = std::fs::read(&dest_path).ok();
                        if src_bytes == dst_bytes {
                            println!("destination: up-to-date");
                            println!("planned_action: no-op");
                        } else {
                            println!("destination: exists (differs)");
                            println!("planned_action: skip (conflict)");
                        }
                    } else {
                        println!("destination: exists");
                    }
                } else {
                    println!("destination: exists (not a file)");
                    println!("planned_action: skip (type conflict)");
                }
            } else {
                println!("destination: missing");
                if eligible {
                    println!("planned_action: copy");
                }
            }
        }
        println!();
    }

    Ok(())
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
