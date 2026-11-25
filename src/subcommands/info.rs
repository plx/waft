//! `info` subcommand — detailed status for one or more paths.

use clap::Args;
use std::collections::HashSet;
use std::path::PathBuf;

use crate::cli::Cli;
use crate::config::ResolvedPolicy;
use crate::context::{self, CommandKind};
use crate::error::Result;
use crate::git::{GitBackend, default_git_backend};
use crate::model::RepoContext;
use crate::validate;
use waft_verified::report_view::{
    self, ActionLabel, DestLabel, InfoDestination, LstatType, SkipKind, SourceKindLabel,
};

/// Arguments for the info command.
#[derive(Debug, Args)]
pub struct InfoArgs {
    /// Paths to inspect.
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,
}

/// Run the `info` subcommand.
pub fn run_info(cli: &Cli, policy: &ResolvedPolicy, args: &InfoArgs) -> Result<()> {
    let git = default_git_backend()?;

    let ctx = context::resolve_context(
        git.as_ref(),
        cli.source.as_deref(),
        cli.dest.as_deref(),
        cli.directory.as_deref(),
        CommandKind::Info,
    )?;

    run_info_with_context(cli, policy, args, &ctx, git.as_ref())
}

pub(crate) fn run_info_with_context(
    cli: &Cli,
    policy: &ResolvedPolicy,
    args: &InfoArgs,
    ctx: &RepoContext,
    git: &dyn GitBackend,
) -> Result<()> {
    if cli.verbose > 0 && !cli.quiet {
        print_resolved_policy(policy);
    }

    // Validate
    let report = validate::validate(ctx, git, policy.symlink_policy);
    // Errors stop the command; otherwise warnings print unless quiet.
    validate::report_preflight(&report, cli.quiet)?;

    // Normalize all input paths to repo-relative
    let mut rel_paths = Vec::new();
    for path in &args.paths {
        let rp = cli.normalize_source_path(path, ctx)?;
        rel_paths.push(rp);
    }

    let pass = super::eligible_records(git, &ctx.source_root, policy, ctx.core_ignore_case)?;
    super::note_empty_selection(policy, &pass, cli.quiet);
    let eligible_set: HashSet<_> = pass.records.into_iter().map(|record| record.path).collect();

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
        // One `lstat`; the verified kernel names its file type with the same
        // precedence the planner's source examination uses.
        let source_metadata = std::fs::symlink_metadata(&abs_path).ok();
        let source_kind = report_view::source_kind(source_metadata.as_ref().map(|m| {
            let file_type = m.file_type();
            LstatType {
                is_symlink: file_type.is_symlink(),
                is_file: file_type.is_file(),
                is_dir: file_type.is_dir(),
            }
        }));
        let source_exists = source_kind.exists();
        let source_is_regular = source_kind.is_regular_file();

        let is_tracked = tracked_set.contains(rp);
        // The alias question is about `rp`, not about the eligible entry, so
        // resolve it once here rather than once per entry: the probe's
        // directory reads would otherwise repeat across the whole eligible
        // set, which is every selectable file in the repository.
        let eligible = eligible_set.contains(rp)
            || (source_exists && {
                let alias_probe = crate::git::RepoPathAliasProbe::new(&ctx.source_root, rp);
                eligible_set.iter().any(|path| {
                    crate::git::repo_paths_equivalent(
                        path.as_str(),
                        rp.as_str(),
                        ctx.core_ignore_case,
                    ) || alias_probe.resolves_to(path)
                })
            });

        // Git ignore status
        let gitignore_str = if is_tracked {
            "tracked".to_string()
        } else if let Some(record) = ignore_map.get(rp.as_str()) {
            if record.ignored {
                let info = record
                    .match_info
                    .as_ref()
                    .expect("ignored records include their matching rule");
                format!(
                    "ignored ({}:{}: {})",
                    info.source_file.display(),
                    info.line,
                    info.pattern
                )
            } else if let Some(ref info) = record.match_info {
                format!(
                    "not ignored ({}:{}: {})",
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
        let engine = crate::worktreeinclude_engine::engine_for(policy.semantics);
        let wti = engine.evaluate(
            &ctx.source_root,
            rp.as_str(),
            source_kind.is_directory(),
            ctx.core_ignore_case,
            policy.symlink_policy,
        );
        let wti_str = super::format_worktreeinclude_status(&wti, eligible, policy.semantics);

        if !cli.quiet {
            println!("path: {rp}");
            println!(
                "source_exists: {}",
                if source_exists { "yes" } else { "no" }
            );
            println!("source_kind: {}", source_kind_text(source_kind));
            println!("tracked: {}", if is_tracked { "yes" } else { "no" });
            println!("gitignore: {gitignore_str}");
            println!("worktreeinclude: {wti_str}");
            println!("eligible_to_copy: {}", if eligible { "yes" } else { "no" });
        }

        // Destination info if available
        if let Some(ref dest_root) = ctx.dest_root {
            let dest_path = rp.to_path(dest_root);

            // Only run full classification when source is a regular file
            // (matching planner preconditions). For missing/non-regular sources
            // classify_destination's read-based comparison would be misleading.
            // The classification runs even when quiet; the presence check for
            // other sources only when something will be printed.
            let destination = if source_is_regular {
                let state = crate::planner::classify_destination(
                    rp,
                    &abs_path,
                    &dest_path,
                    &dest_tracked_set,
                    &fs,
                );
                Some(InfoDestination::Classified(super::list::dest_state_view(
                    &state,
                )))
            } else if !cli.quiet {
                Some(InfoDestination::Unclassified {
                    present: std::fs::symlink_metadata(&dest_path).is_ok(),
                })
            } else {
                None
            };
            if let Some(destination) = destination
                && !cli.quiet
            {
                // The remedy is named only where it works; the classification
                // itself is the same everywhere.
                let view = report_view::info_destination(
                    destination,
                    eligible,
                    crate::fs::overwrite_supported(),
                );
                println!("destination: {}", destination_text(view.destination));
                if let Some(action) = view.planned_action {
                    println!("planned_action: {}", info_action_text(action));
                }
            }
        }
        if !cli.quiet {
            println!();
        }
    }

    Ok(())
}

/// The text `info` prints for a source kind.
fn source_kind_text(kind: SourceKindLabel) -> &'static str {
    match kind {
        SourceKindLabel::Missing => "missing",
        SourceKindLabel::Symlink => "symlink",
        SourceKindLabel::File => "file",
        SourceKindLabel::Directory => "directory",
        SourceKindLabel::Other => "other",
    }
}

/// The text `info` prints for a destination label.
fn destination_text(label: DestLabel) -> &'static str {
    match label {
        DestLabel::Missing => "missing",
        DestLabel::UpToDate => "up-to-date",
        DestLabel::PermissionsDiffer => "permissions-differ",
        DestLabel::UntrackedConflict => "untracked-conflict",
        DestLabel::TrackedConflict => "tracked-conflict",
        DestLabel::TypeConflict => "type-conflict",
        DestLabel::UnsafePath => "unsafe-path",
        DestLabel::Exists => "exists",
    }
}

/// The text `info` prints for a planned action.
///
/// The kernel proves `info` never plans `UnsupportedSourceType` (it plans
/// only for classified regular sources); the arm exists for exhaustiveness.
fn info_action_text(action: ActionLabel) -> &'static str {
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
            SkipKind::UntrackedConflict => {
                if remedy {
                    "skip (untracked conflict; --overwrite replaces)"
                } else {
                    "skip (untracked conflict)"
                }
            }
            SkipKind::TrackedConflict => "skip (tracked conflict)",
            SkipKind::TypeConflict => "skip (type conflict)",
            SkipKind::UnsafePath => "skip (unsafe path)",
            SkipKind::UnsupportedSourceType => "skip (unsupported source type)",
        },
    }
}

/// Print the active resolved policy in a stable, machine-readable format.
fn print_resolved_policy(policy: &ResolvedPolicy) {
    println!("policy:");
    println!("  profile: {}", policy.profile.as_str());
    println!("  when_missing: {}", policy.when_missing.as_str());
    println!("  semantics: {}", policy.semantics.as_str());
    println!("  symlink_policy: {}", policy.symlink_policy.as_str());
    println!(
        "  builtin_exclude_set: {}",
        policy.builtin_exclude_set.as_str()
    );
    if policy.extra_excludes.is_empty() {
        println!("  extra_excludes: []");
    } else {
        println!("  extra_excludes:");
        for entry in &policy.extra_excludes {
            println!("    - {entry}");
        }
    }
    println!();
}
