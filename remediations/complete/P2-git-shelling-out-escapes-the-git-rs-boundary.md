# P2 — Git Shelling-Out Escapes The `git.rs` Boundary

Source: [`planning/InitialRemediationReport.md`](/Users/prb/github/wiff/planning/InitialRemediationReport.md)


### Deviation
`validate.rs` directly shells out to `git` to read global excludes config, violating the architecture boundary.

Plan references:
- `ImplementationPlan.txt` step 5, lines 96-97 (`git.rs` should be the only module shelling out to Git).

Code evidence:
- [`src/validate.rs:170`](/Users/prb/github/wiff/src/validate.rs:170)-[`175`](/Users/prb/github/wiff/src/validate.rs:175) invokes `Command::new("git")` directly.

### Scope
This is a cross-cutting architectural concern affecting testability and future backend substitution.

### Suggested Fix
Move global excludes discovery behind `GitBackend` (e.g., new method), and keep all Git process calls in `git.rs`.

### Validation
- Unit-test validation with mock `GitBackend` input for global excludes path.
- Confirm no direct Git process invocation remains outside `git.rs`.

---

