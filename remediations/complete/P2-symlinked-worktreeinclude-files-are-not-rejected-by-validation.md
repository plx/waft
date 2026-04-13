# P2 — Symlinked `.worktreeinclude` Files Are Not Rejected By Validation

Source: [`planning/InitialRemediationReport.md`](/Users/prb/github/wiff/planning/InitialRemediationReport.md)


### Deviation
Symlinked `.worktreeinclude` is intended to be a hard validation error, but currently passes validation.

Spec references:
- `InitialSpecDocument.txt` lines 253-257 (hard errors include symlinked `.worktreeinclude`).

Code evidence:
- [`src/validate.rs:87`](/Users/prb/github/wiff/src/validate.rs:87) gates logic on `entry.file_type().is_file()`.
- Symlink checks at [`src/validate.rs:91`](/Users/prb/github/wiff/src/validate.rs:91)-[`103`](/Users/prb/github/wiff/src/validate.rs:103) are inside that branch, so symlink entries are skipped.

Concrete behavior evidence:
A repo with `.worktreeinclude -> real.wti` returns `validation passed`.

### Suggested Fix
Detect `.worktreeinclude` symlinks regardless of `is_file()` and emit error.

### Validation
Add integration test: symlinked `.worktreeinclude` must make `waft validate` fail.

---

