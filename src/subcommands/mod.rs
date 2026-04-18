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
use crate::error::Result;
use crate::git::GitBackend;
use crate::path::RepoRelPath;

pub use copy::{CopyArgs, run_copy};
pub use info::{InfoArgs, run_info};
pub use list::{ListArgs, run_list};
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
/// The returned set still needs to be filtered against `check_ignore` to
/// retain only paths that are actually git-ignored. Existing callers do that
/// step separately so `info` / `list` verbose output can reference each
/// candidate's matching ignore source.
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
