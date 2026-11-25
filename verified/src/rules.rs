//! Verified kernels for `.worktreeinclude` rule semantics.
//!
//! - [`precedence`]: how per-file match results combine into the final
//!   verdict of the `git` engine (and of the root-only `claude-2026-04`
//!   engine), including both negation caveats and the reported provenance.

pub mod precedence;
pub mod reference;
