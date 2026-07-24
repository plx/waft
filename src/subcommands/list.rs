//! `list` subcommand — enumerate files eligible to copy without mutating.

use clap::Args;

use crate::cli::Cli;
use crate::config::ResolvedPolicy;
use crate::context::{self, CommandKind};
use crate::error::{Error, Result};
use crate::git::{GitBackend, default_git_backend};
use crate::model::{RepoContext, ValidationSeverity};
use crate::validate;

/// Arguments for the list command.
#[derive(Debug, Args)]
pub struct ListArgs {}

/// Run the `list` subcommand.
pub fn run_list(cli: &Cli, policy: &ResolvedPolicy, _args: &ListArgs) -> Result<()> {
    let git = default_git_backend();

    // Resolve context
    let ctx = context::resolve_context(
        git.as_ref(),
        cli.source.as_deref(),
        cli.dest.as_deref(),
        cli.directory.as_deref(),
        CommandKind::List,
    )?;

    run_list_with_context(cli, policy, _args, &ctx, git.as_ref())
}

pub(crate) fn run_list_with_context(
    cli: &Cli,
    policy: &ResolvedPolicy,
    _args: &ListArgs,
    ctx: &RepoContext,
    git: &dyn GitBackend,
) -> Result<()> {
    // Validate
    let report = validate::validate(ctx, git, policy.symlink_policy);
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

    let eligible = super::eligible_records(git, &ctx.source_root, policy, ctx.core_ignore_case)?;

    // Pre-compute destination classification data if verbose + dest available
    let verbose = cli.verbose > 0 && !cli.quiet;
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
        if cli.quiet {
            continue;
        }
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
            let engine = crate::worktreeinclude_engine::engine_for(policy.semantics);
            let wti = engine.evaluate(
                &ctx.source_root,
                path,
                false,
                ctx.core_ignore_case,
                policy.symlink_policy,
            );
            let wti_str = super::format_worktreeinclude_status(&wti, true, policy.semantics);

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
