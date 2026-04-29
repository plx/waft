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
/// Claude consults ONLY the repository's root-level `.worktreeinclude` file;
/// nested `.worktreeinclude` files are silently ignored. This mirrors the
/// matrix expectations:
///
/// - F3 (root `*.env`, `config/!*.env`): both root and nested env files
///   are selected because the nested negation is never read.
/// - F4 (only `config/.worktreeinclude`): no root file → no patterns →
///   nothing selected (combined with `when_missing = blank`).
/// - F5 (root `secrets/`, nested `!private.key`): nested negation
///   ignored, so `secrets/private.key` stays selected.
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
        crate::worktreeinclude::evaluate_root_only(
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
/// Worktrunk's selection algorithm differs from Git's per-directory
/// `.gitignore`-style matching in a way that is not captured by a single
/// per-path evaluation: regardless of which `.worktreeinclude` files
/// exist, wt always starts from the full set of git-ignored untracked
/// files and then SUBTRACTS files matched by *literal* `!<filename>`
/// negations (no globs). Glob negations and positive patterns are
/// ignored.
///
/// Because the per-path engine interface is too narrow for this — there
/// is no "all-ignored" base set per file — `subcommands::select_candidates`
/// special-cases [`WorktreeincludeSemantics::Wt039`] and routes through
/// [`wt_collect_candidates`] below. Calls to `evaluate` itself fall back
/// to [`GitSemantics`] for clients (e.g. `info`) that ask about a single
/// path; this keeps verbose output reasonable even when the candidate
/// set is computed differently.
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

/// Compute candidate paths under the wt-0.39 algorithm.
///
/// 1. Start from `git.list_ignored_untracked(source_root)`.
/// 2. Walk the source tree for `.worktreeinclude` files (honoring
///    `symlink_policy`); for each literal `!<name>` line, resolve the
///    referenced path relative to the rule file's directory and remove
///    it from the candidate set.
///
/// Glob negations are ignored. Positive patterns are ignored — wt's
/// observed behavior is that `.worktreeinclude` is purely subtractive.
pub fn wt_collect_candidates(
    source_root: &std::path::Path,
    git: &dyn crate::git::GitBackend,
    symlink_policy: SymlinkPolicy,
) -> crate::error::Result<Vec<crate::path::RepoRelPath>> {
    let mut paths = git.list_ignored_untracked(source_root)?;
    let removals = collect_wt_literal_negations(source_root, symlink_policy);
    paths.retain(|p| !removals.contains(p.as_str()));
    Ok(paths)
}

fn collect_wt_literal_negations(
    source_root: &std::path::Path,
    symlink_policy: SymlinkPolicy,
) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for entry in walkdir::WalkDir::new(source_root) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_name() != ".worktreeinclude" {
            continue;
        }
        if entry.file_type().is_dir() {
            continue;
        }
        if entry.file_type().is_symlink() && symlink_policy == SymlinkPolicy::Ignore {
            continue;
        }
        if !entry.file_type().is_file() && !entry.file_type().is_symlink() {
            continue;
        }
        let path = entry.path();
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let dir = path.parent().unwrap_or(source_root);
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some(rest) = trimmed.strip_prefix('!') else {
                continue;
            };
            // Skip glob patterns. Wt's observed behavior only honors
            // literal-name negations; anything containing a glob meta
            // character is left in the candidate set.
            if rest.chars().any(|c| matches!(c, '*' | '?' | '[' | ']')) {
                continue;
            }
            let stripped = rest.strip_prefix('/').unwrap_or(rest);
            let abs = dir.join(stripped);
            if let Ok(rel) = crate::path::RepoRelPath::normalize(&abs, source_root) {
                out.insert(rel.as_str().to_string());
            }
        }
    }
    out
}

/// Construct the engine implementing the requested semantics.
pub fn engine_for(s: WorktreeincludeSemantics) -> Box<dyn WorktreeincludeSemanticsEngine> {
    match s {
        WorktreeincludeSemantics::Git => Box::new(GitSemantics),
        WorktreeincludeSemantics::Claude202604 => Box::new(Claude202604Semantics),
        WorktreeincludeSemantics::Wt039 => Box::new(Wt039Semantics),
    }
}
