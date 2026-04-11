//! wiff — `.worktreeinclude` file fixer.
//!
//! A CLI tool for copying local ignored files from a repository's main worktree
//! into a linked worktree, guided by `.worktreeinclude` files that use
//! `.gitignore` syntax.

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
pub mod validate;
pub mod worktreeinclude;
