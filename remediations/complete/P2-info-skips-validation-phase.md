# P2 — `info` Skips Validation Phase

Source: [`planning/InitialRemediationReport.md`](/Users/prb/github/wiff/planning/InitialRemediationReport.md)


### Deviation
`info` does not run validation before reporting, contrary to the planned command pipeline.

Plan/spec references:
- `InitialSpecDocument.txt` line 232 (validation before planning for all commands).
- The architecture note/pipeline in `docs/architecture.md` also states validation as the common stage.

Code evidence:
- [`src/cli.rs:259`](/Users/prb/github/wiff/src/cli.rs:259)-[`406`](/Users/prb/github/wiff/src/cli.rs:406) has no validation call.

Concrete behavior evidence:
With unreadable `.gitignore`:
- `wiff validate --source <repo>` fails (non-zero)
- `wiff info --source <repo> <path>` still succeeds and reports statuses

### Suggested Fix
Mirror `copy`/`list` behavior in `run_info`: run `validate::validate`, fail on errors, and print warnings consistently.

### Validation
Add integration test asserting `info` exits non-zero when validation has in-repo errors.

---

