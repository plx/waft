# P3 — Validation Warning Policy Is Incomplete (No Suspicious-Pattern Warnings)

Source: [`planning/InitialRemediationReport.md`](/Users/prb/github/wiff/planning/InitialRemediationReport.md)


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

