//! Pluggable semantics for `.worktreeinclude` evaluation.
//!
//! The selection algorithm has three documented variants, each captured
//! by a separate engine struct:
//!
//! - [`GitSemantics`]: Git-equivalent per-directory `.gitignore`-style
//!   matching, with the within-file and cross-file negation caveats. This
//!   is what `waft` has done since its initial implementation; PR6
//!   formalizes it as a named engine.
//! - [`Claude202604Semantics`]: a versioned snapshot of Claude Code's
//!   observed behavior. PR6 ships this as a thin delegate to
//!   [`GitSemantics`]; PR7 implements the divergence (notably ignoring
//!   nested `.worktreeinclude` files).
//! - [`Wt039Semantics`]: a versioned snapshot of `worktrunk 0.39`'s
//!   behavior. PR6 ships this as a thin delegate to [`GitSemantics`];
//!   PR8 implements the divergence.
//!
//! Behavior is unchanged in PR6 — the abstraction is in place but each
//! engine returns identical results. Subsequent PRs add the per-engine
//! deviations covered by the fixture matrix.

use std::path::Path;

use crate::config::{SymlinkPolicy, WorktreeincludeSemantics};
use crate::model::WorktreeincludeStatus;

/// A pluggable `.worktreeinclude` evaluation engine.
pub trait WorktreeincludeSemanticsEngine {
    /// Evaluate `rel_path` against the engine's selection algorithm.
    fn evaluate(
        &self,
        repo_root: &Path,
        rel_path: &str,
        is_dir: bool,
        case_insensitive: bool,
        symlink_policy: SymlinkPolicy,
    ) -> WorktreeincludeStatus;
}

/// Git per-directory `.gitignore`-style semantics with negation caveats.
#[derive(Debug, Default)]
pub struct GitSemantics;

impl WorktreeincludeSemanticsEngine for GitSemantics {
    fn evaluate(
        &self,
        repo_root: &Path,
        rel_path: &str,
        is_dir: bool,
        case_insensitive: bool,
        symlink_policy: SymlinkPolicy,
    ) -> WorktreeincludeStatus {
        crate::worktreeinclude::explain(
            repo_root,
            rel_path,
            is_dir,
            case_insensitive,
            symlink_policy,
        )
    }
}

/// Versioned snapshot of Claude Code's observed behavior (2026-04).
///
/// PR6: thin delegate to [`GitSemantics`]. PR7 implements the divergence.
#[derive(Debug, Default)]
pub struct Claude202604Semantics;

impl WorktreeincludeSemanticsEngine for Claude202604Semantics {
    fn evaluate(
        &self,
        repo_root: &Path,
        rel_path: &str,
        is_dir: bool,
        case_insensitive: bool,
        symlink_policy: SymlinkPolicy,
    ) -> WorktreeincludeStatus {
        GitSemantics.evaluate(
            repo_root,
            rel_path,
            is_dir,
            case_insensitive,
            symlink_policy,
        )
    }
}

/// Versioned snapshot of `worktrunk 0.39`'s observed behavior.
///
/// PR6: thin delegate to [`GitSemantics`]. PR8 implements the divergence.
#[derive(Debug, Default)]
pub struct Wt039Semantics;

impl WorktreeincludeSemanticsEngine for Wt039Semantics {
    fn evaluate(
        &self,
        repo_root: &Path,
        rel_path: &str,
        is_dir: bool,
        case_insensitive: bool,
        symlink_policy: SymlinkPolicy,
    ) -> WorktreeincludeStatus {
        GitSemantics.evaluate(
            repo_root,
            rel_path,
            is_dir,
            case_insensitive,
            symlink_policy,
        )
    }
}

/// Construct the engine implementing the requested semantics.
pub fn engine_for(s: WorktreeincludeSemantics) -> Box<dyn WorktreeincludeSemanticsEngine> {
    match s {
        WorktreeincludeSemantics::Git => Box::new(GitSemantics),
        WorktreeincludeSemantics::Claude202604 => Box::new(Claude202604Semantics),
        WorktreeincludeSemantics::Wt039 => Box::new(Wt039Semantics),
    }
}
