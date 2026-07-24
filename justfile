build-release:
    cargo build --release --locked

# Install a reviewed snapshot of waft and its hook under the repository's
# common Git directory. The installer chains the previously effective hooks
# and refuses to chain hooks sourced from a worktree.
install-hooks: build-release
    WAFT="{{justfile_directory()}}/target/release/waft" bash "{{justfile_directory()}}/scripts/install-hooks.sh"

# Restore the hooks configuration that preceded `just install-hooks`.
uninstall-hooks:
    bash "{{justfile_directory()}}/scripts/install-hooks.sh" --uninstall

# End-to-end self-test: builds the release binary, then drives a scratch
# git repo through `waft copy` and the post-checkout hook.
check-self-test: build-release
    bash scripts/self-test.sh

check-format:
    cargo fmt --check

check-clippy:
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

check-test:
    cargo test --workspace --all-features --locked

check-doc-test:
    cargo test --doc --locked

check-doc-build:
    cargo doc --no-deps --locked

check-audit:
    cargo audit --deny warnings

# Measure production code through external test targets so inline #[cfg(test)]
# modules cannot inflate the result.
check-coverage:
    #!/usr/bin/env bash
    set -euo pipefail
    coverage_args=()
    for coverage_file in tests/*.rs; do
      coverage_args+=(--test "$(basename "${coverage_file}" .rs)")
    done
    cargo llvm-cov --workspace --all-features --locked "${coverage_args[@]}" \
      --ignore-filename-regex '(^|/)(tests|benches)/' \
      --fail-under-lines 70 \
      --fail-under-regions 70 \
      --fail-under-functions 65

# Regenerate THIRD_PARTY_LICENSES.md from the current Cargo.lock.
# Run this whenever you add, remove, or update a dependency.
regen-licenses:
    cargo about generate -c about.toml -o THIRD_PARTY_LICENSES.md about.hbs

# Fail if THIRD_PARTY_LICENSES.md is out of date relative to Cargo.lock.
# Mirrors the `licenses` CI job; run locally to debug drift.
check-licenses:
    #!/usr/bin/env bash
    set -euo pipefail
    tmp="$(mktemp)"
    trap 'rm -f "$tmp"' EXIT
    cargo about generate -c about.toml -o "$tmp" about.hbs
    if ! diff -u THIRD_PARTY_LICENSES.md "$tmp"; then
      echo "" >&2
      echo "THIRD_PARTY_LICENSES.md is out of date." >&2
      echo "Run 'just regen-licenses' and commit the result." >&2
      exit 1
    fi

bench-scaling:
    cargo bench --bench scaling --locked

check-worktrunk-parity:
    cargo test --test worktrunk_parity --locked -- --ignored --nocapture

check-claude-worktree-smoke:
    @set -euo pipefail; \
    output="$(claude --worktree --model haiku -p --permission-mode bypassPermissions --no-session-persistence --output-format text "State the full path of your current working directory (CWD).")"; \
    wt_path="$(printf '%s\n' "$output" | grep -Eo '/[^[:space:]]+' | head -n1)"; \
    if [ -z "$wt_path" ]; then \
      echo "failed to parse Claude worktree path" >&2; \
      echo "${output}" >&2; \
      exit 1; \
    fi; \
    echo "claude-created worktree: ${wt_path}"; \
    git worktree remove --force "${wt_path}"

check-worktrunk-parity-3way:
    just check-claude-worktree-smoke
    cargo test --test worktrunk_parity --locked -- --ignored --nocapture
