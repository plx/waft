# P3 — `git worktree list --porcelain -z` Parsing Is Not Faithful To Actual Format

Source: [`planning/InitialRemediationReport.md`](/Users/prb/github/wiff/planning/InitialRemediationReport.md)


### Deviation
The parser/test model for worktree porcelain `-z` output does not match Git’s actual line delimiting behavior.

Plan references:
- `ImplementationPlan.txt` step 5, lines 108 and 115 (porcelain `-z`, parser-tested with NUL-delimited outputs).

Code evidence:
- [`src/git.rs:253`](/Users/prb/github/wiff/src/git.rs:253)-[`285`](/Users/prb/github/wiff/src/git.rs:285) treats each NUL chunk as a multiline record.
- Tests in [`src/git.rs:344`](/Users/prb/github/wiff/src/git.rs:344)-[`368`](/Users/prb/github/wiff/src/git.rs:368) use newline-containing record payloads not representative of real `-z` output.

Impact:
- Path extraction works in common cases, but metadata handling (e.g., `bare`) is brittle/inaccurate.

### Suggested Fix
Implement a stateful parser over NUL-separated fields, starting a new record at `worktree <path>` and consuming subsequent attributes until separator/next record.

### Validation
Add parser tests based on byte streams captured from actual `git worktree list --porcelain -z` output, including linked and bare entries.

---

