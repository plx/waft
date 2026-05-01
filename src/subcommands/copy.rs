//! `copy` subcommand — the full plan-and-execute pipeline.

use clap::Args;

use crate::cli::Cli;
use crate::config::ResolvedPolicy;
use crate::context::{self, CommandKind};
use crate::error::{Error, Result};
use crate::git::default_git_backend;
use crate::model::ValidationSeverity;
use crate::validate;

/// Arguments for the copy command.
#[derive(Debug, Args)]
pub struct CopyArgs {
    /// Show what would be done without making changes.
    #[arg(short = 'n', long)]
    pub dry_run: bool,

    /// Allow overwriting existing untracked destination files.
    #[arg(long)]
    pub overwrite: bool,
}

/// Run the `copy` subcommand.
pub fn run_copy(cli: &Cli, policy: &ResolvedPolicy, args: &CopyArgs) -> Result<()> {
    let git = default_git_backend();
    let fs = crate::fs::RealFs;

    // Resolve context (copy requires a destination)
    let ctx = context::resolve_context(
        git.as_ref(),
        cli.source.as_deref(),
        cli.dest.as_deref(),
        cli.directory.as_deref(),
        CommandKind::Copy,
    )?;

    // Validate
    let report = validate::validate(&ctx, git.as_ref(), policy.symlink_policy);
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

    // Enumerate candidate files per the active policy.
    let mut candidates = super::select_candidates(git.as_ref(), &ctx.source_root, policy)?;
    // Apply post-selection exclusion policy (builtin set + extra excludes).
    crate::policy_filter::filter_paths(&mut candidates, policy, &ctx.source_root)?;
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
        git.as_ref(),
        &fs,
        args.overwrite,
        args.dry_run,
    )?;

    if args.dry_run {
        crate::planner::render_dry_run(&plan);
        return Ok(());
    }

    // Execute
    let copy_report = crate::executor::execute(&plan, &fs, policy.copy_strategy);
    crate::executor::render_report(&copy_report, cli.quiet);

    if let Some((failed, total)) = crate::executor::report_has_failures(&copy_report) {
        return Err(Error::CopyFailed { failed, total });
    }

    Ok(())
}
