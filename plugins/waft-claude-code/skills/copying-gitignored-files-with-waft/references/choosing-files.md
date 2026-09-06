# Choosing files

Copy local state when the new worktree needs the same starting contents and
can diverge afterward. Prefer regenerating cheap, branch-dependent output.
Waft provides a snapshot, not shared live state.

| Candidate | Decision |
| --- | --- |
| `.env`, `.env.local`, local service configuration | Useful when endpoints, credentials, ports, and database names apply to the new worktree; adjust worktree-specific values afterward |
| Local development certificates or credentials | Select specific paths needed for the task; keep contents ignored and out of diagnostics |
| Downloaded assets, models, test datasets | Good when expensive to reproduce and reusable across branches |
| Compiler or build caches | Useful if the tool handles invalidation; copy-on-write can reduce initial cost |
| `node_modules`, virtual environments, generated SDKs | Conditional: check lockfile, runtime, absolute paths, native modules, and symlinks; reinstall/regenerate when those differ |
| Local database snapshots | Copy a quiescent snapshot; copying files from a running database is not a consistency mechanism |
| Logs, sockets, PID/lock files, active sessions | Usually leave behind; runtime state is worktree/process-specific |
| Agent/tool working state | Select deliberate inputs rather than entire state trees; `wt` filters several such directories via `tooling-v1` |
| Tracked config/templates | Already supplied by checkout; waft does not copy tracked source files |

Start with exact paths or narrow patterns in the source `.worktreeinclude`:

```gitignore
/.env.local
/config/dev.local.json
/datasets/reference.bin
```

These paths must also be ignored by Git. Check selection with `waft list` and
preview destination effects with `waft copy --dry-run`. Do not widen to `wt`
or `all-ignored` just to make one missing file appear; first inspect its ignore
rule, include rule, and active policy.
