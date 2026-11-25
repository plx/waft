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
