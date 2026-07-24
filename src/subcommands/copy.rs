//! `copy` subcommand — the full plan-and-execute pipeline.

use clap::Args;

use crate::cli::Cli;
use crate::config::ResolvedPolicy;
use crate::context::{self, CommandKind};
use crate::error::{Error, Result};
use crate::git::{GitBackend, default_git_backend};
use crate::model::{RepoContext, ValidationSeverity};
use crate::validate;

/// Arguments for the copy command.
#[derive(Debug, Args)]
pub struct CopyArgs {
    /// Show what would be done without making changes.
    #[arg(short = 'n', long)]
    pub dry_run: bool,

    /// Compatibility flag; existing destination conflicts fail closed.
    #[arg(long)]
    pub overwrite: bool,
}

/// Run the `copy` subcommand.
pub fn run_copy(cli: &Cli, policy: &ResolvedPolicy, args: &CopyArgs) -> Result<()> {
    let git = default_git_backend();

    // Resolve context (copy requires a destination)
    let ctx = context::resolve_context(
        git.as_ref(),
        cli.source.as_deref(),
        cli.dest.as_deref(),
        cli.directory.as_deref(),
        CommandKind::Copy,
    )?;

    run_copy_with_context(cli, policy, args, &ctx, git.as_ref())
}

pub(crate) fn run_copy_with_context(
    cli: &Cli,
    policy: &ResolvedPolicy,
    args: &CopyArgs,
    ctx: &RepoContext,
    git: &dyn GitBackend,
) -> Result<()> {
    let fs = crate::fs::RealFs;

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

    let eligible: Vec<_> =
        super::eligible_records(git, &ctx.source_root, policy, ctx.core_ignore_case)?
            .into_iter()
            .map(|r| r.path)
            .collect();

    if eligible.is_empty() {
        if !cli.quiet {
            eprintln!("no eligible files found");
        }
        return Ok(());
    }

    let gitlinks = git.gitlinks(&ctx.source_root)?;
    let groups = crate::eligibility_groups::compute(&ctx.source_root, eligible, &gitlinks)?;

    // Build plan
    let plan = crate::planner::plan(ctx, report, groups, git, &fs, args.overwrite, args.dry_run)?;

    if args.dry_run {
        if !cli.quiet {
            crate::planner::render_dry_run(&plan);
        }
        return Ok(());
    }

    // Execute
    let copy_report = crate::executor::execute(&plan, &fs, git, policy.copy_strategy);
    crate::executor::render_report(&copy_report, cli.quiet);

    if let Some((failed, total)) = crate::executor::report_has_failures(&copy_report) {
        return Err(Error::CopyFailed { failed, total });
    }

    Ok(())
}
