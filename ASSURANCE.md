# Copy safety and its limits

This record covers the standalone waft CLI and its local-developer use case:
copying selected ignored regular files between worktrees while Git, editors,
and build tools may also be active. It is not a multi-file transaction, a
backup system, or protection against an unrestricted hostile process running
as the same user.

The September 6 review covered `src/fs.rs`, `src/executor.rs`, `src/git.rs`, and
the planner/model interfaces. It preserved profile selection, skip exit codes,
no-clobber by default, and tracked-name protection. It corrected two reproduced
failures: an error omitted one of two surviving recovery paths, and an
interrupt during lock release could remove the next Git writer's lock.

| Behavior | Implementation and concrete evidence | Reach |
| --- | --- | --- |
| Existing differing destinations survive default copies. A name appearing after planning is not clobbered. | Planner skips conflicts; `publish_noreplace` refuses occupied names. `copy_skips_untracked_conflict_without_overwrite`, `realfs_missing_copy_never_clobbers_path_that_appeared`. | CLI integration on each CI OS; anchored publication on Unix. |
| Explicit overwrite replaces only a destination that still matches its recorded state. | Descriptor identity, mode, length and content fingerprint checks before exchange, then displaced-state verification and conditional rollback. `realfs_overwrite_refuses_a_destination_rewritten_during_publication`, `realfs_overwrite_does_not_undo_a_swap_onto_a_third_writers_file`. | Unix tests; exchange supported by the host filesystem. |
| Unsupported rename flags take a recoverable fallback. | Move aside, no-clobber publish, checked cleanup/restore. `realfs_overwrite_replaces_without_exchange_or_rename_noreplace`, `realfs_overwrite_without_exchange_keeps_both_files_when_the_name_reappears`, `realfs_overwrite_reports_both_staging_and_displaced_leftovers`. | Deterministically injected unsupported/error results exercise real remaining syscalls. These are not field tests of SMB, NFS, or exFAT. |
| Permission repair keeps content, checks the repaired mode and name, and reports concurrent changes. | Descriptor `fchmod` plus revalidation. `copy_overwrite_repairs_permission_only_destination_without_rewriting_it`, `realfs_permission_repair_reports_a_mode_changed_after_the_chmod`, `realfs_permission_repair_refuses_when_the_destination_name_is_taken_after_the_chmod`. | Unix mode bits; no general promise to preserve ownership, ACLs, timestamps, or other metadata. |
| Tracked names are checked again while holding Git's index lock. Cached lookup sees a replaced index. | `execute_refuses_path_that_became_tracked_after_planning`, `execute_holds_destination_index_lock_through_publication`, `a_reused_backend_instance_observes_a_path_becoming_tracked`. | Both gix and Git CLI backends; normal Git writers honoring `index.lock`. |
| Vanished/changed sources fail per file; symlink swaps do not redirect Unix copies. | Source snapshots and descriptor-relative traversal. `plan_source_that_vanished_before_the_type_check_fails_only_that_file`, `realfs_rejects_source_ancestor_replaced_after_planning`, `realfs_rejects_source_replaced_by_fifo_without_blocking`. | Unix traversal tests and platform-independent planning tests. |
| SIGINT/SIGTERM release owned locks without deleting the next writer's lock. | Creation/release deferral and descriptor-identity cleanup. `real_interrupts_clean_owned_locks_and_preserve_foreign_locks` terminates subprocesses with each real signal during creation, publication, release, a foreign acquisition, and a replaced lock; checks destination bytes, lock contents, and termination status. `normal_lock_release_preserves_a_replaced_foreign_lock` covers normal release. | Unix; inherited ignored dispositions are separately tested. |

## Operational limits

- Each file is an independent operation. Earlier successful files remain when a
  later file fails. An error after publication can mean the destination already
  contains the new bytes; recovery errors name surviving files to inspect.
- Exchange gives atomic visibility for one file. Displacement has a brief vacant
  destination window and can leave recovery files if restore fails. File and
  directory syncing reduces durability risk; no power-loss recovery is promised.
- An open directory can be relocated after final pathname revalidation. Directory
  handles prevent redirection to another object, but cannot promise that its
  original pathname remains stable. Avoid concurrent directory relocation.
- Content fingerprints are bounded-memory, non-cryptographic 64-bit concurrency
  guards, not adversarial proofs of byte identity. Checks cannot prevent a writer
  changing a file after the last observation. Permission rollback is best effort.
- POSIX has no conditional unlink. Recovery and lock cleanup still have a window
  between identity verification and removal. Staging/displacement names use fresh
  128-bit random nonces; initial reflink-allocation cleanup also relies on those
  names being private to this run. Do not manipulate `.waft-copy-*` files during a
  run. No hostile same-user pathname-race guarantee is made.
- Index caching relies on normal Git index replacement. Direct index edits that
  ignore the lock are outside the contract. `core.ignoreCase` is fixed per run;
  unrelated hard-linked pathnames are treated as distinct paths. The review did
  not change #27's cache or rerun its historical performance benchmark.
- Signals terminate a process without Rust unwinding: staging/recovery files can
  survive SIGINT/SIGTERM. SIGKILL, crashes, filesystem errors, and non-Unix
  termination can also leave locks. Inspect recovery files and establish that no
  Git writer is active before removing a stale lock. Never bulk-delete recovery
  files merely because their names begin `.waft-copy-`.
- Windows has no supported overwrite/permission-repair path, Unix descriptor
  anchoring, or Unix signal cleanup. Its CI covers supported creation/reporting
  behavior; a passing Windows job does not establish Unix safety guarantees or
  Windows release-binary availability.

Fresh local checks use Rust 1.90 on macOS arm64. Cross-platform results must be
read from CI for the exact reviewed commit; a skipped matrix is not a platform
pass. Optional external-agent compatibility tests and physical network/removable
filesystem tests are not part of this evidence.
