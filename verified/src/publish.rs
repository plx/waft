//! Verified kernel: destination publication protocols.
//!
//! This module decides every step of waft's descriptor-anchored publication
//! (production: `copy_file_anchored_unix` and the functions it drives in
//! `src/fs.rs`): which checks must pass before anything is published; whether
//! a missing destination is published by `RENAME_NOREPLACE` or a hard link,
//! and whether the extra name may then be unlinked; whether an existing
//! destination is replaced by an atomic exchange, undone, or displaced and
//! restored; whether an in-place permission repair may run, be reverted, or be
//! reported; and which single inode each staging or displaced name may ever be
//! unlinked for.
//!
//! # Shape
//!
//! Each protocol is a step machine written as typestates. Production performs
//! exactly the probe or syscall a state names, feeds back the answer, and
//! receives the next state, so any other order fails to compile and the
//! sequence of syscalls is the one the machine asked for. Errors that only
//! abort are returned unchanged (`Result<Next, E>`), and the error type is
//! generic, so production's `io::Error` values — and therefore its messages and
//! error kinds — pass through untouched. Every state keeps a ghost trace of
//! what it asked for ([`trace::Ev`]); traces and specifications are erased from
//! ordinary builds.
//!
//! * [`ordering`]: the pre-publication checks, in order, yielding the
//!   [`ordering::Ready`] token every publication protocol consumes.
//! * [`removal`]: the proven removal (probe, then unlink only on proof) and
//!   the guard state of a staging or displaced name.
//! * [`noreplace`]: no-clobber publication with its hard-link fallback.
//! * [`replace`]: replacement by exchange, its undo, and displacement.
//! * [`repair`]: in-place permission repair.
//! * [`trace`]: the event alphabet and the safety properties.
//! * [`model`]: the abstract filesystem and the filesystem-level theorems.
//!
//! # Theorems (all machine-checked by Verus)
//!
//! * Trace safety (`trace::safe`, a type invariant of every replacement state
//!   and guaranteed for every finished publication): every unlink is issued
//!   right after a probe proved the name holds an inode the run is entitled to
//!   remove, and the destination is never unlinked; hard links are only
//!   `RENAME_NOREPLACE` fallbacks; every exchange is issued right after proving
//!   what the destination holds; the destination is moved aside only after an
//!   unsupported exchange and a fresh re-proof.
//! * No-clobber publication, `noreplace::theorem_noreplace` (NR1–NR4): the
//!   result is an error exactly when nothing was published, and `Consumed`
//!   exactly when the source name is gone; cleanup unlinks only after the
//!   publication's own link, and never without an identity.
//! * Replacement (X1–X6, D1–D7 in [`replace`]): `Replaced` means the planned
//!   inode was verified after it was moved off the destination and then
//!   removed by a proven unlink; the clean failure verdicts mean the names they
//!   say are gone really were consumed; every failed proof disarms the guard.
//! * Guard (in [`removal`]): never re-armed, at most one removal, always
//!   entitled to exactly its current identity, re-aimed only by the replacement
//!   machine right after proving the name holds the new inode.
//! * Repair (R1–R5 in [`repair`]): the read-back verdict with content
//!   precedence; the revert mode is exactly `expected & 0o7777`; a reported
//!   repair is exactly checks → `fchmod` → intact read-back → name proof →
//!   `fsync`, with no namespace change at all.
//! * Ordering (O1–O4 in [`ordering`]): no destination mutation can be issued
//!   before the pre-publication callback returned `Ok`, and the staging name
//!   is proved last.
//! * Filesystem level ([`model`]): T1 — under the A-window and A-fresh axioms,
//!   any inode a waft step makes vanish is one it just proved and was entitled
//!   to remove, never a third writer's file; T2 — whatever the destination
//!   holds survives every waft step; NR5 — a no-clobber publication never
//!   changes an occupied destination; T3 — a repair changes no name; plus a
//!   witness that T1's hypotheses are satisfiable by a real replacement.
//!
//! # Trust boundary
//!
//! What the proofs assume production does faithfully:
//!
//! * performs exactly the syscall or probe each state names, on the names and
//!   identities the state hands out, and feeds back its true result (a probe
//!   that cannot be completed is fed back as "does not hold");
//! * calls the evidence steps (`replace::Exchange::after_destination_verified`,
//!   `replace::Reprove::reproved`, and every `Result`-taking check step) with
//!   the real outcome of the check each one names, and calls the
//!   pre-publication callback once;
//! * implements `trace::Identity::same_inode` as a comparison of
//!   `(st_dev, st_ino)` — the same comparison its identity probes make — and
//!   pins the staging guard to the `fstat` of the file it just created with
//!   `O_EXCL`;
//! * classifies a rename's errno as "flag unsupported" with its errno table;
//! * runs a guard's `Drop` on early return and unwinding (Verus does not model
//!   unwinding);
//! * and the POSIX semantics written as ghost definitions in [`model`], with
//!   its two axioms (A-window, A-fresh).

#[allow(unused_imports)]
use vstd::prelude::*;

pub mod model;
pub mod noreplace;
pub mod ordering;
pub mod removal;
pub mod repair;
pub mod replace;
pub mod trace;

verus! {} // verus!
