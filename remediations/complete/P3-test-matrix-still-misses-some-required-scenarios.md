# P3 — Test Matrix Still Misses Some Required Scenarios

Source: [`planning/InitialRemediationReport.md`](/Users/prb/github/wiff/planning/InitialRemediationReport.md)


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
- Add differential tests that compare explanation tuples from wiff vs `git check-ignore -v -n`.

### Validation
Gate on new tests in CI to prevent regression of these semantics.

---

