# Changelog

All notable changes to waft will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0](https://github.com/plx/waft/releases/tag/v0.1.0) - 2026-09-07

First supported release, from commit
`eaa4198883b9a54167e4d006cf3beb721b04d694`. The entries below describe changes
made during development before this release.

### Security

- Preserve a foreign index lock when a signal arrives during release or the
  lock pathname is replaced: defer termination across the release window and
  compare cleanup with the still-open owned lock.
- Report both recovery paths and both cleanup errors when replacement succeeds
  but the displaced original and a staging name both survive cleanup.

- Protect tracked destination paths using filesystem identity and normalized
  case matching, including case-insensitive macOS aliases.
- Reject an unrecognized `WAFT_GIT_BACKEND` value instead of silently using the
  default backend, so a typo cannot change which implementation enforces the
  tracked-path and repository-boundary checks. Valid values are `gix` and
  `cli`, trimmed and matched without regard to ASCII case.
- Hold Git's cooperative index lock across the final tracked-state check and
  no-clobber publication.
- Anchor Unix source and destination traversal to directory descriptors,
  refuse symlink components, verify the planned source state, and publish with
  descriptor-relative `NOREPLACE`.
- Replace destinations under `--overwrite` only after re-opening them through
  the anchored parent with `O_NOFOLLOW` and matching device, inode, length,
  content fingerprint, and mode against the planning snapshot. Replacement is
  an atomic exchange whose swapped-out file is re-checked against that same
  snapshot before it is unlinked, so an in-place rewrite of the same inode is
  detected rather than clobbered.
- Repair destination permissions only when the pinned source and destination
  snapshots agree on content, so a destination rewritten between the byte
  comparison that classified it and the snapshot that pinned it is never
  `chmod`-ed and reported as a successful repair while holding content that is
  not the source's. The pinned content is required once more after the `chmod`
  lands: a destination rewritten in that last window has the mode it was found
  with restored and is reported as a per-file failure instead of a repair. That
  same read-back compares the mode as well, so a destination another writer
  `chmod`-ed on top of waft's repair is also reported as a per-file failure
  rather than a repair the file no longer carries. Its mode is left exactly as
  that writer set it: waft does not answer a concurrent `chmod` with another
  one. The destination *name* is proved last, after both descriptor facts: a
  file another process renames onto that name while waft holds the planned
  inode open would leave every descriptor check passing on a file nobody can
  reach any more, and the visible destination unrepaired. Neither is chased —
  the file that took the name was never verified here and is not touched, the
  repaired inode is not followed — and the run reports a per-file failure
  instead of a repair.
- Report a replacement whose swapped-out file could not be removed as a
  per-file failure naming the full `.waft-copy-*` path it was left under,
  instead of returning success. The destination holds the planned content, but
  the file it replaced is still on disk and may still hold the secrets that
  were there before. The same holds where a filesystem has no
  `RENAME_NOREPLACE` and publication is a hard link followed by an unlink of
  the staging name: that unlink is conditional on proving the name still holds
  the file this run linked from, and a name another writer re-pointed in
  between is left alone — and now reported. Reporting success there disarmed
  the staging guard and called the copy created or replaced while an
  unexplained `.waft-copy-*` entry, holding whatever that writer put under the
  name, stayed on disk. The per-file failure says the destination itself is
  fine and names the full path of what was left.
- Never unlink a file waft has not just proven is the one it planned against.
  If a recovery step fails and strands another writer's file under a
  `.waft-copy-*` name, or strands the prepared replacement because the
  destination name is held by somebody else, the file is kept and named in the
  per-file error instead of being cleaned up. Undoing a lost race is held to
  the same standard: the identity of the file this run wrote is pinned before
  the exchange, and both the undo swap and the cleanup that follows it are
  performed only after re-proving that the name they act on still holds that
  file. A third writer taking the destination name mid-undo would otherwise be
  swapped under the temporary name and deleted there; instead nothing is moved
  or removed, and the error names the full path of every file left behind.
- Replace destinations on filesystems without an atomic exchange — the
  documented SMB, NFS, and exFAT fallback — by moving the destination aside
  with a plain rename and publishing into the name it left, instead of
  unlinking that name and publishing over it. The unlink was by pathname: a
  writer that published its own file over the destination between the
  descriptor proof and the unlink had that brand-new file deleted. A rename
  deletes nothing, so the displaced file can be identified afterwards. If it is
  the inode that was planned against, the replacement is published with
  no-clobber semantics and the displaced file is unlinked only after re-proving
  it still holds that inode; if it is not, it is moved back with a no-clobber
  restore and the file is reported as changed during publication; if it cannot
  be moved back — the destination name has been taken again, or the filesystem
  has no atomic no-clobber move — it is kept under the name it was displaced to
  and the error names it. The residual is a concurrent writer's file being
  briefly displaced and restored: for a few syscalls the destination name does
  not resolve and that file carries a `.waft-copy-*.displaced` name. It is
  never modified and never deleted. On this path the previous destination is no
  longer "already gone" after a failed publication; it is on disk under one of
  the two names the error spells out.
- Remove `.waft-copy-*` staging files from a drop guard so an unwinding panic
  during publication cannot leave them behind — and only while the name still
  holds the file this run created under it, whose identity the guard pins from
  the open descriptor at creation. The staging name is visible for the whole
  publication window, so cleanup, from `Drop` as well as from the explicit
  removal, re-proves the inode first; a name another process re-pointed is left
  alone, silently on the way out of a panic and as a per-file error naming the
  file otherwise. POSIX has no conditional unlink, so a two-syscall window
  between the proof and the `unlinkat` remains on a fresh 128-bit random name
  nothing but waft creates; that residual is irreducible rather than closed.
- Unlink the live Git `index.lock` from `SIGINT`/`SIGTERM` handlers on Unix
  before re-raising under the previous disposition, so an interrupt during the
  publish window cannot leave a stale lock. A signal the process inherited as
  ignored (`nohup`, background jobs without job control) is left ignored:
  handling it would delete the live lock and then return into the publish
  window without it. The disposition is queried before anything is installed,
  so an inherited ignore is never waft's even momentarily; installing first and
  putting the ignore back afterwards left a window in which a delivery ran
  waft's handler before it had recorded what to restore, fell back to
  `SIG_DFL`, and terminated a process that was started to ignore the signal
  outright. A signal arriving while the lock is being created — after
  `create_new` may have produced the file but before cleanup is armed — is
  recorded and deferred rather than re-raised: the handler cannot tell whether
  a lock exists yet or whether it is waft's, and terminating there would leave
  an `index.lock` that blocks every later Git and waft operation. The
  acquisition acts on the record as soon as it knows, removing only a lock this
  process created and then re-raising, so no deferred signal is lost and no
  other writer's lock is touched. Windows and other non-Unix targets have no
  such handler; an interrupt there can still leave `.git/**/index.lock` behind
  for the user to delete.
- Install the optional Git hook and a reviewed waft binary outside checked-out
  worktrees, while chaining only regular-file trusted hooks, rejecting
  per-worktree overrides, and ignoring ambient executable overrides at run
  time.
- Add dependency auditing, pinned CI actions, and artifact provenance.

### Changed

- **`--overwrite` now replaces files instead of aborting the run.** The
  previous behavior — rejecting the entire plan with an "cannot safely
  replace" error the moment any untracked conflict was found — is gone,
  together with the `UnsafeOverwrite` error variant. `--overwrite` now
  performs a race-safe per-file replacement, and a conflict it cannot prove
  safe is a per-file failure rather than a whole-run abort. Tracked
  destinations remain untouchable under every flag combination.
- Classify "destination content equal, permissions differ" separately from a
  generic untracked conflict in plans, `--dry-run`, `info`, `list`, and skip
  reporting, and name `--overwrite` as its remedy. This is the migration path
  for files published by earlier waft builds, which always wrote mode `0600`
  and would otherwise be permanent conflicts.
- Decide whether `--overwrite` can act on an existing destination while
  planning. On platforms without the anchored replacement path — currently
  every non-Unix target — such a file is one per-file failure reported
  identically by `--dry-run` and the executed run, with the same nonzero exit,
  rather than a plan that promised a replacement and only failed at
  publication. Skip, `info`, and `list` output there describes the same
  conflict without naming `--overwrite` as its remedy.
- Report replacements and permission repairs distinctly from creations
  (`replaced:` and `repaired permissions:` lines, with matching summary
  clauses that appear only when non-zero).
- Treat an unreadable or vanished source as a per-file failure during planning
  instead of aborting the whole run with nothing copied. The failure is
  reported, counted, and reflected in the exit status; every other file still
  proceeds. `--dry-run` reports the same failures on stderr — including under
  `--quiet` — and exits nonzero like the run it describes. This covers the
  first look at the source as well as the snapshot that follows it: a path that
  was eligible when discovery listed it and cannot be examined at all when
  planning reaches it is that same per-file failure, not an "unsupported source
  type" skip that would drop the file from the run and still exit zero. A
  source that was examined and simply is not a regular file — a directory, a
  symlink, a device — remains an ordinary skip.
- Update `scripts/self-test.sh` to pin the new `--overwrite` contract:
  untracked conflicts are replaced and the run succeeds, while a tracked
  destination is still never written.
- Retry destination Git index-lock acquisition three times over roughly 150ms
  so a transient index writer (IDE, fsmonitor, background `git status`) is not
  a sporadic per-file failure. A lock still held after that fails the file with
  an explicit message and is never removed.
- Fall back to the `linkat` publication path on `ENOTSUP`/`EOPNOTSUPP` as well
  as `ENOSYS`/`EINVAL`, fixing hard per-file failures on macOS SMB, NFS, and
  exFAT destinations.
- Use one eligibility calculation for copy, list, info, and dry-run behavior.
- Make the gix and Git CLI backends share selection semantics and exercise all
  compatibility profiles through both.
- Preserve file permissions, compare large files with bounded memory, and copy
  selected directory trees as individually checked file operations.
- Resolve project configuration from the trusted source worktree and add
  `--isolated` for managed operation.
- Make profile selection reset its coordinated knobs at that layer while
  preserving same-layer and higher-precedence explicit overrides.
- Prefer immediate per-file index rechecks over a long-held batch lock. Each
  recheck now reuses one index snapshot per repository, revalidated by a single
  `stat` of the index file, so its cost no longer scales with index size: the
  Git CLI backend answers all tracked-state questions for a run from one
  `ls-files` rather than spawning subprocesses per published file under the
  destination index lock.
- Bound tracked-path lookup to the colliding folded-name entries. Deciding
  whether a candidate is tracked is a hash lookup plus, only on a case-folding
  collision, a filesystem-identity check against the 0-1 index entries that
  collide — replacing a scan that could stat every index entry per query.
- Decide case sensitivity from the repository's own `core.ignoreCase` rather
  than assuming macOS and Windows always fold case. On a case-sensitive volume
  configured `core.ignoreCase = false`, a distinct untracked file whose name
  folds onto a tracked one stays eligible, and on platforms that never alias
  case a directory such as `Vendor/` is no longer treated as the registered
  `vendor/` submodule boundary. Folded protection is kept when the key is
  true, and when the key is absent the platform default still decides.
  Repository-boundary and config-discovery comparisons, like exclusion
  pattern matching, keep a deliberately more conservative rule: on macOS and
  Windows they stay folded regardless of the key, because those filesystems
  may alias case anyway and walking into a registered submodule (or trusting
  its config) is the failure that must not happen, while a spurious boundary
  only leaves one directory unscanned.
- Resolve `core.ignoreCase` once per repository per run, so a run cannot apply
  folded protection to some paths and exact matching to others, and so the
  per-file tracked-state recheck does not repeat a config lookup under the
  destination index lock. Editing the key mid-run is not observed by that run.
- Answer `info`'s "is this path a filesystem alias of an eligible one?"
  question with one directory resolution per queried path instead of one per
  eligible entry. On a repository with 3200 eligible files, `waft info` on a
  tracked, non-eligible path drops from 5.6s to 0.9s.
- Normalize worktree paths reported by the Git CLI backend, matching the gix
  backend. A linked worktree whose recorded path holds a symlinked spelling is
  now classified and anchored the same way under either backend.
- Declare Rust 1.90 as the minimum supported Rust version.
- Restrict source packages to an explicit allowlist.
- Document installation from a reviewed Git revision; waft is not yet
  published to crates.io.
- **(breaking) Selecting a compat profile now resets that profile's own knobs
  from lower-precedence layers.** Previously a `when_missing`, `semantics`,
  symlink-policy, or `builtin_exclude_set` value set in any earlier layer
  survived a later `--compat-profile`, so a profile could be selected and
  silently not applied; now choosing a profile installs its coordinated knob
  values at that layer — including the built-in exclude set, which changes
  which files are selected — and only same-layer or higher-precedence
  explicit settings override them.
- **(breaking) Destination equality now includes permission bits.** A
  destination whose content matched the source used to count as up-to-date
  regardless of its mode; it is now classified as "content equal, permissions
  differ", reported as a skip that names `--overwrite` as the remedy, and
  repaired in place (without rewriting content) when that flag is supplied.

### Added

- Name every skipped file and its reason in an executed `copy` run, in the
  same words `--dry-run` uses (for example
  `skip: .env (untracked conflict; --overwrite replaces the destination)`).
  Skips remain exit status 0; `--quiet` suppresses the lines along with the
  rest of the non-error output.
- Print a one-line note on stderr when the active configuration selects
  nothing and no `.worktreeinclude` was consulted, so `waft list` and a bare
  `waft` in an unconfigured repository are no longer silent. The note names
  whatever is responsible: an absent rule file, a rule file outside the
  repository root under root-only `claude-2026-04` semantics, a symlinked rule
  file skipped by `--worktreeinclude-symlink-policy ignore`, or an explicit
  `--when-missing-worktreeinclude blank` rather than the profile it overrode.
  A consulted rule file that legitimately selects nothing does not emit it,
  Symlink hints describe skipped entries without opening their targets or
  promising that a different policy will validate or select files.
  The note is also absent for a configuration that still selects without one (`wt`,
  `--when-missing-worktreeinclude all-ignored`), nor `--quiet`.

### Fixed

- Report running outside a repository as `not inside a Git repository
  (searched from <path>)` instead of the doubly prefixed, internal-sounding
  `error: git error: gix failed to discover repository from …`. The backend's
  own diagnostics remain in the error source chain and are printed under
  `-v`/`--verbose`.

### Removed

- Removed the wide filesystem-identity scan from tracked-path lookup, an
  accepted narrowing of protection. Previously a candidate whose name matched
  no tracked name could still be reported as tracked if it resolved to the same
  filesystem object as *any* index entry, at a cost of up to one `stat` per
  index entry per query. Two pathnames a filesystem aliases without a Unicode
  case-folding or exact-byte match — most concretely, a deliberately
  hard-linked second name for a tracked file — are now treated as distinct
  paths. Case aliases, including the ones whose folding the platform performs
  natively, remain protected: they collide under folding and are confirmed by
  identity against just the colliding entries.
