# Folder-Copy Fast Path: Implementation Plan

## Purpose

Today every copy goes through a per-file pipeline: `select_candidates` ->
`policy_filter::filter_paths` -> `git.check_ignore` -> `planner::plan` (one
`CopyOp` per file) -> `executor::execute` (one `fs.copy_file` per file).
That is correct, but it leaves performance on the table when a source
subtree can be copied as a unit.

This plan adds a conservative fast path for directories whose physical
source subtree is safe to copy wholesale and whose destination subtree does
not yet exist. The slow per-file path remains the fallback for every other
case.

The important constraint is exactness: the fast path is allowed only when
copying the whole physical source directory would produce the same observable
destination state as copying the planned files individually. If that invariant
is not provable, the planner expands the directory back into per-file work.

## Design Summary

- Compute fully copyable directories as a post-hoc transformation of
  `(eligible_set, source_tree)`, after candidate selection, policy filtering,
  and `git.check_ignore` have already run. This keeps the semantics engines
  unchanged.
- Treat a directory as fast-path eligible only if every physical descendant is
  safe to reproduce:
  - regular files must be in the eligible set,
  - directories must contain at least one eligible regular-file descendant,
  - symlinks, special files, empty directories, tracked source files,
    ignored-but-not-selected files, submodules, nested Git checkouts, and
    `.git` boundaries block promotion of all ancestors.
- Never fast-path the repository root. A normal source root contains `.git`;
  cloning it would copy repository metadata. This also preserves the
  `RepoRelPath` invariant that repo-relative paths are never empty.
- Before emitting `CopyDir`, the planner verifies that none of the files
  covered by the directory are tracked in the destination index. A missing
  destination directory does not prove this: a linked worktree may have
  tracked files deleted from disk.
- A `CopyDir` operation carries the manifest of files it covers. Execution
  may use a recursive COW primitive when the physical tree still exactly
  matches that manifest; otherwise it falls back to copying the manifest files
  into an atomic temporary directory and renaming it into place.
- The fast path activates only when the destination directory itself is
  missing and the destination parent path is not symlinked. Existing
  destinations, unsafe parents, and tracked destination conflicts all fall
  through to per-file planning.

## Correctness Invariants

### Source Exactness

For a directory `D` to be represented by one `CopyDir`, all descendants under
`D` must be explainable by the eligible file manifest.

This explicitly rules out:

- a submodule or nested repository under `D`;
- a `.git` directory or linked-worktree `.git` pointer under `D`;
- a symlink under `D`, even if it is selected by `.worktreeinclude`;
- an empty directory under `D`;
- any regular file under `D` that is not in the eligible set.

Those entries must block promotion of `D`, not be silently skipped. Candidate
enumeration skips Git boundaries to avoid leaking files from other repos, but
directory cloning cannot skip entries: if the physical entry is there, a
wholesale clone would copy it. The grouping algorithm must therefore treat
these skipped boundaries as blockers for the parent chain.

### Destination Exactness

`dst/D` being absent is necessary but not sufficient. Before `CopyDir` is
emitted, the planner must call `git.tracked_paths(dest_root, covered_files)`
and confirm the result is empty for the covered manifest. If any covered path
is tracked in the destination, the directory is expanded to per-file planning,
where the existing `TrackedConflict` behavior applies.

### Atomicity

Existing `copy_file` writes through a temp file in the destination directory
and renames into place. `copy_dir` must provide the same user-visible
property at directory granularity:

1. create/copy into a unique temporary sibling directory;
2. on success, atomically rename the temp directory to the final destination;
3. on failure, remove the temp directory and leave `dst/D` absent.

The executor creates missing destination parents before invoking the
primitive, after checking those parents do not contain symlinks.

### Empty Directories

The current per-file pipeline never creates empty source directories. A
directory containing an empty descendant is therefore not wholesale-safe. The
empty directory blocks promotion of its ancestors; eligible sibling files can
still flow through the normal per-file path or through smaller safe
subdirectories.

### Symlink Reporting

If a symlink path itself is selected and ignored, it must remain in
`remaining_files` so the existing planner emits `Skip(UnsupportedSourceType)`.
The symlink also blocks promotion of all ancestor directories.

## Idempotency Argument

`waft copy` must remain idempotent: running it twice must produce the same
filesystem state, with counts that reflect the same number of eligible files.

- First run, safe source dir and missing destination dir: planner emits one
  `CopyDir` covering `N` files. Executor reports the aggregate line as
  `copy-dir: <dir> (N files)` and increments `copied` by `N`.
- Second run, destination dir present: the fast-path gate fails, the directory
  expands to per-file planning, and each file classifies as `UpToDate`.
  Executor reports `N up-to-date`.

The filesystem state is the same as the per-file implementation. Only the
execution route differs.

## Execution Constraints

- Each PR builds and tests cleanly on its own; no PR is half-wired.
- Behavior change is concentrated in PR4. PRs 1-3 are plumbing and tests with
  no production planner emission.
- Do not introduce a config knob for the fast path. The gates above are
  conservative enough to ship default-on. If field evidence later requires an
  escape hatch, add `WAFT_DISABLE_COPY_DIR=1` in a separate change.

## Global Gate

Run at the end of every PR:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --doc
cargo test --test backend_parity
```

For PR4 onward, also run:

```sh
just check-worktrunk-parity-3way
cargo bench --bench scaling -- copy_plan
```

---

## PR1: macOS `clonefile` Spike

### Goal

Settle the implementation choice for the directory-level COW primitive on
macOS before planner work depends on it.

### Scope

- Add `tests/clonefile_spike.rs`, gated with `#[cfg(target_os = "macos")]`.
  The test builds a temp source directory containing only regular files and
  non-empty directories, calls `reflink_copy::reflink(src_dir, dst_dir)` with
  a non-existent destination, and asserts whether the recursive tree appears.
- If `reflink_copy::reflink` does not support directory inputs, add
  `src/sys/clonefile.rs` exposing:

  ```rust
  pub fn clonefile_dir(src: &Path, dst: &Path) -> io::Result<()>;
  ```

  The wrapper calls `libc::clonefile(src, dst, CLONE_NOFOLLOW)` on macOS only,
  has one small unsafe block, and documents the syscall contract.
- Document the chosen primitive in the module comment. The primitive is only
  used after PR3's exact-manifest preflight; it must not be called on arbitrary
  repository subtrees.

### Expected Files

- `tests/clonefile_spike.rs`
- `src/sys/clonefile.rs` if the crate does not expose the needed behavior
- `src/lib.rs` if a `sys` module is added
- `Cargo.toml` if a new `libc` dependency is required

### Acceptance

- `cargo test --test clonefile_spike` passes on macOS and is skipped or
  trivially passes elsewhere.
- We know which API `RealFs::copy_dir_exact` should call on macOS.

---

## PR2: `eligibility_groups` Module (No Callers)

### Goal

Land the source-tree grouping algorithm and tests in isolation, with no
observable behavior change.

### API

Add `src/eligibility_groups.rs`:

```rust
pub struct EligibleDir {
    pub rel_path: RepoRelPath,       // never root
    pub files: Vec<RepoRelPath>,     // eligible regular files under this dir
}

pub struct EligibilityGroups {
    pub full_dirs: Vec<EligibleDir>,       // maximal-only, sorted
    pub remaining_files: Vec<RepoRelPath>, // remaining eligible paths, sorted
}

pub fn compute(
    source_root: &Path,
    eligible: Vec<RepoRelPath>,
    gitlinks: &HashSet<String>,
) -> Result<EligibilityGroups>;
```

### Algorithm

Walk the physical source tree bottom-up without following symlinks. Do not
silently filter out blockers; record them.

For each entry:

- Root entry: never emit it as a candidate.
- Git boundary directory or linked-worktree boundary: mark its parent blocked
  and do not descend.
- Symlink or special file: mark its parent blocked. If its repo-relative path
  is in the eligible set, leave it in `remaining_files` for the normal planner
  to skip as `UnsupportedSourceType`.
- Regular file: if it is in the eligible set, record it as copyable for the
  parent chain; otherwise mark its parent blocked.
- Directory after children: it is structurally copyable only if it has at
  least one eligible regular-file descendant and no blocked child. Empty
  directories therefore block their parent.

After the walk, emit the maximal antichain of non-root copyable directories.
Remove every file covered by an emitted directory from `remaining_files`.

The root's `.git` directory must block root promotion. This is intentional:
the fast path should never clone the repository root.

### Git Boundary Helpers

- Add `GitBackend::gitlinks(source_root) -> Result<HashSet<String>>` by
  extracting the existing mode-160000 index logic from `git.rs`.
- Expose a shared helper for Git-boundary detection, or move the current
  `is_nested_git_boundary` logic to a small `walk` module. The grouping
  module needs the same definition as candidate enumeration, but with blocker
  semantics instead of skip semantics.

### Tests

Inline tests in `eligibility_groups.rs`:

- `all_files_under_top_level_dir_marks_that_dir` - `cfg/*` all eligible,
  asserts `cfg` is emitted and root is not.
- `root_is_never_promoted` - all normal files are eligible, but root contains
  `.git`; asserts no root `CopyDir` representation exists.
- `partial_directory_does_not_promote` - one regular file under a sibling is
  not eligible; eligible files remain in `remaining_files`.
- `nested_full_subdir_under_partial_root` - only the safe nested subdir is
  emitted.
- `maximal_only` - nested safe directories produce only the outermost safe
  non-root directory.
- `symlink_blocks_ancestor_and_remains_remaining_when_eligible` - selected
  symlink stays in `remaining_files`; ancestors are not promoted.
- `empty_dir_blocks_ancestor` - empty descendant prevents parent promotion.
- `gitlink_blocks_ancestor` - registered submodule directory prevents parent
  promotion and is not copied.
- `nested_git_repo_blocks_ancestor` - independent nested checkout prevents
  parent promotion and is not copied.
- `tracked_or_other_ineligible_file_blocks_ancestor` - any source file absent
  from `eligible` blocks promotion.

Backend parity coverage in `tests/backend_parity.rs`:

- `gitlinks_parity` - `GitGix` and `GitCli` return the same registered
  submodule paths.

### Acceptance

- Module compiles; tests pass.
- No production caller uses it yet.
- The algorithm is `O(N)` in source tree entries and performs metadata checks
  only, not file-content reads.

---

## PR3: `CopyDir` Plumbing (No Planner Emission)

### Goal

Add the model, filesystem primitive, executor dispatch, dry-run rendering, and
reporting support for directory batch operations. Production planning still
does not emit `CopyDir`.

### `src/model.rs`

Add:

```rust
pub enum PlannedEntry {
    Copy(CopyOp),
    CopyDir(CopyDirOp),
    NoOp(NoOpEntry),
    Skip(SkipEntry),
}

pub struct CopyDirOp {
    pub rel_path: RepoRelPath,       // directory path, never root
    pub src_abs: PathBuf,
    pub dst_abs: PathBuf,
    pub files: Vec<RepoRelPath>,     // repo-relative manifest covered by this op
}

impl CopyDirOp {
    pub fn file_count(&self) -> usize { self.files.len() }
}
```

Update `PlannedEntry::rel_path()` for sorting/reporting. Because `CopyDir`
never represents root, it can use the existing `RepoRelPath` type.

### `src/fs.rs`

Extend `FileSystem`:

```rust
/// Copy exactly the manifest of regular files under `src` into a new
/// destination directory `dst`. `expected_files` are relative to `src`.
/// `dst` must not already exist. Implementations must not copy symlinks,
/// special files, empty directories, or entries absent from the manifest.
fn copy_dir_exact(
    &self,
    src: &Path,
    dst: &Path,
    expected_files: &[PathBuf],
    strategy: CopyStrategy,
) -> io::Result<()>;
```

`RealFs::copy_dir_exact`:

1. Refuse with `AlreadyExists` if any entry already exists at `dst`,
   including a broken symlink (`symlink_metadata`, not follow-based
   `Path::exists`).
2. Require `dst.parent()` to exist; the executor creates it.
3. Build a unique temporary sibling directory/path in the destination parent.
4. Preflight the physical `src` tree against `expected_files`:
   - every expected file exists, is a regular file, and is not a symlink;
   - no extra regular files exist;
   - no symlinks or special files exist;
   - no empty directories exist;
   - every directory is structural for at least one expected file.
5. If any expected manifest file is missing, symlinked, or non-regular, fail
   without copying. This preserves the existing "do not copy source symlinks"
   rule even if the source changes after planning.
6. If the preflight is exact and the strategy allows COW on macOS, clone the
   source directory to a temporary sibling path and rename that temp path to
   `dst`.
7. If preflight fails only because extra entries, empty directories, symlinks,
   or special files exist outside the manifest, do not clone. Create a
   temporary sibling directory, copy only `expected_files` into it with
   `copy_file`, creating needed parents, then rename the temp directory to
   `dst`.
8. On failure, remove the temporary directory/path and leave `dst` absent.

The manifest fallback is a correctness guard for stale plans and platforms
without recursive COW. It preserves the same destination shape as the per-file
pipeline.

`MockFs` implementations should copy only the provided manifest and record
that `copy_dir_exact` was called.

### `src/executor.rs`

Add a `CopyDir` arm:

- dry run: increment `copied` by `op.file_count()` and push one aggregate
  result;
- real run: call `execute_copy_dir`; success/failure increments counts by
  `op.file_count()`.

`execute_copy_dir`:

```rust
fn execute_copy_dir(
    op: &CopyDirOp,
    fs: &dyn FileSystem,
    strategy: CopyStrategy,
) -> Result<(), String> {
    if fs.parent_has_symlink(&op.dst_abs) {
        return Err(format!("{}: destination parent contains a symlink", op.rel_path));
    }
    if let Some(parent) = op.dst_abs.parent() {
        fs.create_dir_all(parent)
            .map_err(|e| format!("{}: failed to create directory: {e}", op.rel_path))?;
    }
    let manifest = files_relative_to_dir(&op.rel_path, &op.files)?;
    fs.copy_dir_exact(&op.src_abs, &op.dst_abs, &manifest, strategy)
        .map_err(|e| format!("{}: failed to copy directory: {e}", op.rel_path))
}
```

This explicitly handles missing parents for nested promoted directories.

### Reporting

Keep one aggregate line per `CopyDir`, but keep counts file-based:

```text
copy-dir: cfg (3 files)
3 copied, 0 failed, 0 skipped, 0 up-to-date
```

The cleanest implementation is to pass both the plan and report to
`render_report`, so report rendering can match aggregate results back to
`PlannedEntry::CopyDir`. Alternatively, widen `CopyResult` with a small kind
tag and file count.

Dry-run rendering prints:

```text
copy-dir: cfg (3 files)
```

and counts `CopyDir` by `file_count()`.

### Tests

- `realfs_copy_dir_exact_clones_or_copies_tree` - safe manifest produces the
  expected destination tree.
- `realfs_copy_dir_exact_refuses_existing_dst`.
- `realfs_copy_dir_exact_does_not_copy_empty_dirs` - empty source descendant
  is not reproduced.
- `realfs_copy_dir_exact_rejects_expected_symlink`.
- `realfs_copy_dir_exact_cleans_temp_on_failure`.
- `execute_copydir_calls_fs_and_counts_files`.
- `execute_copydir_creates_missing_parent`.
- `execute_copydir_records_failure_counted_by_file_count`.
- dry-run/report tests for the aggregate `copy-dir` line.

### Acceptance

- Build and tests pass.
- Production planner has no `CopyDir` construction sites.
- Existing copy integration output remains unchanged.

---

## PR4: Wire Planner to Emit `CopyDir`

### Goal

Behavior change. Planner consumes `EligibilityGroups`, emits `CopyDir` for
source-exact directories whose destination state is safe, and falls back to
per-file planning everywhere else.

### `src/planner.rs`

Change the signature:

```rust
pub fn plan(
    ctx: &RepoContext,
    validation: ValidationReport,
    groups: EligibilityGroups,
    git: &dyn GitBackend,
    fs: &dyn FileSystem,
    overwrite: bool,
    dry_run: bool,
) -> Result<CopyPlan>;
```

Main flow:

1. Build `all_manifest_files` from `groups.remaining_files` plus every
   `EligibleDir.files`.
2. Call `git.tracked_paths(dest_root, &all_manifest_files)` once.
3. Start the per-file work list with `groups.remaining_files`.
4. For each `EligibleDir`:
   - if any covered file is tracked in `dest_tracked`, append the files to the
     per-file work list;
   - else if `fs.parent_has_symlink(dst_abs)` is true, append the files to the
     per-file work list so existing `UnsafePath` reporting/counts apply;
   - else if `fs.exists(dst_abs)` is true, append the files to the per-file
     work list;
   - otherwise emit `PlannedEntry::CopyDir(CopyDirOp { ... })`.
5. Run the existing per-file classification loop over the final work list,
   using the precomputed `dest_tracked` set.
6. Sort entries deterministically by `PlannedEntry::rel_path()`.

Do not aggregate unsafe or tracked conflicts into one directory-level skip in
this PR. Per-file fallback preserves existing skip reasons and counts.

### `src/subcommands/copy.rs`

After `git.check_ignore`:

```rust
let eligible: Vec<_> = ignore_results
    .into_iter()
    .filter(|r| r.match_info.is_some())
    .map(|r| r.path)
    .collect();

let gitlinks = git.gitlinks(&ctx.source_root)?;
let groups = crate::eligibility_groups::compute(
    &ctx.source_root,
    eligible,
    &gitlinks,
)?;

let plan = crate::planner::plan(
    &ctx,
    report,
    groups,
    git.as_ref(),
    &fs,
    args.overwrite,
    args.dry_run,
)?;
```

Verify `list` and `info` do not call `planner::plan`. They currently stop
earlier or call `classify_destination` directly for verbose explanations.

### Planner Tests

- `plan_emits_copydir_for_missing_untracked_dst`.
- `plan_falls_back_when_dst_dir_exists`.
- `plan_falls_back_when_dst_parent_has_symlink`.
- `plan_falls_back_when_any_covered_dest_file_is_tracked_even_if_missing`.
- `plan_mixed_full_and_partial`.
- `plan_copydir_skips_source_file_type_checks_for_dir_itself`.
- `plan_counts_copydir_by_manifest_size_in_dry_run`.

### Integration Tests

- `copy_uses_fast_path_for_safe_fresh_subtree` - source has safe `cfg/` plus
  a root `.env`; stderr contains one `copy-dir: cfg (N files)` and no per-file
  `copied:` lines for `cfg` descendants.
- `copy_falls_back_for_existing_dst_dir`.
- `copy_idempotency_with_fast_path`.
- `copy_fast_path_does_not_write_missing_tracked_dest_file` - destination
  index tracks one covered file that is deleted from disk; planner must not
  emit `CopyDir`, and that file remains a tracked conflict.
- `copy_fast_path_skips_subtree_with_symlink` - no `copy-dir` for the parent;
  selected symlink is reported as `UnsupportedSourceType`.
- `copy_fast_path_does_not_clone_gitlink_contents` - parent containing a
  registered submodule is not promoted; submodule contents are absent from
  destination. Include a separate safe sibling directory to prove the fast
  path still activates elsewhere in the same run.
- `copy_fast_path_does_not_clone_nested_repo_contents`.
- `copy_fast_path_does_not_create_empty_dirs`.
- `copy_fast_path_handles_nested_dir_with_missing_parent`.

### Suggested Fixture Shape

Use a safe fast-path subtree plus blockers elsewhere:

```rust
fn setup_with_safe_full_dir() -> (TempDir, TempDir) {
    let (main_dir, wt_dir) = setup_worktrees();
    write_file(main_dir.path(), ".gitignore", "cfg/\n.env\n");
    write_file(main_dir.path(), ".worktreeinclude", "cfg/\n.env\n");
    write_file(main_dir.path(), "cfg/a.conf", "a\n");
    write_file(main_dir.path(), "cfg/b.conf", "b\n");
    write_file(main_dir.path(), "cfg/nested/c.conf", "c\n");
    write_file(main_dir.path(), ".env", "X=1\n");
    git(main_dir.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(main_dir.path(), &["commit", "-m", "fixture"]);
    (main_dir, wt_dir)
}
```

`cfg/` is the only `CopyDir`; `.env` remains a per-file `Copy`.

### Benchmarks

Add a benchmark to `benches/scaling.rs` that measures grouping + planning +
execution against a mock filesystem:

- safe tree: all files under one safe `cfg/` directory, destination missing;
- blocker tree: same shape plus one empty dir, symlink, or ineligible sentinel
  to force per-file fallback.

Record both fast-path and fallback timings. Expected improvement should be
reported in the PR description, but correctness gates are more important than
hard-coding a platform-specific ratio in tests.

### Acceptance

- All Global Gate commands pass.
- New integration tests pass.
- `copy_idempotency_with_fast_path` proves the repeated-run behavior.
- Gitlink and nested-repo tests prove wholesale cloning cannot leak other
  repository contents.
- Missing tracked destination test proves deleted tracked files are not
  recreated by `CopyDir`.
- Empty-directory test proves no new empty directories appear.
- Benchmark numbers are recorded in the PR description.

---

## Risk & Rollback

Highest-risk failure modes:

1. Wholesale-copying content that the per-file pipeline would not copy:
   mitigated by blocker semantics for Git boundaries, symlinks, empty dirs,
   special files, and ineligible regular files, plus exact-manifest execution.
2. Recreating missing tracked destination files:
   mitigated by checking destination trackedness for every covered file before
   emitting `CopyDir`.
3. Partial destination trees after failure:
   mitigated by temp sibling directory plus atomic rename.

Rollback path: revert PR4. PRs 1-3 are dormant after revert and can remain
merged until removed or reused.

## Out of Scope

- Fast path for existing destination directories. That would require a
  recursive content-equality and trackedness proof for the destination tree.
- Directory-level aggregate skip reporting. This plan intentionally falls
  back to per-file skips to preserve existing reasons and counts.
- Copying empty directories. Current behavior is file-driven and does not
  reproduce them.
- Fast-pathing the repository root.
