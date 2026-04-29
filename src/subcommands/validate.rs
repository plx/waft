use clap::Args;

use crate::cli::Cli;
use crate::config::ResolvedPolicy;
use crate::context::{self, CommandKind};
use crate::error::{Error, Result};
use crate::git::default_git_backend;
use crate::model::ValidationSeverity;
use crate::validate;

/// Arguments for the validate command.
#[derive(Debug, Args)]
pub struct ValidateArgs {}

/// Run the `validate` subcommand.
pub fn run_validate(cli: &Cli, policy: &ResolvedPolicy, _args: &ValidateArgs) -> Result<()> {
    let git = default_git_backend();

    let ctx = context::resolve_context(
        git.as_ref(),
        cli.source.as_deref(),
        cli.dest.as_deref(),
        cli.directory.as_deref(),
        CommandKind::Validate,
    )?;

    let report = validate::validate(&ctx, git.as_ref(), policy.symlink_policy);

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
