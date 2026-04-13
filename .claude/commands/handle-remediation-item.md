---
argument-hint: [path-to-remediation-item]
user-invocable: true
disable-model-invocation: true
---

Handle a single remediation item end-to-end. The remediation file path is: $ARGUMENTS.

## Workflow

Follow these steps in order. Do not skip steps.

### 1. Read and understand the issue

Read the remediation file at the path above. It describes a deviation from the project's implementation plan, with code evidence, scope, suggested fix, and validation criteria.

Then do whatever research you need — read the referenced source files, the implementation plan sections, related tests, etc. — until you fully understand the problem and what the fix should look like.

### 2. Write tests first

Before changing any production code, introduce unit tests and/or integration tests that exercise the broken behavior described in the remediation. These tests should **fail** against the current code (confirming the bug exists) and **pass** once the fix is applied.

Run `cargo test` to confirm the new tests fail as expected.

### 3. Implement the fix

Make the minimum changes needed to address the issue. Follow the suggested fix in the remediation file as guidance, but use your judgment — the suggestion is a starting point, not a mandate.

### 4. Verify

Run the full test suite (`cargo test`) and confirm:
- All new tests pass
- All pre-existing tests still pass
- Clippy is clean (`cargo clippy -- -D warnings`)

If anything fails, fix it before proceeding.

### 5. Commit the fix

Stage and commit all changed and new files with a descriptive commit message summarizing what was fixed and why. Use conventional-commit style (e.g., `fix: ...` or `test: ...`). End the commit message with:

```
Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
```

### 6. Codex review

Invoke the `/codex:rescue` skill to review **only your fix** — not the broader project. Ask it to check:
- Correctness of the fix relative to the remediation description
- Whether any edge cases were missed
- Code quality and idiomatic Rust

### 7. Address Codex feedback

If Codex identifies issues, address them. Then re-run `cargo test` and `cargo clippy -- -D warnings` to confirm everything still passes.

### 8. Commit review fixes

If you made changes in step 7, commit them with a message like:

```
fix: address review feedback for <remediation-name>

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
```

### 9. Move the remediation file

Move the file from `remediations/pending/` to `remediations/complete/`:

```bash
git mv remediations/pending/<filename> remediations/complete/<filename>
```

### 10. Final commit

Commit the move:

```
chore: mark <remediation-name> as complete

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
```

## Important notes

- Work carefully and methodically. Quality matters more than speed.
- Do not modify files unrelated to the remediation.
- If the fix touches architectural boundaries (e.g., module responsibilities), respect the project's existing design.
- If you get stuck or the issue is ambiguous, document what you found and what you tried rather than guessing.
