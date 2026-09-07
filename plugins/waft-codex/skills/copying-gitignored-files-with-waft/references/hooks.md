# Post-checkout hook

Use a Git `post-checkout` hook when ignored files should be seeded after
creating or switching a linked worktree. This is a Git hook, separate from
Claude Code plugin hooks. The skill does not install or enable it automatically.

## Install a reviewed snapshot

Run the installer from a reviewed checkout of the waft repository, with the
target repository as the working directory and an absolute path to an existing
waft binary to snapshot:

```sh
cd /path/to/target-repository
WAFT=/absolute/path/to/waft bash /path/to/reviewed-waft/scripts/install-hooks.sh
```

This installs a copy of the binary, hook, and dispatcher under the repository's
common Git directory. An absolute `core.hooksPath` points there, so changing
branches cannot replace the installed code. Re-run the installer after
upgrading waft.

Inspect existing hook configuration first. The installer chains the previously
effective trusted hooks and rejects hooks sourced from a worktree, symlinked
hooks, and worktree-scoped `core.hooksPath` overrides. Preserve existing hook
actions when resolving an installer refusal. Do not point `core.hooksPath` at
a tracked `hooks/` directory: a checkout can replace that code before Git runs it.

Restore the previous hook configuration by invoking the reviewed installer
with `--uninstall` from the target repository.

## Execution policy

The managed hook invokes its pinned sibling binary with `--isolated`. User
config, `WAFT_CONFIG_PATH`, and `WAFT_*` policy variables do not affect automatic
selection; source-project `.waft.toml` files still apply. Ambient `WAFT` and
`PATH` cannot replace the managed binary. Operational `WAFT_GIT_BACKEND` remains
available. See [configuration](configuration.md) for source-config discovery.

## Trigger behavior and verification

The hook reacts to branch checkouts with a changed HEAD in a linked worktree,
including creation with `git worktree add`. File checkouts, unchanged HEAD,
the main worktree, and creation with `--no-checkout` do not trigger copying.
Default copying fills missing files and preserves conflicting local files.

Preview the managed hook's selection with the installed binary and the same
isolated policy, from the linked worktree:

```sh
"$(git rev-parse --git-common-dir)/waft-hooks/.waft-bin" --isolated copy --dry-run
```

To test without invoking an agent, use a disposable linked worktree and
synthetic ignored files; inspect that only selected contents appeared. A waft
failure is reported by the hook but does not undo the checkout or earlier copies.
