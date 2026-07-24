//! `info` subcommand — detailed status for one or more paths.

use clap::Args;
use std::collections::HashSet;
use std::path::PathBuf;

use crate::cli::Cli;
use crate::config::ResolvedPolicy;
use crate::context::{self, CommandKind};
use crate::error::{Error, Result};
use crate::git::{GitBackend, default_git_backend};
use crate::model::{RepoContext, ValidationSeverity};
use crate::validate;

/// Arguments for the info command.
#[derive(Debug, Args)]
pub struct InfoArgs {
    /// Paths to inspect.
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,
}

/// Run the `info` subcommand.
pub fn run_info(cli: &Cli, policy: &ResolvedPolicy, args: &InfoArgs) -> Result<()> {
    let git = default_git_backend();

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

    // Normalize all input paths to repo-relative
    let mut rel_paths = Vec::new();
    for path in &args.paths {
        let rp = cli.normalize_source_path(path, ctx)?;
        rel_paths.push(rp);
    }

    let eligible_set: HashSet<_> =
        super::eligible_records(git, &ctx.source_root, policy, ctx.core_ignore_case)?
            .into_iter()
            .map(|record| record.path)
            .collect();

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
        let source_metadata = std::fs::symlink_metadata(&abs_path).ok();
        let source_exists = source_metadata.is_some();
        let source_kind = match source_metadata.as_ref().map(|m| m.file_type()) {
            None => "missing",
            Some(kind) if kind.is_symlink() => "symlink",
            Some(kind) if kind.is_file() => "file",
            Some(kind) if kind.is_dir() => "directory",
            Some(_) => "other",
        };
        let source_is_regular = source_kind == "file";

        let is_tracked = tracked_set.contains(rp);
        let eligible = eligible_set.contains(rp)
            || (source_exists
                && eligible_set.iter().any(|path| {
                    crate::git::repo_paths_equivalent(
                        path.as_str(),
                        rp.as_str(),
                        ctx.core_ignore_case,
                    ) || crate::git::repo_paths_alias_on_filesystem(&ctx.source_root, path, rp)
                }));

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
            source_kind == "directory",
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
            println!("source_kind: {source_kind}");
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
            if source_is_regular {
                let state = crate::planner::classify_destination(
                    rp,
                    &abs_path,
                    &dest_path,
                    &dest_tracked_set,
                    &fs,
                );
                if !cli.quiet {
                    match state {
                        crate::model::DestinationState::Missing => {
                            println!("destination: missing");
                            if eligible {
                                println!("planned_action: copy");
                            }
                        }
                        crate::model::DestinationState::UpToDate => {
                            println!("destination: up-to-date");
                            if eligible {
                                println!("planned_action: no-op");
                            }
                        }
                        crate::model::DestinationState::UntrackedConflict => {
                            println!("destination: untracked-conflict");
                            if eligible {
                                println!("planned_action: skip (untracked conflict)");
                            }
                        }
                        crate::model::DestinationState::TrackedConflict => {
                            println!("destination: tracked-conflict");
                            if eligible {
                                println!("planned_action: skip (tracked conflict)");
                            }
                        }
                        crate::model::DestinationState::TypeConflict => {
                            println!("destination: type-conflict");
                            if eligible {
                                println!("planned_action: skip (type conflict)");
                            }
                        }
                        crate::model::DestinationState::UnsafePath => {
                            println!("destination: unsafe-path");
                            if eligible {
                                println!("planned_action: skip (unsafe path)");
                            }
                        }
                    }
                }
            } else if !cli.quiet {
                if std::fs::symlink_metadata(&dest_path).is_ok() {
                    println!("destination: exists");
                } else {
                    println!("destination: missing");
                }
            }
        }
        if !cli.quiet {
            println!();
        }
    }

    Ok(())
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
