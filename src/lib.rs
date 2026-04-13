//! wiff — `.worktreeinclude` file fixer.
//!
//! `wiff` is a CLI tool for copying local ignored files from a repository's
//! main worktree into a linked worktree, guided by `.worktreeinclude` files
//! that use `.gitignore` syntax.
//!
//! # Architecture
//!
//! The tool follows a strict plan/execute design:
//!
//! 1. **Context resolution** ([`context`]) — determines source and destination
//!    worktrees from CLI args and Git state.
//! 2. **Validation** ([`validate`]) — checks all `.gitignore`, `.worktreeinclude`,
//!    and exclude files for syntax errors.
//! 3. **Candidate enumeration** — uses `git ls-files --exclude-per-directory` to
//!    find `.worktreeinclude`-selected files, then `git check-ignore` to verify
//!    they are actually Git-ignored.
//! 4. **Planning** ([`planner`]) — classifies each destination path (missing,
//!    up-to-date, conflict, etc.) and produces a [`model::CopyPlan`].
//! 5. **Execution** ([`executor`]) — applies copy operations atomically via
//!    temp files, with symlink safety checks.
//!
//! # Key Design Decisions
//!
//! - Git CLI is authoritative for ignore membership and trackedness.
//! - `.worktreeinclude` is an independent matcher with Git-style per-directory
//!   semantics.
//! - The `ignore` crate is used for parsing and explanation, not as the oracle.
//! - Discovery never mutates the filesystem; only `copy` (without `--dry-run`)
//!   writes files.

#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

pub mod cli;
pub mod context;
pub mod error;
pub mod executor;
pub mod fs;
pub mod git;
pub mod model;
pub mod path;
pub mod planner;
/// Subcommand argument types and command handlers.
pub mod subcommands;
pub mod validate;
pub mod worktreeinclude;
