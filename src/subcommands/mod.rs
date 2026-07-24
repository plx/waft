//! Subcommand argument types and top-level handlers.
//!
//! Each submodule owns the `clap::Args` struct and the `run_*` entry point
//! for one subcommand. The handlers share the same early stages — context
//! resolution and validation — then diverge based on which pipeline stage the
//! subcommand needs to reach (see `docs/architecture.md`).

mod copy;
mod info;
mod list;
mod validate;

use std::path::Path;

use crate::config::{ResolvedPolicy, WhenMissingWorktreeinclude, WorktreeincludeSemantics};
use crate::error::{Error, Result};
use crate::git::{GitBackend, IgnoreCheckRecord};
use crate::model::WorktreeincludeStatus;
use crate::path::RepoRelPath;

pub(crate) use copy::run_copy_with_context;
pub use copy::{CopyArgs, run_copy};
pub(crate) use info::run_info_with_context;
pub use info::{InfoArgs, run_info};
pub(crate) use list::run_list_with_context;
pub use list::{ListArgs, run_list};
pub(crate) use validate::run_validate_with_context;
pub use validate::{ValidateArgs, run_validate};

/// Select candidate paths in `source_root` according to the active policy.
///
/// Mirrors the contract documented in the worktreeinclude config schema:
///
/// - If at least one `.worktreeinclude` file exists anywhere in the repo,
///   selection runs through the `.worktreeinclude` matcher.
/// - Otherwise, `policy.when_missing` decides:
///   - `blank`: no candidates,
///   - `all-ignored`: every git-ignored untracked file is a candidate.
///
/// The returned set is only the profile selection stage. Call
/// [`eligible_records`] to apply exclusions, Git-ignore membership, and
/// physical source-type checks.
pub(crate) fn select_candidates(
    git: &dyn GitBackend,
    source_root: &Path,
    policy: &ResolvedPolicy,
) -> Result<Vec<RepoRelPath>> {
    if git.worktreeinclude_exists_anywhere(source_root, policy.symlink_policy)? {
        // The wt-0.39 engine is too unusual for the per-path
        // `list_worktreeinclude_candidates` shape (it's purely subtractive
        // on top of the all-ignored set); route it through a dedicated
        // helper. when_missing is not consulted here because a rule file
        // exists; explicit-selection mode is engaged.
        if policy.semantics == WorktreeincludeSemantics::Wt039 {
            return crate::worktreeinclude_engine::wt_collect_candidates(
                source_root,
                git,
                policy.symlink_policy,
            );
        }
        git.list_worktreeinclude_candidates(source_root, policy.semantics, policy.symlink_policy)
    } else {
        match policy.when_missing {
            WhenMissingWorktreeinclude::Blank => Ok(Vec::new()),
            WhenMissingWorktreeinclude::AllIgnored => git.list_ignored_untracked(source_root),
        }
    }
}

/// Evaluate the complete, command-independent eligibility contract.
///
/// A returned path is selected by the configured worktreeinclude/fallback
/// semantics, survives policy exclusions, is Git-ignored and untracked, and
/// is a physical regular file (not a symlink). Keeping this pass shared makes
/// `list`, `info`, and `copy` agree on the meaning of "eligible".
pub(crate) fn eligible_records(
    git: &dyn GitBackend,
    source_root: &Path,
    policy: &ResolvedPolicy,
    core_ignore_case: bool,
) -> Result<Vec<IgnoreCheckRecord>> {
    let mut candidates = select_candidates(git, source_root, policy)?;
    candidates.sort();
    candidates.dedup();

    crate::policy_filter::filter_paths_with_case(
        &mut candidates,
        policy,
        source_root,
        crate::policy_filter::effective_case_insensitive(core_ignore_case),
    )?;
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let mut eligible = Vec::new();
    for record in git.check_ignore(source_root, &candidates)? {
        if !record.ignored {
            continue;
        }

        let source = record.path.to_path(source_root);
        let metadata = match std::fs::symlink_metadata(&source) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source_error) => {
                return Err(Error::Io {
                    context: format!("reading source metadata for {}", source.display()),
                    source: source_error,
                });
            }
        };
        if metadata.file_type().is_file() {
            eligible.push(record);
        }
    }

    eligible.sort_by(|a, b| a.path.cmp(&b.path));
    eligible.dedup_by(|a, b| a.path == b.path);
    Ok(eligible)
}

/// Render a per-path explanation without contradicting the canonical
/// selection result. Wt 0.39 is set-based: glob negations shown by the
/// diagnostic Git matcher are deliberately non-operative.
pub(crate) fn format_worktreeinclude_status(
    status: &WorktreeincludeStatus,
    selected: bool,
    semantics: WorktreeincludeSemantics,
) -> String {
    match status {
        WorktreeincludeStatus::Included {
            file,
            line,
            pattern,
        } => format!("included ({}:{}: {})", file.display(), line, pattern),
        WorktreeincludeStatus::ExcludedByNegation {
            file,
            line,
            pattern,
        } if selected && semantics == WorktreeincludeSemantics::Wt039 => format!(
            "selected (effective wt-0.39 policy; rule does not subtract this path: {}:{}: {})",
            file.display(),
            line,
            pattern
        ),
        WorktreeincludeStatus::ExcludedByNegation {
            file,
            line,
            pattern,
        } => format!("excluded ({}:{}: {})", file.display(), line, pattern),
        WorktreeincludeStatus::NoMatch if selected => "selected (effective policy)".to_string(),
        WorktreeincludeStatus::NoMatch => "no match".to_string(),
    }
}
