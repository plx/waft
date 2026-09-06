# Configuration and advanced options

Precedence, low → high: built-ins → user TOML → source-project TOMLs →
`WAFT_*` environment → CLI. The user file is
`$XDG_CONFIG_HOME/waft/config.toml`, or `~/.config/waft/config.toml`.
`--config /absolute/file.toml` overrides `WAFT_CONFIG_PATH`, which overrides
user-file discovery. This replaces only the user layer, not project/env layers.
Project files are named `.waft.toml` and come from the resolved **source**
worktree, from its root down to the invocation directory's equivalent source
path. A linked cwd or `-C` maps to that source path; an unrelated cwd uses the
source root. Destination-branch configs do not control copying. Project configs
must be regular files: symlinks fail, and discovery stops at nested repositories
and registered submodules.

Layers apply in order. Selecting a profile resets its four owned settings
(missing-rule behavior, semantics, symlink policy, built-in excludes); explicit
settings in the same or a later layer then override it. For example, CLI
`--compat-profile claude` resets user `semantics = "git"` to `claude-2026-04`.
Other scalar settings use the last explicit value. `waft info -v .env` shows
resolved selection policy (not copy strategy).

`--isolated` omits user config, `WAFT_CONFIG_PATH`, and `WAFT_*` policy
environment variables. Source-project configs and CLI overrides still apply,
as does operational backend selection (`WAFT_GIT_BACKEND`). It conflicts with
`--config` and is the mode used by the managed checkout hook.

## CLI / TOML / environment mapping

TOML keys below use `table.key` notation. Values are the same across interfaces.

| CLI flag | TOML key | Environment variable |
| --- | --- | --- |
| `--compat-profile` | `compat.profile` | `WAFT_COMPAT_PROFILE` |
| `--when-missing-worktreeinclude` | `worktreeinclude.when_missing` | `WAFT_WHEN_MISSING_WORKTREEINCLUDE` |
| `--worktreeinclude-semantics` | `worktreeinclude.semantics` | `WAFT_WORKTREEINCLUDE_SEMANTICS` |
| `--worktreeinclude-symlink-policy` | `worktreeinclude.symlink_policy` | `WAFT_WORKTREEINCLUDE_SYMLINK_POLICY` |
| `--builtin-exclude-set` | `exclude.builtin_set` | `WAFT_BUILTIN_EXCLUDE_SET` |
| `--extra-exclude` (repeatable) | `exclude.extra` (array) | `WAFT_EXTRA_EXCLUDE` (comma-separated) |
| `--replace-extra-excludes` | `exclude.replace_extra` (boolean) | `WAFT_REPLACE_EXTRA_EXCLUDES` (boolean) |
| `--copy-strategy` | `copy.strategy` | `WAFT_COPY_STRATEGY` |

See [profiles](profiles.md) for selection values and
[commands](commands.md) for copy strategies. Use full CLI spellings; config
aliases are not necessarily accepted by CLI enum parsing.

Example `.waft.toml` (underscores in TOML keys, hyphens in enum values):

```toml
version = 1

[compat]
profile = "git"

[exclude]
extra = ["*.bak", "logs/"]

[copy]
strategy = "auto"
```

Unknown keys, bad enum values, and unsupported versions fail config parsing.

## Extra excludes

`extra` arrays append across layers. `replace_extra = true` (or the CLI/env
equivalent) clears inherited extras at that layer, then adds its new entries.
An empty array by itself does not clear inherited entries. This does not clear
the built-in set.

```sh
waft list --replace-extra-excludes --extra-exclude '*.bak' --extra-exclude 'logs/'
```

Excludes use Gitignore-style patterns rooted at the source. They run after
selection and cannot add unselected or non-ignored files. Negations in the
exclude matcher can exempt a selected file from an earlier filter, subject to
Git's directory caveats.

`tooling-v1` excludes these directories and their contents:
`.conductor/`, `.claude/`, `.worktrees/`, `.git/`, `.jj/`, `.hg/`, `.svn/`,
`.bzr/`, `.pijul/`, `.sl/`, `.entire/`, `.pi/`. It is a fixed set, not a
complete inventory of agent state. Use `none` only when those otherwise
eligible paths should be included, then add narrower excludes as needed.
