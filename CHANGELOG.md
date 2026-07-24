# Changelog

All notable changes to waft will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Until the first supported release, changes remain under `Unreleased`.

## Unreleased

### Security

- Protect tracked destination paths using filesystem identity and normalized
  case matching, including case-insensitive macOS aliases.
- Hold Git's cooperative index lock across the final tracked-state check and
  no-clobber publication.
- Anchor Unix source and destination traversal to directory descriptors,
  refuse symlink components, verify the planned source state, and publish with
  descriptor-relative `NOREPLACE`.
- Retain `--overwrite` parsing for compatibility while failing before mutation
  when it would replace an existing untracked pathname.
- Install the optional Git hook and a reviewed waft binary outside checked-out
  worktrees, while chaining only regular-file trusted hooks, rejecting
  per-worktree overrides, and ignoring ambient executable overrides at run
  time.
- Add dependency auditing, pinned CI actions, and artifact provenance.

### Changed

- Use one eligibility calculation for copy, list, info, and dry-run behavior.
- Make the gix and Git CLI backends share selection semantics and exercise all
  compatibility profiles through both.
- Preserve file permissions, compare large files with bounded memory, and copy
  selected directory trees as individually checked file operations.
- Resolve project configuration from the trusted source worktree and add
  `--isolated` for managed operation.
- Make profile selection reset its coordinated knobs at that layer while
  preserving same-layer and higher-precedence explicit overrides.
- Prefer immediate per-file index rechecks over a long-held batch lock; large
  manifests in very large indexes should be benchmarked before automation.
- Declare Rust 1.90 as the minimum supported Rust version.
- Restrict source packages to an explicit allowlist.
- Document installation from a reviewed Git revision; waft is not yet
  published to crates.io.
