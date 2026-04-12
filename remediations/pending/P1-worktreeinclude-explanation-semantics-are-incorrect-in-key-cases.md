# P1 — `.worktreeinclude` Explanation Semantics Are Incorrect In Key Cases

Source: [`planning/InitialRemediationReport.md`](/Users/prb/github/wiff/planning/InitialRemediationReport.md)


### Deviation
The explanation engine in `src/worktreeinclude.rs` does not fully implement Git-style per-directory semantics as required.

Plan references:
- `ImplementationPlan.txt` step 8, lines 155-160 and 166-168 (collect applicable files, evaluate shallow->deep, Git-style precedence, cover anchored/negation/`**` cases).
- `InitialSpecDocument.txt` lines 83-94 and 266-280 (Git-compatible semantics; explanation engine parity expectations).

Code evidence:
- [`src/worktreeinclude.rs:91`](/Users/prb/github/wiff/src/worktreeinclude.rs:91) builds a fresh matcher per line, not per file context.
- [`src/worktreeinclude.rs:109`](/Users/prb/github/wiff/src/worktreeinclude.rs:109) uses `file_dir.join(rel_path)`, which duplicates directory segments for nested matcher roots.

Concrete behavior evidence:
1. Nested anchored pattern mismatch:
```text
config/.worktreeinclude: /foo
query path: config/foo
expected: included (Git selects it)
actual: worktreeinclude: no match
```
2. Git negation caveat mismatch:
```text
.worktreeinclude:
  dir/
  !dir/keep
query path: dir/keep
expected: still selected (Git caveat)
actual: excluded by negation
```

### Scope
This affects all consumers of explanation output (`info`, `list -v`) and unit tests in `src/worktreeinclude.rs` that currently miss these edge cases.

### Suggested Fix
- Rework evaluation to model true per-file Gitignore semantics (not isolated single-line matchers).
- Ensure path matching is done against the correct root-relative path for each applicable `.worktreeinclude` file.
- Keep line/pattern provenance from the winning rule as first-class output.

### Validation
Add/adjust tests for:
- nested anchored patterns (`sub/.worktreeinclude` with `/foo`)
- Git negation caveat scenarios (`dir/` + `!dir/keep`)
- deeper-file precedence with anchored + negated combinations

---

