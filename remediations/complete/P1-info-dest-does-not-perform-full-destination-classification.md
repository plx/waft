# P1 — `info --dest` Does Not Perform Full Destination Classification

Source: [`planning/InitialRemediationReport.md`](/Users/prb/github/wiff/planning/InitialRemediationReport.md)


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

