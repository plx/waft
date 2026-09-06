# Commands and copying

## Executable

Check `waft --version` if availability or version is unknown. This reference
describes waft 0.1.0. To build and install from a checkout of the waft repository
with a Rust toolchain:

```sh
cargo install --path . --locked
```

For a local build only, use `cargo build --release --locked` and
`./target/release/waft`. Do not run those build commands in the target project
unless it is the waft source checkout. No binary is bundled with the skill.

## Commands and paths

| Command | Result |
| --- | --- |
| `waft` or `waft copy` | Execute the copy plan |
| `waft copy -n` | Mutation-free plan: copies, no-ops, skips, totals |
| `waft list` | Source selection, sorted paths; no destination required |
| `waft list -v --dest /path/to/linked` | Selection with size, rule provenance, predicted destination action |
| `waft info -v .env config/local.json` | Per-path diagnostics plus resolved selection policy |
| `waft validate` | Lint source ignore/rule files; no destination required |

- With neither path flag, a linked cwd selects main → current linked worktree.
  In the main worktree, read-only commands use main; copy needs `--dest`.
- `--dest` alone uses main as source. `--source` alone does **not** infer a
  destination, even from a linked cwd; supply both flags for copying.
- `--source` can select another worktree for inspection. Copy always requires
  main → linked in the same worktree family; no arbitrary directories or
  linked → linked copying.
- `-C /path` changes invocation context, not the process cwd. Relative
  `--source`/`--dest` and `--config` paths resolve there. Relative `info` paths
  resolve from that invocation directory, mapped into the source worktree
  when invoked from a linked worktree. Project configs come from that source
  view; see [configuration](configuration.md).
- Global options work before or after the command. `-q` suppresses routine
  messages; `-v` adds detail. For a useful preview, omit `-q`.

## Copy outcomes

Missing destination files are copied; byte-identical files are no-ops.
Different untracked files are skipped unless `copy --overwrite` is used.
Tracked destination paths, type conflicts, and unsafe paths remain protected.
Use `copy --dry-run --overwrite` to preview intentional replacement.

Sources must be regular files; symlinks and special files are not copied.
Waft blocks writes through symlinked destination parents, creates needed
directories, and writes via temporary paths and rename. An eligible whole
directory can appear as `copy-dir` in the plan; empty directories are not copied.
Execution errors can leave earlier successful copies in place: this is not a
transaction across the entire worktree. No destination cleanup or deletion sync.

## Copy strategy

`--copy-strategy` changes transfer mechanics, not selection:

| Value | Behavior |
| --- | --- |
| `auto` (default) | Attempt copy-on-write on macOS; plain copy elsewhere |
| `simple-copy` | Plain byte copy |
| `cow-copy` | Attempt reflink/copy-on-write; fall back to plain copy if unsupported |

Copy-on-write keeps source and destination logically independent; it is not a
hard link. Consider `cow-copy` for large reusable caches on a supporting
filesystem. `cow-copy` does not require reflink support to succeed.
