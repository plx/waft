# Parity Summary (waft vs wt vs claude)

## Re-run Commands

- Run the full 3-way wrapper (Claude temp worktree smoke + parity harness):

```sh
just check-worktrunk-parity-3way
```

- Run only the Claude worktree smoke check (create + cleanup in current repo):

```sh
just check-claude-worktree-smoke
```

- Run only the parity harness:

```sh
just check-worktrunk-parity
```

The harness writes the detailed matrix to:

```text
.context/worktrunk_parity_report.md
```

## Scenario Matrix

The harness covers these scenarios:

- root-level `.worktreeinclude`
- no `.worktreeinclude`
- nested `.worktreeinclude` override
- nested anchored pattern (`/foo`) in nested `.worktreeinclude`
- cross-file negation caveat (`dir/` in root + nested `!file`)
- nested linked worktree in repo (`.worktrees/`)
- tool-state directory selection (`.conductor/`)
- symlinked `.worktreeinclude`

## Current 3-Way Results

From the latest run in this workspace:

- Scenarios: 8
- All three agree: 2
- Pairwise agreement (`waft=wt`): 2
- Pairwise agreement (`waft=claude`): 5
- Pairwise agreement (`wt=claude`): 4

### Scenario-by-Scenario Outcome

1. `root-simple`
   - All three agree (`.env` copied).

2. `no-worktreeinclude`
   - `waft=claude` (none copied).
   - `wt` copied ignored files by default (`.env`, `cache/build.bin`).

3. `nested-worktreeinclude-override`
   - `wt=claude` (`root.env`, `config/sub.env`).
   - `waft` copied only `root.env`.

4. `nested-anchored-pattern`
   - No pair agrees.
   - `waft`: `config/foo`
   - `wt`: `config/foo`, `foo`
   - `claude`: none

5. `cross-file-negation-caveat`
   - `waft=claude` (`secrets/private.key`).
   - `wt`: none.

6. `nested-worktree-in-repo`
   - All three agree (none copied).

7. `tool-state-directory`
   - `waft=claude` (`.conductor/state/dev.key`).
   - `wt`: none.

8. `symlinked-worktreeinclude`
   - `wt=claude` (`.env` copied).
   - `waft` failed validation (rejects symlinked `.worktreeinclude`).

## Command Used for Claude Worktree Creation

The tested headless command shape is:

```sh
claude --worktree --model haiku -p --permission-mode bypassPermissions --no-session-persistence --output-format text "State the full path of your current working directory (CWD)."
```

This reliably creates a temporary worktree and returns its path in stdout; the wrapper command extracts that path and removes it with `git worktree remove --force`.
