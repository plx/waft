//! waft — `.worktreeinclude` file fixer.
//!
//! `waft` is a CLI tool for copying local ignored files from a repository's
//! main worktree into a linked worktree, guided by `.worktreeinclude` files
//! that use `.gitignore` syntax.
//!
//! # Architecture
//!
//! The tool follows a strict plan/execute design:
//!
//! 1. **Policy resolution** ([`config`]) — merges built-in defaults, user
//!    config, project `.waft.toml` files, environment variables, and CLI
//!    flags into a single [`config::ResolvedPolicy`].
//! 2. **Context resolution** ([`context`]) — determines source and destination
//!    worktrees from CLI args and Git state.
//! 3. **Validation** ([`validate`]) — checks all `.gitignore`, `.worktreeinclude`,
//!    and exclude files for syntax errors.
//! 4. **Candidate selection** (in [`subcommands`]) — dispatches on whether
//!    any `.worktreeinclude` exists, runs the matcher engine chosen by
//!    [`config::WorktreeincludeSemantics`] (see [`worktreeinclude_engine`]),
//!    and verifies eligibility via the Git backend's `check_ignore`. The
//!    post-selection filter ([`policy_filter`]) drops paths matched by the
//!    active built-in or extra exclusion patterns.
//! 5. **Planning** ([`planner`]) — classifies each destination path (missing,
//!    up-to-date, conflict, etc.) and produces a [`model::CopyPlan`].
//! 6. **Execution** ([`executor`]) — applies copy operations atomically via
//!    temp files, with symlink safety checks.
//!
//! # Key Design Decisions
//!
//! - The Git backend ([`git`]) is authoritative for ignore membership and
//!   trackedness. Two interchangeable implementations are provided: `GitGix`
//!   (default, in-process via the `gix` crate) and `GitCli` (selected via
//!   `WAFT_GIT_BACKEND=cli`); backend parity tests pin them to the same
//!   observable behavior.
//! - `.worktreeinclude` is matched by a pluggable engine selected by the
//!   active compat profile. The default `claude-2026-04` engine reads only
//!   the repository's root-level rule file; the `git` engine reproduces
//!   Git's per-directory exclude semantics; the `wt-0.39` engine implements
//!   `worktrunk 0.39`'s subtractive literal-name negations.
//! - The `ignore` crate is used for parsing and explanation, not as the
//!   final authority on Git-ignored status.
//! - Discovery never mutates the filesystem; only `copy` (without `--dry-run`)
//!   writes files.

#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

pub mod cli;
pub mod config;
pub mod context;
pub mod eligibility_groups;
pub mod error;
pub mod executor;
pub mod fs;
pub mod git;
pub mod model;
pub mod path;
pub mod planner;
pub mod policy_filter;
/// Subcommand argument types and command handlers.
pub mod subcommands;
mod sys;
pub mod validate;
mod walk;
pub mod worktreeinclude;
pub mod worktreeinclude_engine;
