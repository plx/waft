#!/usr/bin/env bash
# self-test — exercise waft and the post-checkout hook end-to-end against
# a throwaway git repo. Suitable for `just check-self-test` and CI.
#
# Inputs (env):
#   WAFT          path to the waft binary (default: target/release/waft)
#   KEEP_TMP      if set non-empty, leave the scratch repo for inspection

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WAFT="${WAFT:-$REPO_ROOT/target/release/waft}"

if [ ! -x "$WAFT" ]; then
  echo "self-test: waft binary not found or not executable: $WAFT" >&2
  echo "  build it first: cargo build --release" >&2
  exit 1
fi

TMP="$(mktemp -d "${TMPDIR:-/tmp}/waft-self-test.XXXXXX")"
cleanup() {
  if [ -n "${KEEP_TMP:-}" ]; then
    echo "self-test: leaving scratch dir at $TMP" >&2
  else
    rm -rf "$TMP"
  fi
}
trap cleanup EXIT

PASS=0
FAIL=0
section() { printf '\n=== %s ===\n' "$1"; }
ok()      { PASS=$((PASS+1)); printf '  ok    %s\n' "$1"; }
fail()    { FAIL=$((FAIL+1)); printf '  FAIL  %s\n' "$1" >&2; }

assert_file_exists() {
  if [ -f "$1" ]; then ok "exists: ${1#"$TMP"/}"; else fail "missing: ${1#"$TMP"/}"; fi
}

assert_file_absent() {
  if [ ! -e "$1" ]; then ok "absent: ${1#"$TMP"/}"; else fail "unexpected: ${1#"$TMP"/}"; fi
}

assert_file_content() {
  local path="$1" expect="$2"
  if [ -f "$path" ] && [ "$(cat "$path")" = "$expect" ]; then
    ok "content: ${path#"$TMP"/}"
  else
    fail "content mismatch: ${path#"$TMP"/} (expected '$expect', got '$(cat "$path" 2>/dev/null || echo MISSING)')"
  fi
}

# -----------------------------------------------------------------------
# Build a throwaway repo that mirrors the conventions of this project:
# .gitignore covers .env*, the .worktreeinclude opts those files in.
# -----------------------------------------------------------------------
MAIN="$TMP/main"
mkdir -p "$MAIN"
cd "$MAIN"

git init --quiet --initial-branch=main
git config user.email "self-test@waft.local"
git config user.name  "waft self-test"
git config commit.gpgsign false

cat >.gitignore <<'EOF'
.env
.env.*
.envrc
local-secret
build-cache/
EOF

cat >.worktreeinclude <<'EOF'
.env
.env.*
.envrc
EOF

echo "tracked content" >tracked.txt

git add .gitignore .worktreeinclude tracked.txt
git commit --quiet -m "initial commit"

# Fixture untracked-but-ignored files in the main worktree.
echo "ENV_VALUE_FROM_MAIN" >.env
echo "LOCAL_OVERRIDE"       >.env.local
echo "DIRENV_CONFIG"        >.envrc
echo "should-not-copy"      >local-secret
mkdir -p build-cache
echo "stale build artifact" >build-cache/output.bin

# Sanity: every fixture is git-ignored as expected.
for f in .env .env.local .envrc local-secret build-cache/output.bin; do
  git check-ignore -q "$f" || { echo "self-test: expected $f to be git-ignored" >&2; exit 1; }
done

# -----------------------------------------------------------------------
# Test 1 — waft copy from a fresh linked worktree.
# -----------------------------------------------------------------------
section "Test 1: waft copy into a freshly-added linked worktree"

LINKED1="$TMP/linked-explicit"
git worktree add --quiet -b self-test/explicit "$LINKED1"

(cd "$LINKED1" && "$WAFT" --quiet)

assert_file_content "$LINKED1/.env"       "ENV_VALUE_FROM_MAIN"
assert_file_content "$LINKED1/.env.local" "LOCAL_OVERRIDE"
assert_file_content "$LINKED1/.envrc"     "DIRENV_CONFIG"
assert_file_absent  "$LINKED1/local-secret"
assert_file_absent  "$LINKED1/build-cache"

# Tracked content must remain untouched.
assert_file_content "$LINKED1/tracked.txt" "tracked content"

# -----------------------------------------------------------------------
# Test 2 — post-checkout hook fires automatically on `git worktree add`.
# -----------------------------------------------------------------------
section "Test 2: post-checkout hook runs waft on git worktree add"

# Wire the project's hook directory into this scratch repo and point the
# hook at our just-built binary so it doesn't depend on PATH.
git config core.hooksPath "$REPO_ROOT/hooks"

LINKED2="$TMP/linked-hook"
WAFT="$WAFT" git worktree add -b self-test/hook "$LINKED2" >/dev/null

assert_file_content "$LINKED2/.env"       "ENV_VALUE_FROM_MAIN"
assert_file_content "$LINKED2/.env.local" "LOCAL_OVERRIDE"
assert_file_content "$LINKED2/.envrc"     "DIRENV_CONFIG"
assert_file_absent  "$LINKED2/local-secret"
assert_file_absent  "$LINKED2/build-cache"

# -----------------------------------------------------------------------
# Test 3 — running waft in the main worktree is a no-op-ish error and
# does NOT mutate fixture files.
# -----------------------------------------------------------------------
section "Test 3: waft refuses to copy from main worktree without --dest"

set +e
(cd "$MAIN" && "$WAFT" --quiet) 2>/dev/null
status=$?
set -e

if [ "$status" -ne 0 ]; then
  ok "waft errored as expected when run from main worktree (exit $status)"
else
  fail "waft from main worktree should require --dest"
fi
assert_file_content "$MAIN/.env" "ENV_VALUE_FROM_MAIN"

# -----------------------------------------------------------------------
# Test 4 — overwrite safety: waft must not clobber an existing untracked
# destination file unless --overwrite is passed.
# -----------------------------------------------------------------------
section "Test 4: overwrite safety"

LINKED3="$TMP/linked-overwrite"
# Add the worktree without firing the hook so we can stage a conflict.
git -c core.hooksPath=/dev/null worktree add --quiet -b self-test/overwrite "$LINKED3"
echo "PRE_EXISTING_LOCAL" >"$LINKED3/.env"

# Without --overwrite, waft must leave the existing file alone (it may
# exit 0 with a skip; what matters is the content is preserved).
(cd "$LINKED3" && "$WAFT" --quiet) || true
assert_file_content "$LINKED3/.env" "PRE_EXISTING_LOCAL"

# With --overwrite, waft replaces the existing file from the source.
(cd "$LINKED3" && "$WAFT" copy --overwrite --quiet)
assert_file_content "$LINKED3/.env" "ENV_VALUE_FROM_MAIN"

# -----------------------------------------------------------------------
# Summary
# -----------------------------------------------------------------------
printf '\n%d passed, %d failed\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ]
