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

fn run_copy(cli: &Cli, args: &CopyArgs) -> Result<()> {
    let git = GitCli::new();
    let fs = crate::fs::RealFs;

    // Resolve context (copy requires a destination)
    let ctx = context::resolve_context(
        &git,
        cli.source.as_deref(),
        cli.dest.as_deref(),
        cli.directory.as_deref(),
        CommandKind::Copy,
    )?;

    // Validate
    let report = validate::validate(&ctx, &git);
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

    // Enumerate eligible files
    let candidates = git.list_worktreeinclude_candidates(&ctx.source_root)?;
    if candidates.is_empty() {
        if !cli.quiet {
            eprintln!("no eligible files found");
        }
        return Ok(());
    }

    let ignore_results = git.check_ignore(&ctx.source_root, &candidates)?;
    let eligible: Vec<_> = ignore_results
        .into_iter()
        .filter(|r| r.match_info.is_some())
        .map(|r| r.path)
        .collect();

    if eligible.is_empty() {
        if !cli.quiet {
            eprintln!("no eligible files found");
        }
        return Ok(());
    }

    // Build plan
    let plan = crate::planner::plan(
        &ctx,
        report,
        eligible,
        &git,
        &fs,
        args.overwrite,
        args.dry_run,
    )?;

    if args.dry_run {
        crate::planner::render_dry_run(&plan);
        return Ok(());
    }

    // Execute
    let copy_report = crate::executor::execute(&plan, &fs);
    crate::executor::render_report(&copy_report, cli.quiet);

    if let Some((failed, total)) = crate::executor::report_has_failures(&copy_report) {
        return Err(Error::CopyFailed { failed, total });
    }

    Ok(())
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
    let report = validate::validate(&ctx, &git);
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

    // Keep only records with real ignore hits
    let mut eligible: Vec<_> = ignore_results
        .into_iter()
        .filter(|r| r.match_info.is_some())
        .collect();

    // Sort lexically by path
    eligible.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));

    // Pre-compute destination classification data if verbose + dest available
    let verbose = cli.verbose > 0;
    let dest_tracked_set = if verbose {
        if let Some(ref dest_root) = ctx.dest_root {
            let rel_paths: Vec<_> = eligible
                .iter()
                .map(|r| r.path.clone())
                .collect();
            git.tracked_paths(dest_root, &rel_paths)?
        } else {
            std::collections::HashSet::new()
        }
    } else {
        std::collections::HashSet::new()
    };
    let fs = crate::fs::RealFs;

    // Output
    for record in &eligible {
        let path = record.path.as_str();
        if verbose {
            let abs_path = record.path.to_path(&ctx.source_root);

            // Source size
            let size = std::fs::metadata(&abs_path)
                .map(|m| m.len())
                .unwrap_or(0);

            // Git ignore info
            let gitignore_str = if let Some(ref info) = record.match_info {
                format!(
                    "ignored ({}:{}: {})",
                    info.source_file.display(),
                    info.line,
                    info.pattern
                )
            } else {
                "not ignored".to_string()
            };

            // Worktreeinclude info
            let wti = worktreeinclude::explain(
                &ctx.source_root,
                path,
                false,
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

            println!("{path}\tsize: {size}\tgitignore: {gitignore_str}\tworktreeinclude: {wti_str}");

            // Predicted action (only when --dest is available)
            if let Some(ref dest_root) = ctx.dest_root {
                if abs_path.is_symlink() || !abs_path.is_file() {
                    // Planner skips non-regular-file and symlink sources
                    if abs_path.exists() {
                        println!("\taction: skip (unsupported source type)");
                    }
                } else {
                    let dest_path = record.path.to_path(dest_root);
                    let state = crate::planner::classify_destination(
                        &record.path,
                        &abs_path,
                        &dest_path,
                        &dest_tracked_set,
                        &fs,
                    );
                    let action_str = match state {
                        crate::model::DestinationState::Missing => "copy",
                        crate::model::DestinationState::UpToDate => "no-op",
                        crate::model::DestinationState::UntrackedConflict => {
                            "skip (untracked conflict)"
                        }
                        crate::model::DestinationState::TrackedConflict => {
                            "skip (tracked conflict)"
                        }
                        crate::model::DestinationState::TypeConflict => "skip (type conflict)",
                        crate::model::DestinationState::UnsafePath => "skip (unsafe path)",
                    };
                    println!("\taction: {action_str}");
                }
            }
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

    // Query destination trackedness if destination is known
    let fs = crate::fs::RealFs;
    let dest_tracked_set = if let Some(ref dest_root) = ctx.dest_root {
        git.tracked_paths(dest_root, &rel_paths)?
    } else {
        std::collections::HashSet::new()
    };

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

            // Only run full classification when source is a regular file
            // (matching planner preconditions). For missing/non-regular sources
            // classify_destination's read-based comparison would be misleading.
            if source_exists && abs_path.is_file() {
                let state = crate::planner::classify_destination(
                    rp,
                    &abs_path,
                    &dest_path,
                    &dest_tracked_set,
                    &fs,
                );
                match state {
                    crate::model::DestinationState::Missing => {
                        println!("destination: missing");
                        if eligible {
                            println!("planned_action: copy");
                        }
                    }
                    crate::model::DestinationState::UpToDate => {
                        println!("destination: up-to-date");
                        println!("planned_action: no-op");
                    }
                    crate::model::DestinationState::UntrackedConflict => {
                        println!("destination: untracked-conflict");
                        println!("planned_action: skip (untracked conflict)");
                    }
                    crate::model::DestinationState::TrackedConflict => {
                        println!("destination: tracked-conflict");
                        println!("planned_action: skip (tracked conflict)");
                    }
                    crate::model::DestinationState::TypeConflict => {
                        println!("destination: type-conflict");
                        println!("planned_action: skip (type conflict)");
                    }
                    crate::model::DestinationState::UnsafePath => {
                        println!("destination: unsafe-path");
                        println!("planned_action: skip (unsafe path)");
                    }
                }
            } else if dest_path.exists() {
                println!("destination: exists");
            } else {
                println!("destination: missing");
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

    let report = validate::validate(&ctx, &git);

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
