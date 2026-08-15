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
| `--overwrite` | Replace untracked destinations that differ, and repair untracked destinations whose content matches but whose permissions do not |

Without `--overwrite`, a destination that exists and differs is skipped and
left exactly as it is. Tracked destinations are never written, with or without
the flag.

`--overwrite` distinguishes two cases, and both name the file they act on:

- **untracked conflict** — content differs. The new content is prepared in a
  temporary file and swapped into place atomically, reported as `replaced:`.
- **content equal, permissions differ** — only the mode is wrong. The mode is
  fixed on the verified file descriptor and nothing is rewritten, reported as
  `repaired permissions:`. This is the expected state for files published by
  pre-release waft builds, which always wrote mode `0600`.

Every `--overwrite` action is checked against the exact bytes and mode
observed while planning. A destination that changed in between is reported as
a per-file failure and left untouched; the rest of the run continues. A
permissions repair additionally requires the pinned source and destination
snapshots to agree on content, so a destination rewritten between the byte
comparison and the snapshot is never quietly `chmod`-ed and called repaired.

Replacement uses an atomic exchange primitive (`renameat2` `RENAME_EXCHANGE` on
Linux, `renameatx_np` `RENAME_SWAP` on macOS) where the filesystem has one.
Where it has none — notably SMB, NFS, and exFAT destinations — waft moves the
destination aside with a plain rename and publishes into the name it left, with
no-clobber semantics. Nothing is unlinked by name: the displaced file is
identified after the move, and it is removed only once the replacement is
published and its name is proven to still hold it. If it turns out not to be
the file that was planned against — another writer got there first — it is
moved back and the file is reported as failed. If a name is taken in one of
those windows so that neither the publication nor the move back can happen
without clobbering, nothing is deleted: the error names the `.waft-copy-*` file
holding the prepared replacement and the `.waft-copy-*.displaced` file holding
the previous destination, so both can be recovered by hand. `--overwrite` is
not supported on Windows and reports a per-file failure there.

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

`WAFT_GIT_BACKEND` selects the Git implementation: `gix` (the default,
in-process) or `cli` (shells out to `git`). The value is trimmed and matched
without regard to ASCII case, so `cli` and `CLI` are the same choice. Any other
value is an error naming the valid ones rather than a silent fall back to the
default, since the two backends are what enforce waft's safety checks.

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
- **No accidental replacement** — a destination that did not exist while
  planning is published with no-clobber semantics and never overwrites a path
  that appeared in the meantime
- **Proven replacement only** — `--overwrite` re-opens the destination through
  the anchored parent with `O_NOFOLLOW` and requires its device, inode,
  length, content fingerprint, and mode to still match the planning snapshot.
  Replacement is a single atomic exchange whose swapped-out file is re-checked
  against that same snapshot before it is unlinked; a lost race is exchanged
  back and reported as a per-file failure — unless the destination name has
  been taken again in the meantime, in which case nothing is moved back and
  both files are named in the error. Without an exchange primitive the
  destination is moved aside rather than unlinked, and put back if what moved
  turns out not to be the planned file. A permissions repair also requires
  the pinned source and destination snapshots to agree on content, so the
  "content is already equal" premise is re-established at publication time
  rather than inherited from an earlier comparison, and the mode is read back
  afterwards so a repair another writer `chmod`-ed away is reported as a
  failure instead of a success
- **Nothing unverified is deleted** — waft only unlinks a name it has just
  proven still holds the inode it planned against, including from the cleanup
  that runs when a publication unwinds. If a recovery step fails and leaves
  another writer's file under a `.waft-copy-*` name, that file is kept and
  named in the error rather than cleaned up. POSIX has no conditional unlink,
  so a two-syscall window between that proof and the removal remains — on a
  fresh 128-bit random name nothing but waft creates
- **Per-file failures stay per-file** — a source that vanishes, a destination
  that changes mid-flight, or a locked index affects only that file's result
  and exit accounting; the rest of the run proceeds
- **Descriptor-anchored traversal on Unix** — source and destination
  components are opened relative to canonical worktree directory handles with
  `O_NOFOLLOW`; source state is matched to its planning snapshot, and a
  destination parent is revalidated before publication
- **Durable atomic visibility** — file contents are synced to a temp file
  before publication, then the parent directory is synced on Unix
- **No orphan temporaries** — `.waft-copy-*` staging files are removed by a
  drop guard, including when a publication unwinds, as long as the name still
  holds the file waft put there; one another process has taken over is left
  alone instead
- **Interrupt-safe index locking** — on Unix, `SIGINT` and `SIGTERM` unlink the
  live `index.lock` before re-raising under their previous disposition, so an
  interrupt during the publish window cannot leave a stale lock. A signal that
  arrives while the lock is still being created is recorded and acted on the
  moment its ownership is known, rather than re-raised into a process that
  would die with the lock on disk or delete a lock another writer holds. A
  signal the process inherited as ignored is left ignored: waft would otherwise
  drop its own lock and keep publishing without it. Other platforms have no
  such handler; an interrupt there may require deleting `.git/**/index.lock` by
  hand
- **Dry-run is mutation-free** — `--dry-run` reads only, writes nothing, and
  reports planning failures on stderr with the same nonzero exit the real run
  would produce

Normal Git writers honor the index lock and cannot change trackedness across
the final check and publication. A process that edits the index directly while
ignoring Git's lock protocol remains outside that guarantee. Unix directory
handles prevent a symlink or name swap from redirecting publication to a
different directory object. An already-open authorized directory can still be
relocated by another process after final revalidation; avoid concurrent
directory relocation when the destination pathname itself must remain stable.
For immediacy without holding Git's index lock during content preparation,
waft reacquires the lock and rechecks the index for each published file. The
recheck reuses one index snapshot per repository, revalidated by a single
`stat` of the index file — Git publishes a new index by renaming `index.lock`
into place, so a change to trackedness always invalidates the snapshot. A
recheck therefore costs a stat and a hash lookup per file rather than a fresh
index read (and, for the Git CLI backend, a subprocess) under the lock.

### Case sensitivity

Whether `Secret.env` and `secret.env` name the same path is decided by the
repository's own `core.ignoreCase`, which Git records per checkout after
probing the filesystem. When it is true, a differently-cased spelling of a
tracked path is protected as that tracked path, and a differently-cased
spelling of a registered submodule directory is treated as that submodule
boundary. When it is false, tracked-path comparison is exact — a
case-sensitive volume really does hold two distinct files — except that a
tracked path is still protected when the filesystem itself resolves both
spellings to the same entry. If the key is absent entirely, macOS and Windows
fall back to folding and other platforms to exact comparison.

Repository-boundary comparisons (registered submodules during walks and
project-config discovery) stay folded on macOS and Windows regardless of the
key: those filesystems may alias case even when `core.ignoreCase` is false,
and treating a case-aliased submodule directory as ordinary source content
would select another repository's files. On other platforms the key decides,
so a case-sensitive checkout keeps `Vendor/` distinct from a registered
`vendor/` submodule.

This answer is read once per run, so a single invocation cannot apply folded
protection to some paths and exact matching to others. Editing `core.ignoreCase`
while waft is running has no effect on that run.

Two pathnames that a filesystem aliases in a way neither Unicode case folding
nor an exact match detects (for example a deliberately hard-linked second name
for a tracked file) are treated as distinct paths.

Exclusion patterns are matched separately and more conservatively: on macOS and
Windows they stay case-insensitive even when `core.ignoreCase` is false, since
withholding a file is safer than leaking one.

Lock acquisition retries briefly (three attempts over roughly 150ms) so a
short-lived Git writer does not become a per-file failure; a lock still held
after that fails the file with an explicit message and is never removed.

Replacement under `--overwrite` proves the outgoing file's identity *and*
content, so an in-place rewrite of the same inode is caught rather than
clobbered. The remaining window is inherent to POSIX: on filesystems without an
exchange primitive, waft must move the proven file aside and then publish, so a
file recreated at that pathname in between causes a reported failure rather than
a replacement. Nothing is deleted in that case — the file that was there is
under the `.waft-copy-*.displaced` name given in the error, and waft's prepared
content under the `.waft-copy-*` name — but the destination pathname is briefly
vacant while the move is made, and it ends up holding whichever file won the
race. Cleanup itself is subject to the same limit: waft proves that a name still
holds the file it created immediately before unlinking it, and POSIX has no way
to make those two steps one.


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
