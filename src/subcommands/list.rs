use clap::Args;

use crate::cli::Cli;
use crate::context::{self, CommandKind};
use crate::error::{Error, Result};
use crate::git::{GitBackend, GitCli};
use crate::model::ValidationSeverity;
use crate::validate;
use crate::worktreeinclude;

/// Arguments for the list command.
#[derive(Debug, Args)]
pub struct ListArgs {}

/// Run the `list` subcommand.
pub fn run_list(cli: &Cli, _args: &ListArgs) -> Result<()> {
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
            let rel_paths: Vec<_> = eligible.iter().map(|r| r.path.clone()).collect();
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
            let size = std::fs::metadata(&abs_path).map(|m| m.len()).unwrap_or(0);

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
            let wti = worktreeinclude::explain(&ctx.source_root, path, false, ctx.core_ignore_case);
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

            println!(
                "{path}\tsize: {size}\tgitignore: {gitignore_str}\tworktreeinclude: {wti_str}"
            );

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
