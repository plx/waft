# wiff

Copy `.worktreeinclude`-selected ignored files between Git worktrees.

## What it does

When you use `git worktree` to work on multiple branches simultaneously,
local configuration files (`.env`, API keys, build caches) don't carry
over to linked worktrees because they're in `.gitignore`.

`wiff` solves this: create a `.worktreeinclude` file listing which ignored
files you want copied, and `wiff` handles the rest.

## Quick start

```sh
# In a linked worktree — copies from main worktree automatically
wiff

# Explicit source and destination
wiff copy --source /path/to/main --dest /path/to/linked

# See what would be copied
wiff copy --dry-run

# List eligible files
wiff list

# Inspect a specific file
wiff info .env

# Validate ignore files
wiff validate
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
| `wiff` / `wiff copy` | Copy eligible files (default command) |
| `wiff list` | List eligible files without copying |
| `wiff info <PATH>...` | Show detailed status for specific files |
| `wiff validate` | Check ignore files for syntax errors |

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

## Safety guarantees

- **Tracked files are never overwritten** in the destination worktree
- **Symlink traversal is blocked** — wiff refuses to follow symlinks in
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
