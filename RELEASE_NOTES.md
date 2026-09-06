# waft v0.1.0

waft copies selected ignored files between Git worktrees. This initial version
ships Linux and macOS binaries for x86_64 and ARM64, with checksums and GitHub
build provenance. See INSTALL.md in each archive for verification commands.

- Preserve existing destinations by default and name skipped conflicts.
- Restore explicit Unix overwrite, with tracked-file checks under Git's index
  lock, destination revalidation, and recovery-path reporting.
- Repair files that differ only in permissions with `--overwrite`.
- Keep progress and per-file errors when selected sources vanish or change.
- Explain empty selections and skipped rule symlinks without promising that
  a policy change will validate or select files.
- Retain profile semantics, documented configuration precedence, both Git
  backends, and bounded cached tracked-path lookup.

The binary archives contain no hook installer. Optional hooks require a
reviewed source checkout. Windows has source CI coverage but no release binary,
managed hook support, or overwrite/permission-repair parity. Individual file
publication is checked; copying a batch is not an atomic transaction. Crashes
and SIGKILL can leave recovery files or a stale index lock. See ASSURANCE.md
for the tested guarantees, platform coverage, and residual limits.
