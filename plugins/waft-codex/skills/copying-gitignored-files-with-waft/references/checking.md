# Checking and troubleshooting

## Lint without copying

```sh
# Inspect this checkout, even if it is a linked worktree.
waft validate --source .

# Inspect a chosen source from a predictable config context.
waft -C /path/to/main validate --source /path/to/main
```

Validation checks discovered `.gitignore` and `.worktreeinclude` files, the
source's `.git/info/exclude` when present at that path, and configured global
excludes. It reports file/line diagnostics for invalid patterns, unreadable
files, duplicate patterns, and some ineffective child negations.

- In-repo parse/read errors fail; global-exclude issues and suspicious legal
  patterns warn. Warnings alone exit successfully; there is no strict or
  warnings-as-errors flag and no auto-fix mode.
- Runtime/config/validation errors exit 1; CLI usage errors exit 2. A successful
  copy can still contain skips: exit status alone does not prove all files copied.
- Validation scans nested rule files even when `claude` selection ignores them.
  A malformed nested file can therefore block copying.
- `copy`, `list`, and `info` validate first too. `validate` is a syntax/lint
  check, not a proof of selection or a full validation of extra-exclude globs.
- In a linked worktree, bare `waft validate` defaults to the **main** source;
  `--source .` is needed to lint edits in the current linked checkout. When that
  checkout has a `.git` file, validation does not resolve its shared
  `info/exclude` through the common Git directory.

## Explain a missing or skipped file

```sh
waft -C /path/to/main info -v --source /path/to/main --dest /path/to/linked .env
waft list -v --source /path/to/main --dest /path/to/linked
waft copy --source /path/to/main --dest /path/to/linked --dry-run
```

Check in order: source/path resolution → regular file exists → untracked and
Git-ignored → active rule semantics → built-in/extra exclusions → destination
conflict. `git -C /path/to/main check-ignore -v -- .env` can independently
explain Git's ignore decision without reading the file's contents.

`info` reports rule provenance, tracked/ignored status, source kind, and
destination state; `-v` also prints resolved selection policy. Its eligibility
uses the same selection pipeline as `list` and `copy`, including profiles,
missing-rule fallback, extra excludes, and regular-file checks. Destination
diagnostics assume overwrite is disabled. Use `list` for the selected set and
**`copy --dry-run` for the actual plan**, with the same profile, excludes,
paths, and overwrite flag as the intended copy.
