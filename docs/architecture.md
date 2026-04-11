# wiff Architecture

## Foundational Decisions (v1)

These four decisions are fixed for the entire v1 implementation and must not be
revisited during development.

### 1. Git CLI is authoritative for actual Git-ignored membership and trackedness

wiff does **not** reimplement Git's ignore logic for determining whether a file
is actually ignored or tracked. Instead, it shells out to `git check-ignore` and
`git ls-files` and treats their answers as ground truth. The `ignore` crate is
used only for parsing, validation, and explanation — never as the final oracle
for ignore/tracked status.

### 2. `.worktreeinclude` is an independent matcher with Git-style per-directory semantics

`.worktreeinclude` files use `.gitignore` syntax. Patterns in a per-directory
`.worktreeinclude` are relative to the directory containing that file. Deeper
`.worktreeinclude` files take precedence over shallower ones, mirroring Git's
behavior for per-directory exclude files. A normal pattern selects a path for
copy; a `!`-negated pattern deselects it.

Because `.worktreeinclude` intentionally uses Git-style exclude semantics, Git's
negation caveat also applies: a file inside an excluded parent directory cannot
be re-included by a deeper negation pattern. Users should prefer `dir/*` plus
`!dir/keep` over `dir/` plus `!dir/keep`.

### 3. The CLI uses plan/execute internally; discovery never mutates the filesystem

All commands follow a strict two-phase design:

1. **Plan**: discover eligible files, classify destination state, build a
   `CopyPlan`. This phase is read-only.
2. **Execute**: apply `CopyOp` entries from the plan. Only `copy` (without
   `--dry-run`) reaches this phase.

`list`, `info`, and `validate` never enter the execute phase. `copy --dry-run`
renders the plan without executing it.

### 4. `ignore` is used as parser/compiler reference, not as the only source of truth

The `ignore` crate's `gitignore` module provides pattern parsing, compilation,
and matching. wiff uses it for `.worktreeinclude` explanation and for validation
of ignore files. However, the crate does not consult the Git index and cannot
determine whether a file is tracked, so it cannot be the sole authority for
"Git-ignored" status.

## Repo Layout

```
src/
  main.rs              # Entry point: parse args, dispatch, exit
  lib.rs               # Library root with public module declarations
  cli.rs               # clap derive CLI structs and dispatch
  context.rs           # Repo/worktree context resolution
  git.rs               # GitBackend trait and GitCli implementation
  validate.rs          # Ignore/worktreeinclude file validation
  worktreeinclude.rs   # .worktreeinclude explanation engine
  planner.rs           # Read-only copy planning
  executor.rs          # CopyPlan execution with atomic writes
  model.rs             # Domain types (CopyPlan, InfoReport, etc.)
  path.rs              # RepoRelPath and path normalization
  fs.rs                # FileSystem trait for testability
  error.rs             # Error types via thiserror
tests/
  cli.rs               # CLI integration tests (help, dispatch)
  git_integration.rs   # Tests with real Git repos in temp dirs
  copy_integration.rs  # Copy execution integration tests
  info_integration.rs  # Info command integration tests
  property.rs          # Differential/property tests against Git
docs/
  architecture.md      # This file
```

## Command Pipeline

All commands follow this pipeline:

1. **CLI parsing** (`cli.rs`) — clap derive parses args, dispatches to handler
2. **Context resolution** (`context.rs`) — resolve source/dest worktrees via
   `git rev-parse` and `git worktree list`
3. **Validation** (`validate.rs`) — parse all ignore files with `GitignoreBuilder`
4. **Candidate enumeration** (`git.rs`) — `git ls-files --exclude-per-directory`
5. **Ignore filtering** (`git.rs`) — `git check-ignore --stdin -z -v -n`
6. **Planning** (`planner.rs`) — classify destinations, produce `CopyPlan`
7. **Execution** (`executor.rs`) — atomic copy via temp file + rename

Commands stop at different stages: `validate` at step 3, `list` at step 5,
`info` at step 5 plus explanation, `copy --dry-run` at step 6, and `copy` at
step 7.

## Testing Strategy

Tests are organized in three layers:

1. **Unit tests** — pure logic in `path.rs`, `git.rs` (parser tests),
   `planner.rs` (mock filesystem), `worktreeinclude.rs`, `context.rs`
   (mock git backend)
2. **Integration tests** — real Git repos in temp directories for all commands
3. **Differential/property tests** — compare wiff output against Git oracle
   (`git ls-files` + `git check-ignore`) with both deterministic scenarios
   and proptest-generated random repos

The differential tests are the most important layer because they verify that
wiff's behavior matches Git's behavior exactly, catching edge cases in
precedence, negation, anchoring, and recursive patterns.

## Why Nested `.worktreeinclude` Must Mirror Git's Per-Directory Exclude Behavior

Git's per-directory exclude files (`.gitignore`, and files loaded via
`--exclude-per-directory`) follow two key rules:

1. **Patterns are relative to the directory containing the file.** A pattern
   `*.log` in `subdir/.gitignore` matches `subdir/foo.log` but not
   `other/foo.log`.

2. **Deeper files take precedence over shallower ones.** If the root
   `.gitignore` ignores `*.log` and `subdir/.gitignore` contains `!*.log`, then
   `subdir/foo.log` is un-ignored (re-included).

`.worktreeinclude` must follow these same rules because:

- Users expect the same syntax to behave the same way.
- Git's `--exclude-per-directory=.worktreeinclude` flag already implements these
  semantics, and wiff uses that flag for authoritative candidate enumeration.
- The explanation engine must produce results consistent with what Git computes,
  which requires matching the same per-directory evaluation model.
