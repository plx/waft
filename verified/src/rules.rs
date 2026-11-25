//! Verified kernels for `.worktreeinclude` rule semantics.
//!
//! - [`precedence`]: how per-file match results combine into the final
//!   verdict of the `git` engine (and of the root-only `claude-2026-04`
//!   engine), including both negation caveats and the reported provenance.
//! - [`reference`]: a per-level model relating those verdicts to Git's own
//!   per-directory semantics, with the proven agreements and the concrete
//!   divergences of the current behavior.
//! - [`lines`]: which rule lines become patterns, the line provenance
//!   lookup, and the wt-0.39 literal-negation grammar.

pub mod lines;
pub mod precedence;
pub mod reference;
