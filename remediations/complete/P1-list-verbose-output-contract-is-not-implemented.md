# P1 — `list --verbose` Output Contract Is Not Implemented

Source: [`planning/InitialRemediationReport.md`](/Users/prb/github/wiff/planning/InitialRemediationReport.md)


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

