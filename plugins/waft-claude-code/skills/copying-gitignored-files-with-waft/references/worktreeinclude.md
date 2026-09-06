# `.worktreeinclude` rules

Write rules in the **source** worktree. Usually track the rule file so the
selection is shared; keep the selected local contents ignored. Adding a name to
`.worktreeinclude` does not make it Git-ignored or untrack it.

For `claude` and `git`, positive patterns select; `!` patterns deselect.
Syntax follows `.gitignore`:

| Pattern | Meaning |
| --- | --- |
| `.env` | A basename at any depth within the rule file's scope |
| `/.env` | Only `.env` next to this rule file |
| `config/local.json` | Path relative to the rule file's directory |
| `*.env.local` | Matching basenames recursively |
| `**/*.key` | Matching key files recursively |
| `cache/` | Select a directory's contents; see negation caveat below |
| `!test.key` | Deselect a matching name, subject to ancestor selection |
| `\#name`, `\!name` | Literal leading `#` or `!` |

Blank lines and `#` comments are ignored. Trailing spaces are ignored unless
escaped. Quote globs in shell flags, not in the rule file. Case sensitivity
follows the source repository's `core.ignoreCase`.

```gitignore
# Root environment file and local service configuration
/.env
config/*.local.json
!config/test.local.json
```

## Scope and precedence

`claude` reads only root `.worktreeinclude`. `git` also reads nested files;
each is relative to its directory. Last matching rule wins within a file;
deeper matching files override shallower ones, subject to directory selection.
`wt` has different, subtractive semantics: see [profiles](profiles.md).

## Directory selection caveat

In `claude` and `git`, selecting a parent directory prevents a child negation
from deselecting its contents. For example, `secrets/` followed by
`!secrets/private.key` still selects `private.key`. Under `git`, putting
`!private.key` in `secrets/.worktreeinclude` does not evade that caveat.

Select files with a narrower pattern such as `secrets/*.key` when child
exceptions are needed, or use a separate post-selection filter:

```sh
waft copy --dry-run --extra-exclude 'secrets/private.key'
```

Check representative paths with `waft list` and a dry run, especially after
changing directory patterns. [Validation](checking.md) catches some suspicious
patterns but cannot prove the selected set matches your intent.
