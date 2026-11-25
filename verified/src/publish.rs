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
//! the probe or syscall a state names, feeds back the answer, and receives the
//! next state. The typestates fix the order in which answers can be fed back:
//! each step consumes its state, so code that skips or reorders a step does
//! not compile. They do not gate the syscalls themselves — nothing stops
//! unverified code from issuing a syscall the machine did not ask for, or from
//! feeding a step an answer it made up — so "production issues exactly the
//! named syscall at that point and feeds back its real result" is part of the
//! trust boundary below, not a compile-time guarantee.
//!
//! Errors that only abort are returned unchanged (`Result<Next, E>`), and the
//! error type is generic, so production's `io::Error` values — and therefore
//! its messages and error kinds — pass through untouched. Every state keeps a
//! ghost trace of what it asked for ([`trace::Ev`]); traces and specifications
//! are erased from ordinary builds.
//!
//! A sub-protocol a machine hands out (a proven removal, a no-clobber
//! publication) carries the state waiting for it, so its finished record can
//! only be fed back to that state; no step takes a sub-protocol record or a
//! guard under a precondition production would have to establish.
//!
//! * [`ordering`]: the pre-publication checks, in order, yielding the
//!   [`ordering::Ready`] token every publication protocol consumes, and the
//!   staging guard created with them.
//! * [`removal`]: the proven removal (probe, then unlink only on proof) and
//!   the guard state of a staging or displaced name.
//! * [`noreplace`]: no-clobber publication with its hard-link fallback.
//! * [`replace`]: replacement by exchange, its undo, and displacement.
//! * [`repair`]: in-place permission repair.
//! * [`trace`]: the event alphabet and the safety properties.
//! * [`model`]: the abstract filesystem and the filesystem-level theorems.
//!
//! # Theorems (machine-checked by Verus)
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
//!   exactly when the source name is gone; cleanup unlinks only right after
//!   the publication's own link and a probe for the publication's identity,
//!   and never without an identity. A publication of a missing destination
//!   disarms the staging guard exactly when something was published.
//! * Replacement (X1–X6, D1–D7 in [`replace`]): `Replaced` means the planned
//!   inode was verified after it was moved off the destination and then
//!   removed by a proven unlink of exactly that inode; every displacement
//!   verdict says exactly which names the run consumed and carries exactly
//!   the leftover reasons and errors that produced it; every failed proof
//!   disarms the guard.
//! * Guard (in [`removal`]), per guard state value: its name and identity never
//!   change, it is never re-armed, a drop asks for at most one removal, and
//!   every removal it hands out is entitled to exactly its name and identity.
//! * Repair (R1–R5 in [`repair`]): the read-back verdict with content
//!   precedence; the mode applied is the planned source's; the revert mode is
//!   exactly `expected & 0o7777`; a reported repair is exactly checks →
//!   `fchmod` → intact read-back → name proof → `fsync`, with no namespace
//!   change at all.
//! * Ordering (O1–O4 in [`ordering`]): the order of the checks is Rust's
//!   (typestates); Verus proves each step passes errors through, is `Ok`
//!   exactly when its check was, and records exactly its own check, so the
//!   [`ordering::Ready`] token's trace is the six checks in order, with the
//!   pre-publication callback before the staging name's proof.
//! * Filesystem level ([`model`]): T1 — under the A-window and A-fresh axioms,
//!   any inode a waft step makes vanish is one it just proved and was entitled
//!   to remove; T1′ — with the A-link axiom as well, that inode is always the
//!   run's own staging file or the verified planned destination, never a third
//!   writer's file; T2 — whatever the destination holds survives every waft
//!   step (from consistent answers alone); NR5 — a no-clobber publication never
//!   changes an occupied destination; T3 — a repair changes no name; plus a
//!   witness that T1's hypotheses are satisfiable by a real replacement.
//!
//! # Trust boundary
//!
//! What the proofs assume production does faithfully:
//!
//! * issues exactly the syscall or probe each state names, at that point, on
//!   the names and identities the state hands out, and feeds back its true
//!   result (a probe that cannot be completed is fed back as "does not hold");
//!   states, including [`ordering::Ready`] and a removal's `Removed` outcome,
//!   can be produced by feeding made-up answers, so they are evidence only of
//!   the answers fed;
//! * calls the evidence steps (`replace::Exchange::after_destination_verified`,
//!   `replace::Reprove::reproved`, and every `Result`-taking check step) with
//!   the real outcome of the check each one names, and calls the
//!   pre-publication callback once;
//! * implements `trace::Identity::same_inode` as a comparison of
//!   `(st_dev, st_ino)` — the same comparison its identity probes make — and
//!   creates the staging guard ([`ordering::Staged::created`]) with the `fstat`
//!   of the file it just created with `O_EXCL`;
//! * keeps one guard state per guarded name for the whole publication and
//!   never replaces it with another value: the guard guarantees above are
//!   about each guard state value, and a field holding one can always be
//!   overwritten by unverified code;
//! * classifies a rename's errno as "flag unsupported" with its errno table;
//! * runs a guard's `Drop` on early return and unwinding (Verus does not model
//!   unwinding);
//! * and the POSIX semantics written as ghost definitions in [`model`], with
//!   its axioms (A-window, A-fresh; A-link for T1′ only).

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
