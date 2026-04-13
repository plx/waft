use clap::Args;

use crate::cli::Cli;
use crate::context::{self, CommandKind};
use crate::error::{Error, Result};
use crate::git::{GitBackend, GitCli};
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
pub fn run_copy(cli: &Cli, args: &CopyArgs) -> Result<()> {
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
