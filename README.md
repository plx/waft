# waft

Copy `.worktreeinclude`-selected ignored files between Git worktrees.

## What it does

When you use `git worktree` to work on multiple branches simultaneously,
local configuration files (`.env`, API keys, build caches) don't carry
over to linked worktrees because they're in `.gitignore`.

`waft` solves this: create a `.worktreeinclude` file listing which ignored
files you want copied, and `waft` handles the rest.

## Installation

`waft` is not currently published to crates.io. Install a reviewed revision
directly from the repository:

```sh
cargo install --git https://github.com/plx/waft \
  --rev REVIEWED_COMMIT_SHA --locked waft
```

Replace `REVIEWED_COMMIT_SHA` with the full commit you reviewed. Omitting
`--rev` installs the current tip of the default branch and is not recommended
for managed environments.

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

By default (`claude` profile), only the repository's root-level
`.worktreeinclude` is consulted. Pick `--compat-profile git` if you need
nested `.worktreeinclude` files to compose like nested `.gitignore`
files (patterns relative to their directory, deeper files winning over
shallower ones), or `--compat-profile wt` for worktrunk parity. See
"Compatibility profiles" below for the full matrix.

## Eligibility rule

A file is eligible for copying when **all** of these are true:

1. It exists in the source worktree
2. It is a regular file (not a symlink, directory, etc.)
3. It is selected by the active compat profile (matches a
   `.worktreeinclude` pattern, or — under `wt` / `--when-missing-worktreeinclude all-ignored` — is git-ignored even without a rule file)
4. It is Git-ignored (not tracked)
5. It is not dropped by the active exclusion set
   (`--builtin-exclude-set`, `--extra-exclude`)

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
| `--isolated` | Ignore user config and `WAFT_*` policy environment variables |

## Copy options

| Option | Description |
|--------|-------------|
| `-n, --dry-run` | Show what would be done without copying |
| `--overwrite` | Compatibility flag; fails closed on existing destination conflicts |

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
3. Project configs: each `.waft.toml` in the resolved source worktree, from
   its repo root down to the invocation directory's equivalent source path
4. Environment variables (`WAFT_*`)
5. CLI flags

Project configs must be regular files. Symlinked configs fail closed, and
discovery does not cross nested-repository or registered-submodule boundaries.

Selecting a profile resets all profile-owned knobs at that layer. Explicit
knobs in the same layer, or in a later layer, can override the preset; knobs
from lower layers cannot silently alter a higher-precedence profile.

For managed or hermetic use, `--isolated` removes layers 2 and 4. Built-in
defaults, source-repository project configs, and CLI overrides still apply in
that order. `--isolated` conflicts with `--config`; it also ignores
`WAFT_CONFIG_PATH`. Operational environment such as `WAFT_GIT_BACKEND` is not
part of policy resolution and remains available.

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
| `--isolated` | Use only defaults, source project config, and CLI policy flags |

Example `.waft.toml`:

```toml
version = 1

[compat]
profile = "git"

[exclude]
extra = ["*.bak"]
```

## Safety guarantees

- **Tracked-file protection** — destination trackedness is checked while
  planning and again under Git's cooperative index lock immediately before
  publication
- **No replacement** — every destination is published with no-clobber
  semantics; existing pathnames are preserved, and `--overwrite` fails closed
  when it encounters an untracked conflict
- **Descriptor-anchored traversal on Unix** — source and destination
  components are opened relative to canonical worktree directory handles with
  `O_NOFOLLOW`; source state is matched to its planning snapshot, and a
  destination parent is revalidated before publication
- **Durable atomic visibility** — file contents are synced to a temp file
  before publication, then the parent directory is synced on Unix
- **Dry-run is mutation-free** — `--dry-run` reads only, writes nothing

Normal Git writers honor the index lock and cannot change trackedness across
the final check and publication. A process that edits the index directly while
ignoring Git's lock protocol remains outside that guarantee. Unix directory
handles prevent a symlink or name swap from redirecting publication to a
different directory object. An already-open authorized directory can still be
relocated by another process after final revalidation; avoid concurrent
directory relocation when the destination pathname itself must remain stable.
For immediacy without holding Git's index lock during content preparation,
waft reacquires the lock and rechecks the index for each published file. That
cost is proportional to selected files times index size; keep automatic
manifests narrow and benchmark large cache manifests in monorepos.

## Website development

The production Astro/Starlight site lives in [`site/`](site/). Its visual,
component, and content-voice source of truth is the
[`site/design-system/`](site/design-system/) directory. Read
[`AGENTS.md`](AGENTS.md),
[`site/design-system/SKILL.md`](site/design-system/SKILL.md), and
[`site/design-system/README.md`](site/design-system/README.md) before changing
the site.

Run the complete site validation from `site/`:

```sh
npm run validate
```

## Building

```sh
just build-release
```

## Optional post-checkout hook

Review the checkout, then install the automatic worktree hook with:

```sh
just install-hooks
```

The installer builds `waft`, then copies the reviewed binary and hook into
the repository's common Git directory. It configures an absolute
`core.hooksPath`, proxies the standard Git hooks to the previously effective
trusted hook directory, and refuses to chain hooks sourced from a checked-out
worktree or through a symlink. It also refuses worktree-scoped hook overrides
in any extant linked worktree, since those would bypass the shared managed
path. Subsequent branch changes cannot replace the installed hook or binary.
The automatic hook always invokes that pinned sibling binary and runs
`waft --isolated`, so ambient `WAFT`, user config, and `WAFT_*` policy
variables cannot change automatic execution or copy selection; trusted
project config from the source worktree still applies. Re-run the command
after upgrading `waft`.

Do **not** configure `core.hooksPath` to this repository's tracked `hooks/`
directory. A branch can change tracked hook content before Git executes it.

To restore the prior hook configuration:

```sh
just uninstall-hooks
```

## Testing

```sh
just check-test
```

`waft` intentionally creates additional copies of ignored files. Prefer
short-lived credentials or secret injection over copying high-value,
long-lived secrets, and review `waft copy --dry-run` before enabling the
automatic hook.

## License

MIT (see [LICENSE](LICENSE)). Third-party crate notices are tracked in
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md); regenerate with
`just regen-licenses` after dependency changes (CI enforces this).
