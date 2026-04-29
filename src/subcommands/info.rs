use clap::Args;
use std::path::PathBuf;

use crate::cli::Cli;
use crate::config::ResolvedPolicy;
use crate::context::{self, CommandKind};
use crate::error::{Error, Result};
use crate::git::default_git_backend;
use crate::model::ValidationSeverity;
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

    if cli.verbose > 0 {
        print_resolved_policy(policy);
    }

    let ctx = context::resolve_context(
        git.as_ref(),
        cli.source.as_deref(),
        cli.dest.as_deref(),
        cli.directory.as_deref(),
        CommandKind::Info,
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

    // Normalize all input paths to repo-relative
    let mut rel_paths = Vec::new();
    for path in &args.paths {
        let rp = crate::path::RepoRelPath::normalize(path, &ctx.source_root)?;
        rel_paths.push(rp);
    }

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
        let source_exists = abs_path.exists();
        let source_kind = if !source_exists {
            "missing"
        } else if abs_path.is_file() {
            "file"
        } else if abs_path.is_dir() {
            "directory"
        } else if abs_path.is_symlink() {
            "symlink"
        } else {
            "other"
        };

        let is_tracked = tracked_set.contains(rp);

        // Git ignore status
        let gitignore_str = if is_tracked {
            "tracked".to_string()
        } else if let Some(record) = ignore_map.get(rp.as_str()) {
            if let Some(ref info) = record.match_info {
                format!(
                    "ignored ({}:{}: {})",
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
            abs_path.is_dir(),
            ctx.core_ignore_case,
            policy.symlink_policy,
        );
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

        // Eligibility
        let is_ignored = !is_tracked
            && ignore_map
                .get(rp.as_str())
                .map(|r| r.match_info.is_some())
                .unwrap_or(false);
        let is_included = matches!(wti, crate::model::WorktreeincludeStatus::Included { .. });
        let eligible =
            source_exists && abs_path.is_file() && !is_tracked && is_ignored && is_included;

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

        // Destination info if available
        if let Some(ref dest_root) = ctx.dest_root {
            let dest_path = rp.to_path(dest_root);

            // Only run full classification when source is a regular file
            // (matching planner preconditions). For missing/non-regular sources
            // classify_destination's read-based comparison would be misleading.
            if source_exists && abs_path.is_file() {
                let state = crate::planner::classify_destination(
                    rp,
                    &abs_path,
                    &dest_path,
                    &dest_tracked_set,
                    &fs,
                );
                match state {
                    crate::model::DestinationState::Missing => {
                        println!("destination: missing");
                        if eligible {
                            println!("planned_action: copy");
                        }
                    }
                    crate::model::DestinationState::UpToDate => {
                        println!("destination: up-to-date");
                        println!("planned_action: no-op");
                    }
                    crate::model::DestinationState::UntrackedConflict => {
                        println!("destination: untracked-conflict");
                        println!("planned_action: skip (untracked conflict)");
                    }
                    crate::model::DestinationState::TrackedConflict => {
                        println!("destination: tracked-conflict");
                        println!("planned_action: skip (tracked conflict)");
                    }
                    crate::model::DestinationState::TypeConflict => {
                        println!("destination: type-conflict");
                        println!("planned_action: skip (type conflict)");
                    }
                    crate::model::DestinationState::UnsafePath => {
                        println!("destination: unsafe-path");
                        println!("planned_action: skip (unsafe path)");
                    }
                }
            } else if dest_path.exists() {
                println!("destination: exists");
            } else {
                println!("destination: missing");
            }
        }
        println!();
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
