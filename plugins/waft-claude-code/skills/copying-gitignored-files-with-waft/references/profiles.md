# Compatibility profiles

These are waft's versioned compatibility behaviors, not a claim about every
version of Claude Code or Worktrunk. `waft` is the executable; `git` names its
Git-style mode. There is no `--compat-profile waft`.

| `--compat-profile` | Missing rules | `--worktreeinclude-semantics` | Rule-file symlinks | Built-in excludes |
| --- | --- | --- | --- | --- |
| `claude` (default) | Select nothing | `claude-2026-04`: root file only | `follow` | `none` |
| `git` | Select nothing | `git`: nested, per-directory rules | `ignore` | `none` |
| `wt` | All ignored, untracked files | `wt-0.39`: all ignored minus literal path negations | `follow` | `tooling-v1` |

Choose `claude` for root-only selection, `git` for nested rules, `wt` to
reproduce waft's Worktrunk 0.39 compatibility behavior. Agent host does not
determine the right profile: Codex can use any of them.

## `wt` is subtractive

Even with rule files present, `wt-0.39` starts from **all** Git-ignored,
untracked files. Positive lines do not restrict selection. Glob negations such
as `!*.key` are ignored. A literal `!private.key` removes that exact path
relative to its rule file, not every basename at every depth. `!cache/` does
not recursively exclude its contents; use `--extra-exclude 'cache/'`.

For example, with `.env` and `cache/build.bin` ignored, a rule file containing
only `.env` selects just `.env` under `claude`/`git`, but both under `wt`.
Use `waft list --compat-profile wt` before adopting it.

## Independent knobs

- `--when-missing-worktreeinclude blank|all-ignored` applies only when no
  `.worktreeinclude` exists anywhere in the source tree under the symlink
  policy. An empty file still counts as present. A nested-only file also
  suppresses fallback, even though `claude` does not use its patterns.
- `--worktreeinclude-symlink-policy follow|ignore|error` controls **rule files**:
  read the symlink target, treat it as absent, or fail validation. This does
  not permit copying symlinked payloads.
- `--builtin-exclude-set none|tooling-v1` and `--extra-exclude` filter the
  selected set independently of the matcher.

A profile selected in a higher layer resets its four owned knobs; explicit
knobs in that same layer or a later layer can override the preset. See
[configuration](configuration.md) when the profile name alone does not explain
behavior.
