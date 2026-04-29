# Worktreeinclude Modes: Implementation PR Plan (Post-gix Rebase)

## Purpose

This is the companion execution plan for:

- [WorktreeincludeConfigAndFixtureMatrix.md](/Users/prb/conductor/workspaces/waft/barcelona/planning/WorktreeincludeConfigAndFixtureMatrix.md)

The plan is intentionally sequenced for safe delivery after rebasing on the just-merged `gix` backend migration.

## Execution Constraints

- Do not begin feature implementation before rebasing onto the `gix` backend branch.
- Keep each PR behaviorally narrow and test-gated.
- Prefer introducing policy knobs first, then wiring behavior incrementally.
- Preserve current behavior behind defaults until the final default switch PR.

## Global Gate (Run At End of Every PR)

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --doc
```

For parity-sensitive PRs, also run:

```sh
just check-worktrunk-parity-3way
```

## PR0: Rebase + Baseline Stabilization

### Goal

Land on top of the `gix` migration and ensure current parity harness still runs.

### Scope

- Rebase branch onto latest `main` with `gix` changes.
- Resolve compile conflicts only; no intentional behavior change.
- Re-run current parity harness and capture baseline report artifact.

### Expected Files

- `src/git.rs` (or equivalent new backend module from migration)
- `tests/worktrunk_parity.rs` (only if API wiring changed)
- `planning/WorktrunkParityReport.md` (refresh run timestamp/results if needed)

### Acceptance

- Build and tests pass.
- Existing 3-way parity report still generates.
- No unexplained behavior drift vs pre-rebase baseline.

---

## PR1: Config/Flags Skeleton (No Behavior Change)

### Goal

Introduce full config schema and CLI flags with resolved policy object, but keep runtime behavior unchanged.

### Scope

1. Add config model module and parser.
2. Add discovery/merge order:
   - built-in defaults
   - user config
   - upward-walk project configs (`.waft.toml`)
   - env vars
   - CLI flags
3. Add CLI flags from the schema doc.
4. Add a `ResolvedPolicy` struct passed through command entrypoints.
5. Keep selector/copy behavior effectively the same as pre-PR.

### Expected Files

- `src/config.rs` (new)
- `src/cli.rs`
- `src/main.rs` or command dispatch module
- `src/context.rs` (if carrying policy)
- `README.md` (flags docs stub)

### Tests

- `tests/config_parse_unit.rs` (new)
- `tests/config_discovery_integration.rs` (new)
- CLI help snapshot updates

### Acceptance

- Every new key/enum parses and validates.
- Precedence order proven by integration tests.
- No copy/list/info behavior change yet.

---

## PR2: Compat Presets + Policy Resolution

### Goal

Implement `--compat-profile` expansion into concrete knobs.

### Scope

- Implement preset resolver for `claude`, `git`, `wt`.
- Resolve conflicts deterministically: explicit knob > preset.
- Expose resolved policy in debug/info output (verbose only).

### Expected Files

- `src/config.rs`
- `src/model.rs` (policy enums/structs if needed)
- `src/subcommands/info.rs` (optional resolved-policy output)

### Tests

- Extend `tests/config_parse_unit.rs` for preset expansion.
- Add `tests/config_resolution_unit.rs` (new) for override precedence.

### Acceptance

- Preset mapping matches schema doc exactly.
- Overrides on top of presets are deterministic and tested.

---

## PR3: `when_missing_worktreeinclude` Behavior

### Goal

Ship first behavior knob: absent `.worktreeinclude` handling.

### Scope

- Add detection: “any relevant `.worktreeinclude` exists”.
- Implement two paths:
  - `blank`: no candidates selected when missing
  - `all-ignored`: all git-ignored untracked files selected when missing
- Keep all other behavior unchanged.

### Backend Notes (gix)

Prefer backend methods that avoid shelling out:

- `list_ignored_untracked(...)`
- `list_worktreeinclude_candidates(...)`
- `worktreeinclude_exists_anywhere(...)`

If `gix` support is incomplete, isolate temporary fallback logic behind backend trait methods, not command code.

### Expected Files

- backend trait + implementation (`src/git.rs` or migrated backend module)
- `src/subcommands/list.rs`
- `src/subcommands/copy.rs`

### Tests

- Add fixture tests for `F2` in:
  - `tests/modes_profile_integration.rs` (new)
  - `tests/modes_flag_override_integration.rs` (new)

### Acceptance

- `F2` outcomes match matrix for `claude|git|wt` profiles.
- Explicit flag overrides verified.

---

## PR4: Built-in and Extra Excludes

### Goal

Add exclusion policy layer independent of matcher semantics.

### Scope

- Implement `builtin_set = none|tooling-v1`.
- Implement `extra` excludes and `replace_extra` semantics.
- Apply exclusion filtering after selection and before plan execution.

### Expected Files

- `src/policy_filter.rs` (new) or equivalent
- `src/subcommands/list.rs`
- `src/subcommands/copy.rs`
- `src/planner.rs` (if filtering moved there)

### Tests

- `F7` profile checks (`claude` includes; `wt` excludes).
- Override checks:
  - `--builtin-exclude-set tooling-v1`
  - `--builtin-exclude-set none`
  - `--extra-exclude` and `--replace-extra-excludes`

### Acceptance

- Filter order is deterministic and documented.
- Tooling exclusions can be extended and overridden as specified.

---

## PR5: Symlinked `.worktreeinclude` Policy

### Goal

Replace hard-coded validation rejection with policy-driven handling.

### Scope

- Implement `symlink_policy`:
  - `follow`
  - `ignore`
  - `error`
- Ensure policy applies consistently to:
  - list candidate selection
  - copy behavior
  - validate reporting

### Expected Files

- `src/validate.rs`
- matcher/selector module(s)
- backend module(s) as needed

### Tests

- `F8` profile checks.
- Explicit override tests for all three policies.

### Acceptance

- `symlink_policy=error` fails deterministically.
- `ignore` and `follow` are behaviorally distinct and verified.

---

## PR6: Matcher Semantics Abstraction

### Goal

Introduce pluggable selection semantics with stable interfaces, then implement `git` semantics first.

### Scope

- Add `WorktreeincludeSemanticsEngine` abstraction.
- Route candidate selection through semantics engine rather than direct ad hoc calls.
- Implement `git` semantics engine using Git-equivalent per-directory behavior.

### Expected Files

- `src/worktreeinclude_engine.rs` (new)
- `src/worktreeinclude.rs` (refactor/split)
- `src/subcommands/list.rs`
- `src/subcommands/copy.rs`

### Tests

- `F1`, `F3`, `F4`, `F5`, `F6` under `git` profile.
- Preserve existing property tests where applicable.

### Acceptance

- `git` profile fully matches matrix expectations.
- No regression in info/explanation fidelity for git semantics.

---

## PR7: `claude-2026-04` Semantics Snapshot

### Goal

Implement observed Claude semantics snapshot as separate engine mode.

### Scope

- Add `claude-2026-04` semantics implementation.
- Keep implementation explicit and version-labeled (no hidden “latest” alias behavior).
- Add diagnostics in verbose mode showing active semantics mode.

### Tests

- `F1`–`F8` under `claude` profile in `tests/modes_profile_integration.rs`.
- Ensure pairwise alignment with parity harness expectations.

### Acceptance

- `claude` profile matches expected outputs from matrix exactly.

---

## PR8: `wt-0.39` Semantics Snapshot

### Goal

Implement observed worktrunk semantics snapshot.

### Scope

- Add `wt-0.39` semantics implementation.
- Keep versioned mode identifier to avoid drift from external `wt` upgrades.

### Tests

- `F1`–`F8` under `wt` profile.
- Include explicit assertions for known divergences (`F3`, `F4`, `F5`, `F7`).

### Acceptance

- `wt` profile matches matrix exactly.

---

## PR9: Full Fixture Suite + Docs + Default Flip

### Goal

Finalize test coverage, docs, and set default profile to `claude`.

### Scope

1. Add/complete fixture-driven suites:
   - `tests/modes_profile_integration.rs`
   - `tests/modes_flag_override_integration.rs`
   - `tests/config_discovery_integration.rs`
2. Update docs:
   - README behavior section
   - CLI help examples for modes
   - config reference docs
3. Flip default to `compat.profile = "claude"`.
4. Re-run 3-way parity report and include final summary.

### Acceptance

- All tests green.
- Matrix doc and implementation behavior align.
- Default OOTB behavior matches Claude profile.

---

## PR10 (Optional Hardening): External Parity CI Job

### Goal

Add non-blocking periodic parity check against local `wt` and `claude` tools for drift detection.

### Scope

- Add script that runs `just check-worktrunk-parity-3way` when binaries are available.
- Emit warning artifact/report on drift instead of failing required CI.

### Acceptance

- Drift detection exists without destabilizing required CI.

---

## Implementation Notes for gix Migration Compatibility

- Keep all repository-scanning and ignore classification behind backend trait boundaries.
- Avoid command-layer direct assumptions about backend internals.
- If any parity mode temporarily requires non-gix fallback behavior, encapsulate it in backend adapters and mark with TODOs tied to issue IDs.

## Definition of Done (Feature Epic)

- `claude` profile is default and matches fixture matrix expectations.
- `git` and `wt` profiles are selectable and verified.
- Flag-level overrides are deterministic and fully tested.
- Upward-walk project config discovery works with documented precedence.
- Parity harness and summary reports are reproducible via `just` commands.
