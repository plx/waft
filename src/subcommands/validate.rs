//! `validate` subcommand — lint ignore and `.worktreeinclude` files.

use clap::Args;

use crate::cli::Cli;
use crate::config::ResolvedPolicy;
use crate::context::{self, CommandKind};
use crate::error::{Error, Result};
use crate::git::{GitBackend, default_git_backend};
use crate::model::RepoContext;
use crate::validate;
use waft_verified::validation::{GateOutcome, ReportMode};

/// Arguments for the validate command.
#[derive(Debug, Args)]
pub struct ValidateArgs {}

/// Run the `validate` subcommand.
pub fn run_validate(cli: &Cli, policy: &ResolvedPolicy, _args: &ValidateArgs) -> Result<()> {
    let git = default_git_backend()?;

    let ctx = context::resolve_context(
        git.as_ref(),
        cli.source.as_deref(),
        cli.dest.as_deref(),
        cli.directory.as_deref(),
        CommandKind::Validate,
    )?;

    run_validate_with_context(cli, policy, _args, &ctx, git.as_ref())
}

pub(crate) fn run_validate_with_context(
    cli: &Cli,
    policy: &ResolvedPolicy,
    _args: &ValidateArgs,
    ctx: &RepoContext,
    git: &dyn GitBackend,
) -> Result<()> {
    let report = validate::validate(ctx, git, policy.symlink_policy);

    // The verified kernel decides which issues print (errors always,
    // warnings unless quiet), whether to announce success, and the outcome.
    let view = waft_verified::validation::validation_view(
        &report.severities(),
        ReportMode::Validate,
        cli.quiet,
    );
    for &index in &view.shown {
        let issue = &report.issues[index];
        let severity = validate::severity_label(issue.severity);
        let location = if let Some(line) = issue.line {
            format!("{}:{}", issue.file.display(), line)
        } else {
            issue.file.display().to_string()
        };
        eprintln!("{severity}: {location}: {}", issue.message);
    }

    if view.announce_pass {
        eprintln!("validation passed");
    }
    match view.outcome {
        GateOutcome::Abort { error_count } => Err(Error::Validation { error_count }),
        GateOutcome::Proceed => Ok(()),
    }
}
