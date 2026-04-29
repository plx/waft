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

## Compatibility profiles

`waft` ships with three coordinated compat profiles selectable via
`--compat-profile <name>`:

| Profile | When `.worktreeinclude` is missing | Matcher semantics | Symlinked rule files | Tool-state excludes |
|---------|-----------------------------------|-------------------|----------------------|---------------------|
| `claude` *(default)* | nothing selected | `claude-2026-04` (root rule file only) | follow | none |
| `git` | nothing selected | `git` (per-directory `.gitignore` rules) | ignore | none |
| `wt` | every git-ignored untracked file selected | `wt-0.39` (all-ignored minus literal-name negations) | follow | `tooling-v1` (`.conductor/`, `.claude/`, etc.) |

The OOTB experience matches Claude Code. Pick `--compat-profile git` for Git's
per-directory exclude semantics, or `--compat-profile wt` for worktrunk parity.

### Layered configuration

Profile and individual knobs are resolved from a layered config in this order
(later layers win for scalars; `extra-exclude` arrays append, with
`replace-extra-excludes` to truncate):

1. Built-in defaults (claude preset)
2. User config: `~/.config/waft/config.toml`
3. Project configs: each `.waft.toml` from repo root down to cwd
4. Environment variables (`WAFT_*`)
5. CLI flags

Explicit knob settings (in any layer) always beat preset values from a
higher-precedence layer.

### Per-knob CLI flags

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

Example `.waft.toml`:

```toml
version = 1

[compat]
profile = "git"

[exclude]
extra = ["*.bak"]
```

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
