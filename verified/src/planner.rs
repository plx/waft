//! Verified kernel: per-file copy-plan decisions.
//!
//! * [`destination`] classifies one destination path (`DestinationState`)
//!   as a staged decision procedure that names each filesystem query in
//!   production's order and stops as soon as the answer is decided.
//!
//! Each submodule documents its theorems and trust assumptions.

#[allow(unused_imports)]
use vstd::prelude::*;

pub mod destination;
pub mod entry;

verus! {} // verus!
