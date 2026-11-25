//! Verified kernel: per-file copy-plan decisions.
//!
//! * [`destination`] classifies one destination path (`DestinationState`)
//!   as a staged decision procedure that names each filesystem query in
//!   production's order and stops as soon as the answer is decided.
//! * [`entry`] decides one eligible path's planned entry (copy, no-op, skip
//!   or failure) the same way, carrying the observed snapshots and errors
//!   into the plan. Its theorems include: a tracked destination is never
//!   planned for writing, nothing existing is replaced without
//!   `--overwrite` on a platform that supports it, every copy carries the
//!   observed source snapshot, and a lost source is a failure, not a skip.
//!
//! Both are driven by `src/planner.rs`, which performs each query the kernel
//! names and records the answer. Each submodule documents its theorems and
//! trust assumptions.

#[allow(unused_imports)]
use vstd::prelude::*;

pub mod destination;
pub mod entry;

verus! {} // verus!
