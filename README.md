# waft

Copy `.worktreeinclude`-selected ignored files between Git worktrees.

## What it does

When you use `git worktree` to work on multiple branches simultaneously,
local configuration files (`.env`, API keys, build caches) don't carry
over to linked worktrees because they're in `.gitignore`.

`waft` solves this: create a `.worktreeinclude` file listing which ignored
files you want copied, and `waft` handles the rest.

## Quick start

```sh
# In a linked worktree — copies from main worktree automatically
waft

# Explicit source and destination
waft copy --source /path/to/main --dest /path/to/linked

# See what would be copied
waft copy --dry-run

# List eligible files
waft list

# Inspect a specific file
waft info .env

# Validate ignore files
waft validate
```

## `.worktreeinclude` format

`.worktreeinclude` uses the same syntax as `.gitignore`:

```gitignore
# Include environment files
.env
*.env.local

# Include all secret keys recursively
**/*.key

# But not test keys
!test.key
```

Nested `.worktreeinclude` files work like nested `.gitignore` files:
patterns are relative to the directory containing the file, and deeper
files take precedence over shallower ones.

## Eligibility rule

A file is eligible for copying when **all** of these are true:

1. It exists in the source worktree
2. It is a regular file (not a symlink, directory, etc.)
3. It matches a `.worktreeinclude` pattern
4. It is Git-ignored (not tracked)

## Commands

| Command | Description |
|---------|-------------|
| `waft` / `waft copy` | Copy eligible files (default command) |
| `waft list` | List eligible files without copying |
| `waft info <PATH>...` | Show detailed status for specific files |
| `waft validate` | Check ignore files for syntax errors |

## Global options

| Option | Description |
|--------|-------------|
| `--source <PATH>` | Source (main) worktree path |
| `--dest <PATH>` | Destination (linked) worktree path |
| `-C <PATH>` | Operate as if started in PATH |
| `-q, --quiet` | Suppress non-error output |
| `-v, --verbose` | Increase output verbosity |

## Copy options

| Option | Description |
|--------|-------------|
| `-n, --dry-run` | Show what would be done without copying |
| `--overwrite` | Allow overwriting existing untracked files |

## Compatibility & policy options

These flags are global and route through a layered config (built-in defaults
< `~/.config/waft/config.toml` < `.waft.toml` walked from repo root to cwd <
`WAFT_*` env vars < CLI flags). The plumbing is in place; behavior wiring lands
in subsequent releases.

| Option | Description |
|--------|-------------|
| `--compat-profile <claude\|git\|wt>` | Coordinated preset selection |
| `--when-missing-worktreeinclude <blank\|all-ignored>` | Behavior when no `.worktreeinclude` exists |
| `--worktreeinclude-semantics <claude-2026-04\|git\|wt-0.39>` | Matcher semantics version |
| `--worktreeinclude-symlink-policy <follow\|ignore\|error>` | How to handle symlinked rule files |
| `--builtin-exclude-set <none\|tooling-v1>` | Curated tool-state exclusion set |
| `--extra-exclude <GLOB>` | Repeatable additional excludes |
| `--replace-extra-excludes` | Drop inherited `extra-exclude` values |
| `--config <PATH>` | Use this file instead of the default user config |

## Safety guarantees

- **Tracked files are never overwritten** in the destination worktree
- **Symlink traversal is blocked** — waft refuses to follow symlinks in
  source files or write through symlinked destination parents
- **Atomic writes** — files are written to a temp file first, then renamed
- **Dry-run is mutation-free** — `--dry-run` reads only, writes nothing

## Building

```sh
just build-release
```

## Testing

```sh
just check-test
```

## License

MIT
