---
name: copying-gitignored-files-with-waft
description: Use waft to copy Git-ignored local files into linked worktrees, edit or check .worktreeinclude rules, choose compatibility settings, or configure automatic copying after checkout.
metadata:
  short-description: Copy ignored files between worktrees with waft
---

# Copying Git-ignored files with waft

`waft` seeds linked Git worktrees with ignored local files from the main
worktree: environment files, local configuration, and reusable caches. It
copies files selected by `.worktreeinclude` and confirmed ignored and
untracked by Git. It does not create worktrees or continuously synchronize them.

The plugin supplies reference knowledge; the `waft` executable must already be
available on `PATH`.

```sh
# From the linked worktree; source defaults to the main worktree.
waft copy --dry-run
waft copy

# Explicit paths, including when running from the main worktree.
waft copy --source /path/to/main --dest /path/to/linked --dry-run

# Inspect selection or check rules without copying.
waft list -v
waft info .env
waft validate
```

Bare `waft` means `waft copy`; spell out `copy` for `--dry-run` or
`--overwrite`. Copying requires the main worktree as source and a linked
worktree in the same repository as destination. Existing destination files are
preserved unless `--overwrite` is requested; tracked files remain protected.

The default profile is `claude` (root `.worktreeinclude` only; absent means
copy nothing). The other profiles are `git` and `wt`; there is no `waft`
profile. Inspect existing policy before changing it. Preview a new rule set or
changed copy policy with the same flags intended for the real copy.

Read only the references needed for the task:

| Reference | Use for |
| --- | --- |
| [Commands and copying](references/commands.md) | Executable checks, path defaults, overwrite behavior, copy strategies |
| [`.worktreeinclude` rules](references/worktreeinclude.md) | Patterns, nesting, negation and directory caveats |
| [Compatibility profiles](references/profiles.md) | `claude`, `git`, `wt`, missing rule files, symlink policy |
| [Configuration and advanced options](references/configuration.md) | TOML, environment variables, precedence, exclusion filters |
| [Checking and troubleshooting](references/checking.md) | Linting, explaining missing files, dry-run authority |
| [Post-checkout hook](references/hooks.md) | Automatic copying with Git hooks |
| [Choosing files](references/choosing-files.md) | What to copy, regenerate, or keep worktree-specific |
