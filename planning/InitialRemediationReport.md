# Initial Remediation Report (Specification Adherence Audit)

This report audits the current implementation against `planning/ImplementationPlan.txt` (with `InitialSpecDocument.txt` and `RipGrepIgnoreExtraction.txt` used as clarifying intent where the plan is terse).

## Summary

The codebase is in solid shape overall (crate structure, command surface, Git-backed selection path, planner/executor split, CI, and broad test coverage are present), but there are several material deviations from the planned behavior and architecture.

The highest-risk deviations are around explanation/reporting correctness (`.worktreeinclude` explanation semantics), and incomplete command contracts for `list --verbose` and `info --dest`.

---

## P1 — `.worktreeinclude` Explanation Semantics Are Incorrect In Key Cases

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

## P1 — `list --verbose` Output Contract Is Not Implemented

### Deviation
Verbose `list` output does not include the required explanatory fields and predicted action behavior.

Plan references:
- `ImplementationPlan.txt` step 9, line 187 (verbose includes ignore-source details + `.worktreeinclude` explanation).
- `ImplementationPlan.txt` step 11, line 227 (with `--dest`, planner reuse for predicted action).
- `InitialSpecDocument.txt` lines 355-362 (source size, Git ignore source/line/pattern, `.worktreeinclude` source/line/pattern, predicted action).

Code evidence:
- [`src/cli.rs:247`](/Users/prb/github/wiff/src/cli.rs:247)-[`250`](/Users/prb/github/wiff/src/cli.rs:250) prints only `path\t{wti:?}`.
- `--dest` is accepted globally but not used to render predicted actions in `list`.

Concrete behavior evidence:
```text
$ waft list --source <repo> --dest <linked> -v
.env    Included { file: ".../.worktreeinclude", line: 1, pattern: ".env" }
```
No Git ignore explanation, no source size, no predicted action.

### Suggested Fix
- Preserve `check-ignore` match metadata in the list rendering path.
- Add structured verbose rendering (size + Git rule + `.worktreeinclude` rule).
- If destination is available, invoke planner classification and print predicted action.

### Validation
- Add integration tests for verbose output shape (`list -v` with and without `--dest`).
- Add tests asserting predicted action changes under missing/up-to-date/untracked-conflict/tracked-conflict.

---

## P1 — `info --dest` Does Not Perform Full Destination Classification

### Deviation
`info` uses ad hoc destination checks and does not classify destination state per planner rules (notably tracked conflicts).

Plan references:
- `ImplementationPlan.txt` step 10, lines 202-203 (classify destination state + predicted action when destination is known).
- `ImplementationPlan.txt` step 11, lines 212-219 (full destination-state taxonomy).
- `InitialSpecDocument.txt` lines 311-329 and 367-375.

Code evidence:
- [`src/cli.rs:373`](/Users/prb/github/wiff/src/cli.rs:373)-[`400`](/Users/prb/github/wiff/src/cli.rs:400) checks only existence/type/content equality and never queries destination trackedness.

Concrete behavior evidence:
When destination `.env` is tracked, `info` reports:
```text
destination: exists (differs)
planned_action: skip (conflict)
```
instead of explicit tracked conflict classification.

### Scope
Any workflow relying on `info` for decision quality (especially pre-copy conflict triage) is affected.

### Suggested Fix
- Reuse planner destination-classification logic (or extract shared classifier) in `info`.
- Query destination trackedness via `git ls-files --cached` against destination root.
- Emit destination status values aligned with `DestinationState` + planned action mapping.

### Validation
- Integration tests for `info --dest` covering: tracked conflict, untracked conflict, up-to-date, type conflict, unsafe path.

---

## P2 — `info` Skips Validation Phase

### Deviation
`info` does not run validation before reporting, contrary to the planned command pipeline.

Plan/spec references:
- `InitialSpecDocument.txt` line 232 (validation before planning for all commands).
- The architecture note/pipeline in `docs/architecture.md` also states validation as the common stage.

Code evidence:
- [`src/cli.rs:259`](/Users/prb/github/wiff/src/cli.rs:259)-[`406`](/Users/prb/github/wiff/src/cli.rs:406) has no validation call.

Concrete behavior evidence:
With unreadable `.gitignore`:
- `waft validate --source <repo>` fails (non-zero)
- `waft info --source <repo> <path>` still succeeds and reports statuses

### Suggested Fix
Mirror `copy`/`list` behavior in `run_info`: run `validate::validate`, fail on errors, and print warnings consistently.

### Validation
Add integration test asserting `info` exits non-zero when validation has in-repo errors.

---

## P2 — Symlinked `.worktreeinclude` Files Are Not Rejected By Validation

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

## P2 — Git Shelling-Out Escapes The `git.rs` Boundary

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

## P3 — `git worktree list --porcelain -z` Parsing Is Not Faithful To Actual Format

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

## P3 — Validation Warning Policy Is Incomplete (No Suspicious-Pattern Warnings)

### Deviation
Validation currently reports parse/read failures only; policy-required warning classes are missing.

Plan references:
- `ImplementationPlan.txt` step 7, lines 146-148 (global invalid as warning; suspicious but legal patterns as warnings).
- `InitialSpecDocument.txt` lines 261-263 (patterns matching nothing, suspicious negations likely shadowed).

Code evidence:
- `src/validate.rs` has no heuristic checks for suspicious-but-legal pattern cases.

### Suggested Fix
Add non-fatal lint-style validators for suspicious patterns and include them as `ValidationSeverity::Warning`.

### Validation
Add targeted unit tests for warning heuristics (without turning valid patterns into errors).

---

## P3 — Test Matrix Still Misses Some Required Scenarios

### Deviation
Despite strong overall coverage, specific required scenarios are not fully represented.

Plan references:
- `ImplementationPlan.txt` step 12, line 245 (integration coverage should include tracked destination conflicts and symlink safety).
- `ImplementationPlan.txt` step 13, lines 256-260 (differential checks should include per-path ignore explanation parity via `git check-ignore -v -n`).

Current gaps:
- `tests/copy_integration.rs` does not include tracked-destination-conflict integration coverage.
- No differential suite asserting per-path explanation parity (source/line/pattern), only selected-set parity.

### Suggested Fix
- Add missing integration cases to `tests/copy_integration.rs`.
- Add differential tests that compare explanation tuples from waft vs `git check-ignore -v -n`.

### Validation
Gate on new tests in CI to prevent regression of these semantics.

---

## Notes

Notable areas that do adhere well to plan:
- repository/module layout and docs scaffold
- default-subcommand dispatch model (`None => copy`)
- Git-backed candidate-first selection in `list`/`copy`
- plan/execute split and deterministic planner sorting
- cross-platform CI + release workflow scaffolding

