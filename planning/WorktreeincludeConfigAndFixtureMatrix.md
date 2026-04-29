# Worktreeinclude Config Schema, CLI Flags, and Fixture Matrix

## Goals

- Default behavior should match Claude Code out-of-the-box behavior.
- Behavior must be explicitly switchable to Git-like semantics and worktrunk-like semantics.
- Divergences should be controlled by narrow, composable flags so targeted tests are easy.

## Configuration File Schema (Draft)

Primary config locations and merge order:

1. Built-in defaults
2. User config: `~/.config/waft/config.toml`
3. Project configs discovered by walking upward from repo root to current working directory: each `.waft.toml` encountered
4. Environment variables (`WAFT_*`)
5. CLI flags

Last writer wins for scalar keys. Array keys append unless a `replace-*` flag is used.

```toml
version = 1

[compat]
# Preset that initializes behavior knobs. Default: "claude".
profile = "claude" # claude | git | wt

[worktreeinclude]
# Behavior when no .worktreeinclude file exists anywhere relevant.
# - "blank": select nothing
# - "all-ignored": treat all git-ignored paths as selected
when_missing = "blank" # blank | all-ignored

# Matcher semantics profile.
# - "claude-2026-04": match current observed Claude behavior
# - "git": Git per-directory exclude semantics
# - "wt-0.39": match current observed worktrunk behavior
semantics = "claude-2026-04" # claude-2026-04 | git | wt-0.39

# How symlinked .worktreeinclude files are handled.
# - "follow": follow symlink target as rule file
# - "ignore": ignore symlinked rule files (as if absent at that location)
# - "error": fail validation/copy
symlink_policy = "follow" # follow | ignore | error

[exclude]
# Built-in exclusion set for tool directories.
# - "none": no built-in excludes
# - "tooling-v1": apply built-in exclusions listed below
builtin_set = "none" # none | tooling-v1

# Additional repo-relative glob excludes applied after worktreeinclude selection.
# Repeatable and mergeable.
extra = []

# If true, `extra` replaces inherited `extra` values from lower-precedence layers.
replace_extra = false
```

Built-in set `tooling-v1` (draft list):

- `.conductor/`
- `.claude/`
- `.worktrees/`
- `.git/` (safety guard)
- `.jj/`
- `.hg/`
- `.svn/`
- `.bzr/`
- `.pijul/`
- `.sl/`
- `.entire/`
- `.pi/`

## CLI Flags (Draft)

```text
--compat-profile <claude|git|wt>

--when-missing-worktreeinclude <blank|all-ignored>
--worktreeinclude-semantics <claude-2026-04|git|wt-0.39>
--worktreeinclude-symlink-policy <follow|ignore|error>

--builtin-exclude-set <none|tooling-v1>
--extra-exclude <GLOB>                # repeatable
--replace-extra-excludes              # boolean

--config <PATH>                       # optional explicit config file
```

Environment variable mapping (draft):

- `WAFT_COMPAT_PROFILE`
- `WAFT_WHEN_MISSING_WORKTREEINCLUDE`
- `WAFT_WORKTREEINCLUDE_SEMANTICS`
- `WAFT_WORKTREEINCLUDE_SYMLINK_POLICY`
- `WAFT_BUILTIN_EXCLUDE_SET`
- `WAFT_EXTRA_EXCLUDE` (comma-separated)
- `WAFT_REPLACE_EXTRA_EXCLUDES`
- `WAFT_CONFIG_PATH`

## Preset Expansion (Draft)

`claude` preset:

- `when_missing = "blank"`
- `semantics = "claude-2026-04"`
- `symlink_policy = "follow"`
- `builtin_set = "none"`
- `extra = []`

`git` preset:

- `when_missing = "blank"`
- `semantics = "git"`
- `symlink_policy = "ignore"`
- `builtin_set = "none"`
- `extra = []`

`wt` preset:

- `when_missing = "all-ignored"`
- `semantics = "wt-0.39"`
- `symlink_policy = "follow"`
- `builtin_set = "tooling-v1"`
- `extra = []`

## Fixture Matrix

All fixtures assume:

- source repo contains committed rule files (`.gitignore`, `.worktreeinclude`, nested `.worktreeinclude` as needed)
- source has untracked files listed below
- destination is a fresh linked worktree
- copy mode is overwrite-enabled for deterministic comparisons
- expected output is the set of copied source-relative paths

### Scenario F1: `root-simple`

Fixture setup:

- `.gitignore`: `.env`
- `.worktreeinclude`: `.env`
- source files: `.env`

Expected outcomes:

- `claude`: success, copied `{.env}`
- `git`: success, copied `{.env}`
- `wt`: success, copied `{.env}`

### Scenario F2: `no-worktreeinclude`

Fixture setup:

- `.gitignore`: `.env`, `cache/`
- no `.worktreeinclude`
- source files: `.env`, `cache/build.bin`

Expected outcomes:

- `claude`: success, copied `{}`
- `git`: success, copied `{}`
- `wt`: success, copied `{.env, cache/build.bin}`

Flag-level verification cases:

- baseline `claude` + `--when-missing-worktreeinclude all-ignored` -> `{.env, cache/build.bin}`
- baseline `wt` + `--when-missing-worktreeinclude blank` -> `{}`

### Scenario F3: `nested-worktreeinclude-override`

Fixture setup:

- `.gitignore`: `*.env`
- root `.worktreeinclude`: `*.env`
- `config/.worktreeinclude`: `!*.env`
- source files: `root.env`, `config/sub.env`

Expected outcomes:

- `claude`: success, copied `{root.env, config/sub.env}`
- `git`: success, copied `{root.env}`
- `wt`: success, copied `{root.env, config/sub.env}`

Flag-level verification cases:

- `claude` + `--worktreeinclude-semantics git` -> `{root.env}`
- `git` + `--worktreeinclude-semantics claude-2026-04` -> `{root.env, config/sub.env}`

### Scenario F4: `nested-anchored-pattern`

Fixture setup:

- `.gitignore`: `foo`, `config/foo`
- `config/.worktreeinclude`: `/foo`
- source files: `foo`, `config/foo`

Expected outcomes:

- `claude`: success, copied `{}`
- `git`: success, copied `{config/foo}`
- `wt`: success, copied `{foo, config/foo}`

Flag-level verification cases:

- `claude` + `--worktreeinclude-semantics git` -> `{config/foo}`
- `wt` + `--worktreeinclude-semantics git` -> `{config/foo}`

### Scenario F5: `cross-file-negation-caveat`

Fixture setup:

- `.gitignore`: `secrets/`
- root `.worktreeinclude`: `secrets/`
- `secrets/.worktreeinclude`: `!private.key`
- source files: `secrets/private.key`

Expected outcomes:

- `claude`: success, copied `{secrets/private.key}`
- `git`: success, copied `{secrets/private.key}`
- `wt`: success, copied `{}`

Flag-level verification cases:

- `wt` + `--worktreeinclude-semantics git` -> `{secrets/private.key}`

### Scenario F6: `nested-worktree-in-repo`

Fixture setup:

- `.gitignore`: `.worktrees/`
- `.worktreeinclude`: `.worktrees/**/*.env`
- nested linked worktree exists at `.worktrees/nested` containing untracked `.env`

Expected outcomes:

- `claude`: success, copied `{}`
- `git`: success, copied `{}`
- `wt`: success, copied `{}`

### Scenario F7: `tool-state-directory`

Fixture setup:

- `.gitignore`: `.conductor/`
- `.worktreeinclude`: `.conductor/**/*.key`
- source files: `.conductor/state/dev.key`

Expected outcomes:

- `claude`: success, copied `{.conductor/state/dev.key}`
- `git`: success, copied `{.conductor/state/dev.key}`
- `wt`: success, copied `{}`

Flag-level verification cases:

- `claude` + `--builtin-exclude-set tooling-v1` -> `{}`
- `wt` + `--builtin-exclude-set none` -> `{.conductor/state/dev.key}`

### Scenario F8: `symlinked-worktreeinclude`

Fixture setup:

- `.gitignore`: `.env`
- symlink `.worktreeinclude -> real.wti`
- `real.wti`: `.env`
- source files: `.env`

Expected outcomes:

- `claude`: success, copied `{.env}`
- `git`: success, copied `{}` (symlinked include file ignored)
- `wt`: success, copied `{.env}`

Flag-level verification cases:

- any profile + `--worktreeinclude-symlink-policy error` -> failure
- any profile + `--worktreeinclude-symlink-policy ignore` -> success, copied `{}`
- any profile + `--worktreeinclude-symlink-policy follow` -> success, copied `{.env}`

## Suggested Test Layout

- `tests/modes_profile_integration.rs`
  - one table-driven test per fixture F1-F8
  - run each fixture under `--compat-profile claude|git|wt`
  - assert exit status + exact copied set

- `tests/modes_flag_override_integration.rs`
  - focused override tests for each flag-level case listed above
  - ensures flags override preset and config values correctly

- `tests/config_discovery_integration.rs`
  - verifies merge order across user config, upward-walk `.waft.toml`, env, CLI

- `tests/config_parse_unit.rs`
  - validates enums and invalid value errors for every schema field

## Notes

- `claude-2026-04` and `wt-0.39` semantics labels are intentionally versioned snapshots to avoid silently changing behavior when external tools change.
- A future update can add `claude-latest` and `wt-latest` aliases once periodic parity refresh automation exists.
