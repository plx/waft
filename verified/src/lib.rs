//! Verus-verified decision kernels for waft.
//!
//! Every function in this crate is written inside a `verus!` block and is
//! checked by `cargo verus verify -p waft-verified` (see
//! `scripts/verus-verify.sh`). Ordinary `cargo build` erases the
//! specifications and proofs, so the production binary runs exactly the
//! executable code that was verified.
//!
//! The crate has no I/O and no dependencies beyond `vstd`. The production
//! crate converts its own types into the plain data these kernels accept and
//! acts on the decisions they return. `VERIFICATION.md` at the repository
//! root records what each kernel proves, what it trusts, and which parts of
//! waft remain outside the verified boundary.

#![forbid(unsafe_code)]
#![allow(missing_docs)]

#[allow(unused_imports)]
use vstd::prelude::*;

pub mod boundary;
pub mod components;
pub mod context;
pub mod exclusion;
pub mod execute;
pub mod fingerprint;
pub mod fs_checks;
pub mod git_parse;
pub mod git_policy;
pub mod groups;
pub mod index_lock;
pub mod lint;
pub mod path;
pub mod planner;
pub mod policy;
pub mod publish;
pub mod report_view;
pub mod rules;
pub mod selection;
pub mod tally;
pub mod tracked;
pub mod validation;
pub mod worktree_map;
