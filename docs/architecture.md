# waft Architecture

## Foundational Decisions

These foundational decisions describe what is invariant across waft's
implementation. Specific knob values (matcher semantics, when-missing
behavior, exclusion sets, symlink policy) are parameterized and live
behind the layered configuration described below; the decisions here
describe the architecture those knobs plug into.

### 1. A Git backend is authoritative for ignore membership and trackedness

waft does **not** reimplement Git's ignore logic for determining whether a
file is actually ignored or tracked. All such decisions go through the
`GitBackend` trait and are answered by one of two interchangeable
implementations:

- `GitGix` (default): an in-process implementation built on the `gix` crate.
- `GitCli`: shells out to `git check-ignore`, `git ls-files`, etc. Selected
  by setting `WAFT_GIT_BACKEND=cli`.

Backend parity tests (`tests/backend_parity.rs`) pin both implementations
to the same observable behavior. The `ignore` crate is used only for
parsing, validation, and explanation — never as the final oracle for
ignore/tracked status.

### 2. `.worktreeinclude` is matched by an independent, pluggable engine

`.worktreeinclude` files use `.gitignore` syntax, but the algorithm that
decides which paths the rules *select* is not the Git backend's job; it
lives in `worktreeinclude_engine`. Three engines ship with v1, selected
by the active compat profile:

- `claude-2026-04` (default `claude` profile) — only the repository's
  root-level `.worktreeinclude` file contributes patterns.
- `git` (`git` profile) — Git-style per-directory exclude semantics, where
  patterns are relative to the directory containing the file and deeper
  files take precedence over shallower ones. This is what waft's pre-modes
  implementation always did.
- `wt-0.39` (`wt` profile) — worktrunk's subtractive algorithm: start from
  every git-ignored untracked file, then remove paths matched by literal
  `!<filename>` negations across all rule files.

The engine is consulted *after* the Git backend has supplied either the
worktreeinclude candidate set or the all-ignored fallback set. See
"Command Pipeline" below.

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
and matching. waft uses it for `.worktreeinclude` explanation, for validation
of ignore files, and as the matcher behind the `policy_filter` exclusion
patterns. The crate does not consult the Git index and cannot determine
whether a file is tracked, so it is never the sole authority for "Git-ignored"
status — that always comes from the Git backend.

## Repo Layout

```
src/
  main.rs                     # Entry point: parse args, dispatch, exit
  lib.rs                      # Library root with public module declarations
  cli.rs                      # clap derive root Cli struct + policy resolution dispatch
  config.rs                   # Layered config schema, profile resolution, ResolvedPolicy
  subcommands/                # Per-subcommand argument types and handlers
    mod.rs                    # select_candidates dispatcher; re-exports per-subcommand
    copy.rs                   # `copy` subcommand (plan + execute)
    list.rs                   # `list` subcommand (enumerate eligible files)
    info.rs                   # `info` subcommand (per-path status)
    validate.rs               # `validate` subcommand (ignore-file linting)
  context.rs                  # Repo/worktree context resolution
  git.rs                      # GitBackend trait, GitGix (default) and GitCli backends
  validate.rs                 # Ignore/worktreeinclude file validation
  worktreeinclude.rs          # Per-directory `.gitignore`-style matcher (used by engines)
  worktreeinclude_engine.rs   # Pluggable matcher engines (Git, Claude2026-04, Wt-0.39)
  policy_filter.rs            # Post-selection exclusion filter (built-in + extras)
  planner.rs                  # Read-only copy planning
  executor.rs                 # CopyPlan execution with atomic writes
  model.rs                    # Domain types (CopyPlan, InfoReport, etc.)
  path.rs                     # RepoRelPath and path normalization
  fs.rs                       # FileSystem trait for testability
  error.rs                    # Error types via thiserror
tests/
  cli.rs                              # CLI integration tests (help, dispatch, flag parsing)
  git_integration.rs                  # Tests against real Git repos in temp dirs
  copy_integration.rs                 # Copy execution integration tests
  info_integration.rs                 # Info command integration tests
  config_parse_unit.rs                # TOML parsing, enum acceptance, schema validation
  config_resolution_unit.rs           # Layer composition and preset expansion
  config_discovery_integration.rs     # Project-config upward walk and precedence
  modes_profile_integration.rs        # Each fixture under each compat profile
  modes_flag_override_integration.rs  # Per-knob CLI flag overrides
  backend_parity.rs                   # GitGix vs GitCli observable equivalence
  worktrunk_parity.rs                 # Cross-tool parity scenarios
  property.rs                         # Differential/property tests against `git` profile oracle
docs/
  architecture.md             # This file
planning/
  WorktreeincludeConfigAndFixtureMatrix.md  # Source of truth for profile/knob/fixture matrix
  WorktreeincludeImplementationPRSteps.md   # PR-by-PR plan for the modes feature
  WorktrunkParityReport.md                  # Observed `wt-0.39` behavior captured for tests
```

## Command Pipeline

All commands follow this pipeline:

1. **CLI parsing** (`cli.rs`) — clap derive parses args, dispatches to a
   subcommand handler in `subcommands/`.
2. **Policy resolution** (`config.rs`) — merge built-in defaults, user
   config (`~/.config/waft/config.toml`), each `.waft.toml` from repo
   root down to cwd, environment variables (`WAFT_*`), and CLI flags
   into a single `ResolvedPolicy`. Explicit knob values always beat
   preset values from a higher-precedence layer.
3. **Context resolution** (`context.rs`) — resolve source/dest worktrees
   via the Git backend (`show_toplevel`, `list_worktrees`).
4. **Validation** (`validate.rs`) — parse all ignore files with
   `GitignoreBuilder`. The `symlink_policy` knob decides what happens
   when a `.worktreeinclude` rule file is itself a symlink.
5. **Candidate selection** (`subcommands::select_candidates`) — dispatches
   on whether any `.worktreeinclude` exists in the repo. If yes, the
   matcher engine selected by `policy.semantics` produces the candidate
   set (root-only for `claude-2026-04`, per-directory for `git`,
   subtractive for `wt-0.39`). If no, `policy.when_missing` decides:
   `blank` selects nothing; `all-ignored` selects every git-ignored
   untracked file from the Git backend.
6. **Policy filter** (`policy_filter.rs`) — drop paths matched by
   `policy.builtin_exclude_set` (e.g. `tooling-v1`) or
   `policy.extra_excludes`.
7. **Ignore filtering** (`git.rs`) — `check_ignore` retains only paths
   the Git backend confirms are git-ignored.
8. **Planning** (`planner.rs`) — classify destinations, produce `CopyPlan`.
9. **Execution** (`executor.rs`) — atomic copy via temp file + rename.

Commands stop at different stages: `validate` at step 4, `list` at step 7,
`info` at step 7 plus per-path explanation, `copy --dry-run` at step 8,
and `copy` at step 9.

## Testing Strategy

Tests are organized in several layers:

1. **Unit tests** — pure logic in `path.rs`, `git.rs` (parser tests),
   `planner.rs` (mock filesystem), `worktreeinclude.rs`, `context.rs`
   (mock git backend), `config.rs` (TOML parsing, layer merge), and
   `policy_filter.rs` (pattern matching).
2. **Config tests** (`tests/config_*.rs`) — schema/enum acceptance,
   layer composition with explicit-beats-preset rules, and project-config
   discovery via upward walk.
3. **Integration tests** — real Git repos in temp directories for all
   commands.
4. **Mode coverage tests** (`tests/modes_profile_integration.rs`,
   `tests/modes_flag_override_integration.rs`) — each fixture in the
   matrix is exercised under every applicable profile and per-knob flag
   override.
5. **Backend parity tests** (`tests/backend_parity.rs`) — pin `GitGix`
   and `GitCli` to identical observable behavior on a representative set
   of repo shapes (linked worktrees, nested checkouts, registered
   submodules, missing rule files, empty rule files).
6. **Differential/property tests** (`tests/property.rs`) — compare waft
   output (run with `--compat-profile git`) against the
   `git ls-files --exclude-per-directory=.worktreeinclude` +
   `git check-ignore` oracle, with both deterministic scenarios and
   proptest-generated random repos.

The differential tests historically anchored waft's correctness against
Git; they remain the per-directory engine's primary correctness gate.
The fixture matrix in `planning/WorktreeincludeConfigAndFixtureMatrix.md`
is the source of truth for the cross-profile expectations exercised by
the mode-coverage tests.

## Per-Directory Semantics (the `git` Engine)

The `git` semantics engine reproduces Git's per-directory exclude file
behavior:

1. **Patterns are relative to the directory containing the file.** A
   pattern `*.log` in `subdir/.gitignore` matches `subdir/foo.log` but
   not `other/foo.log`.

2. **Deeper files take precedence over shallower ones.** If the root
   `.gitignore` ignores `*.log` and `subdir/.gitignore` contains `!*.log`,
   then `subdir/foo.log` is un-ignored (re-included).

Because this engine intentionally uses Git-style exclude semantics, Git's
negation caveat also applies: a file inside an excluded parent directory
cannot be re-included by a deeper negation pattern. Users of this engine
should prefer `dir/*` plus `!dir/keep` over `dir/` plus `!dir/keep`.

The `claude-2026-04` and `wt-0.39` engines deliberately diverge from
these rules; see `planning/WorktreeincludeConfigAndFixtureMatrix.md` for
the per-engine fixture expectations.
