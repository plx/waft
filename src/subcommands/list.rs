//! `list` subcommand — enumerate files eligible to copy without mutating.

use clap::Args;

use crate::cli::Cli;
use crate::config::ResolvedPolicy;
use crate::context::{self, CommandKind};
use crate::error::Result;
use crate::git::{GitBackend, default_git_backend};
use crate::model::{DestinationState, RepoContext};
use crate::validate;
use waft_verified::report_view::{self, ActionLabel, DestState, ListSource, SkipKind};

/// Arguments for the list command.
#[derive(Debug, Args)]
pub struct ListArgs {}

/// Run the `list` subcommand.
pub fn run_list(cli: &Cli, policy: &ResolvedPolicy, _args: &ListArgs) -> Result<()> {
    let git = default_git_backend()?;

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
    // Errors stop the command; otherwise warnings print unless quiet.
    validate::report_preflight(&report, cli.quiet)?;

    let pass = super::eligible_records(git, &ctx.source_root, policy, ctx.core_ignore_case)?;
    super::note_empty_selection(policy, &pass, cli.quiet);
    let eligible = pass.records;

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

            // Predicted action (only when --dest is available). The source
            // checks and the destination classification run in the same
            // order as the planner's preconditions; the verified kernel picks
            // the action from what they observed.
            if let Some(ref dest_root) = ctx.dest_root {
                let source = if abs_path.is_symlink() || !abs_path.is_file() {
                    // Planner skips non-regular-file and symlink sources
                    ListSource::NonRegular {
                        exists: abs_path.exists(),
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
                    ListSource::Regular(dest_state_view(&state))
                };
                if let Some(action) =
                    report_view::list_action(source, crate::fs::overwrite_supported())
                {
                    println!("\taction: {}", list_action_text(action));
                }
            }
        } else {
            println!("{path}");
        }
    }

    Ok(())
}

/// The kernel's view of a destination classification.
pub(super) fn dest_state_view(state: &DestinationState) -> DestState {
    match state {
        DestinationState::Missing => DestState::Missing,
        DestinationState::UpToDate => DestState::UpToDate,
        DestinationState::PermissionsDiffer => DestState::PermissionsDiffer,
        DestinationState::UntrackedConflict => DestState::UntrackedConflict,
        DestinationState::TrackedConflict => DestState::TrackedConflict,
        DestinationState::TypeConflict => DestState::TypeConflict,
        DestinationState::UnsafePath => DestState::UnsafePath,
    }
}

/// The text `list -v` prints for a predicted action.
///
/// The remedy clause appears only where `--overwrite` can act on an existing
/// destination; for an untracked conflict `list -v` never names it.
fn list_action_text(action: ActionLabel) -> &'static str {
    match action {
        ActionLabel::Copy => "copy",
        ActionLabel::NoOp => "no-op",
        ActionLabel::Skip { reason, remedy } => match reason {
            SkipKind::PermissionsDiffer => {
                if remedy {
                    "skip (content equal, permissions differ; --overwrite repairs)"
                } else {
                    "skip (content equal, permissions differ)"
                }
            }
            SkipKind::UntrackedConflict => "skip (untracked conflict)",
            SkipKind::TrackedConflict => "skip (tracked conflict)",
            SkipKind::TypeConflict => "skip (type conflict)",
            SkipKind::UnsafePath => "skip (unsafe path)",
            SkipKind::UnsupportedSourceType => "skip (unsupported source type)",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dest_state_view_maps_each_variant_to_its_namesake() {
        let cases = [
            (DestinationState::Missing, DestState::Missing),
            (DestinationState::UpToDate, DestState::UpToDate),
            (
                DestinationState::PermissionsDiffer,
                DestState::PermissionsDiffer,
            ),
            (
                DestinationState::UntrackedConflict,
                DestState::UntrackedConflict,
            ),
            (
                DestinationState::TrackedConflict,
                DestState::TrackedConflict,
            ),
            (DestinationState::TypeConflict, DestState::TypeConflict),
            (DestinationState::UnsafePath, DestState::UnsafePath),
        ];
        for (state, expected) in cases {
            assert_eq!(dest_state_view(&state), expected, "{state:?}");
        }
    }

    #[test]
    fn list_action_text_matches_the_published_wording() {
        let skip = |reason, remedy| ActionLabel::Skip { reason, remedy };
        assert_eq!(list_action_text(ActionLabel::Copy), "copy");
        assert_eq!(list_action_text(ActionLabel::NoOp), "no-op");
        assert_eq!(
            list_action_text(skip(SkipKind::PermissionsDiffer, true)),
            "skip (content equal, permissions differ; --overwrite repairs)"
        );
        assert_eq!(
            list_action_text(skip(SkipKind::PermissionsDiffer, false)),
            "skip (content equal, permissions differ)"
        );
        for remedy in [true, false] {
            assert_eq!(
                list_action_text(skip(SkipKind::UntrackedConflict, remedy)),
                "skip (untracked conflict)"
            );
        }
        assert_eq!(
            list_action_text(skip(SkipKind::UnsupportedSourceType, false)),
            "skip (unsupported source type)"
        );
    }
}
