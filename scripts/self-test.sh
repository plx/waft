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

# Keep developer and CI machine Git configuration from changing the fixture's
# hook path, ignore behavior, or signing behavior.
export GIT_CONFIG_NOSYSTEM=1
export GIT_CONFIG_GLOBAL=/dev/null

if [ ! -x "$WAFT" ]; then
  echo "self-test: waft binary not found or not executable: $WAFT" >&2
  echo "  build it first: cargo build --release" >&2
  exit 1
fi

TMP="$(mktemp -d "${TMPDIR:-/tmp}/waft-self-test.XXXXXX")"
mkdir -p "$TMP/home" "$TMP/xdg-config"
export HOME="$TMP/home"
export USERPROFILE="$TMP/home"
export XDG_CONFIG_HOME="$TMP/xdg-config"
unset \
  WAFT_CONFIG_PATH \
  WAFT_COMPAT_PROFILE \
  WAFT_WHEN_MISSING_WORKTREEINCLUDE \
  WAFT_WORKTREEINCLUDE_SEMANTICS \
  WAFT_WORKTREEINCLUDE_SYMLINK_POLICY \
  WAFT_BUILTIN_EXCLUDE_SET \
  WAFT_EXTRA_EXCLUDE \
  WAFT_REPLACE_EXTRA_EXCLUDES \
  WAFT_COPY_STRATEGY \
  WAFT_GIT_BACKEND || true

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

# This executable is deliberately tracked to verify that a branch-controlled
# hooks directory is never used after the trusted installer runs.
mkdir -p hooks
cat >hooks/post-checkout <<'EOF'
#!/bin/sh
if [ -n "${WAFT_MALICIOUS_LOG:-}" ]; then
  printf 'branch-controlled hook ran\n' >>"${WAFT_MALICIOUS_LOG}"
fi
EOF
chmod +x hooks/post-checkout

git add .gitignore .worktreeinclude tracked.txt hooks/post-checkout
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

# Put a pre-existing, trusted hook in Git metadata. The waft installer must
# chain it without making branch-controlled hook content executable.
COMMON_HOOKS="$(git rev-parse --git-path hooks)"
mkdir -p "$COMMON_HOOKS"
cat >"$COMMON_HOOKS/post-checkout" <<'EOF'
#!/bin/sh
if [ -n "${WAFT_CHAIN_LOG:-}" ]; then
  printf 'existing hook ran\n' >>"${WAFT_CHAIN_LOG}"
fi
EOF
chmod +x "$COMMON_HOOKS/post-checkout"

WAFT="$WAFT" bash "$REPO_ROOT/scripts/install-hooks.sh"

INSTALLED_HOOKS="$(git config --local --get core.hooksPath)"
case "$INSTALLED_HOOKS" in
  /*) ok "core.hooksPath is absolute" ;;
  *) fail "core.hooksPath is not absolute: $INSTALLED_HOOKS" ;;
esac
case "$INSTALLED_HOOKS/" in
  "$MAIN/"*) fail "installed hooks are inside the checked-out worktree" ;;
  *) ok "installed hooks are outside the checked-out worktree" ;;
esac

# Reinstall through a relative alias of the same managed directory. This must
# refresh the installation without chaining the dispatcher back to itself.
ORIGINAL_CHAIN_DIR="$(cd "$COMMON_HOOKS" && pwd -P)"
git config --local core.hooksPath .git/waft-hooks
WAFT="$WAFT" bash "$REPO_ROOT/scripts/install-hooks.sh"
if [ "$(git config --local --get core.hooksPath)" = "$INSTALLED_HOOKS" ]; then
  ok "managed hook path aliases are normalized"
else
  fail "managed hook path alias was not normalized"
fi
if [ "$(cat "$INSTALLED_HOOKS/.prior-hooks-dir")" = "$ORIGINAL_CHAIN_DIR" ]; then
  ok "managed hook alias does not create a recursive chain"
else
  fail "managed hook alias changed the prior hook chain"
fi

CHAIN_LOG="$TMP/chained-hook.log"
MALICIOUS_LOG="$TMP/branch-controlled-hook.log"
HOSTILE_WAFT_LOG="$TMP/hostile-waft.log"
HOSTILE_WAFT="$TMP/hostile-waft"
cat >"$HOSTILE_WAFT" <<'EOF'
#!/bin/sh
printf 'ambient WAFT ran\n' >>"${WAFT_HOSTILE_LOG}"
exit 99
EOF
chmod +x "$HOSTILE_WAFT"

LINKED2="$TMP/linked-hook"
WAFT_CHAIN_LOG="$CHAIN_LOG" WAFT_MALICIOUS_LOG="$MALICIOUS_LOG" \
  WAFT_HOSTILE_LOG="$HOSTILE_WAFT_LOG" WAFT="$HOSTILE_WAFT" \
  WAFT_EXTRA_EXCLUDE=".env" \
  git worktree add -b self-test/hook "$LINKED2" >/dev/null

assert_file_content "$LINKED2/.env"       "ENV_VALUE_FROM_MAIN"
assert_file_content "$LINKED2/.env.local" "LOCAL_OVERRIDE"
assert_file_content "$LINKED2/.envrc"     "DIRENV_CONFIG"
assert_file_absent  "$LINKED2/local-secret"
assert_file_absent  "$LINKED2/build-cache"
assert_file_content "$CHAIN_LOG" "existing hook ran"
assert_file_absent "$MALICIOUS_LOG"
assert_file_absent "$HOSTILE_WAFT_LOG"

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
# Test 4 — overwrite safety: waft never clobbers an existing pathname.
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

# --overwrite remains accepted for CLI compatibility, but fails closed when it
# would replace an existing file.
if (cd "$LINKED3" && "$WAFT" copy --overwrite --quiet >/dev/null 2>&1); then
  fail "--overwrite unexpectedly replaced an existing destination"
fi
assert_file_content "$LINKED3/.env" "PRE_EXISTING_LOCAL"

# -----------------------------------------------------------------------
# Test 5 — uninstall restores the original Git hook lookup behavior.
# -----------------------------------------------------------------------
section "Test 5: hook installer is reversible"

bash "$REPO_ROOT/scripts/install-hooks.sh" --uninstall
if git config --local --get core.hooksPath >/dev/null 2>&1; then
  fail "hook uninstall left a local core.hooksPath override"
else
  ok "hook uninstall restored the default hooks path"
fi
if [ -x "$COMMON_HOOKS/post-checkout" ]; then
  ok "pre-existing post-checkout hook remains in place"
else
  fail "pre-existing post-checkout hook was removed"
fi

# -----------------------------------------------------------------------
# Test 6 — a hook path in any sibling worktree is branch-controlled.
# -----------------------------------------------------------------------
section "Test 6: hook installer rejects sibling-worktree hook paths"

mkdir -p "$LINKED1/sibling-hooks"
cp "$REPO_ROOT/hooks/dispatcher" "$LINKED1/sibling-hooks/post-checkout"
chmod +x "$LINKED1/sibling-hooks/post-checkout"
git -C "$LINKED1" add sibling-hooks/post-checkout
git -C "$LINKED1" commit --quiet -m "add sibling-controlled hook"

git config --local core.hooksPath ../linked-explicit/sibling-hooks
if WAFT="$WAFT" bash "$REPO_ROOT/scripts/install-hooks.sh"; then
  fail "installer accepted a hook path in a sibling worktree"
else
  ok "installer rejected a hook path in a sibling worktree"
fi
if [ "$(git config --local --get core.hooksPath)" = "../linked-explicit/sibling-hooks" ]; then
  ok "rejected sibling hook path left Git configuration unchanged"
else
  fail "rejected sibling hook path changed Git configuration"
fi
git config --local --unset-all core.hooksPath

# -----------------------------------------------------------------------
# Test 7 — the managed directory itself must stay inside Git metadata.
# -----------------------------------------------------------------------
section "Test 7: hook installer rejects a symlinked managed directory"

COMMON_DIR="$(cd "$(git rev-parse --git-common-dir)" && pwd -P)"
MANAGED_DIR="$COMMON_DIR/waft-hooks"
SYMLINK_TARGET="$LINKED1/managed-hooks"
mkdir -p "$SYMLINK_TARGET"
ln -s "$SYMLINK_TARGET" "$MANAGED_DIR"

if WAFT="$WAFT" bash "$REPO_ROOT/scripts/install-hooks.sh"; then
  fail "installer accepted a symlinked managed hook directory"
else
  ok "installer rejected a symlinked managed hook directory"
fi
assert_file_absent "$SYMLINK_TARGET/post-checkout"
rm "$MANAGED_DIR"

# -----------------------------------------------------------------------
# Test 8 — an unmarked reserved directory belongs to the user, not waft.
# -----------------------------------------------------------------------
section "Test 8: hook installer preserves an unmarked reserved directory"

mkdir -p "$MANAGED_DIR"
echo "corporate hook" >"$MANAGED_DIR/post-checkout"
git config --local core.hooksPath "$MANAGED_DIR"
if WAFT="$WAFT" bash "$REPO_ROOT/scripts/install-hooks.sh"; then
  fail "installer took over an unmarked reserved hook directory"
else
  ok "installer rejected an unmarked reserved hook directory"
fi
assert_file_content "$MANAGED_DIR/post-checkout" "corporate hook"
git config --local --unset-all core.hooksPath
rm -rf "$MANAGED_DIR"

# -----------------------------------------------------------------------
# Test 9 — a higher-precedence worktree setting cannot be silently bypassed.
# -----------------------------------------------------------------------
section "Test 9: hook installer rejects worktree-scoped hook overrides"

WORKTREE_HOOKS="$TMP/worktree-scoped-hooks"
mkdir -p "$WORKTREE_HOOKS"
git config extensions.worktreeConfig true
git config --worktree core.hooksPath "$WORKTREE_HOOKS"
if WAFT="$WAFT" bash "$REPO_ROOT/scripts/install-hooks.sh"; then
  fail "installer accepted a worktree-scoped hook override"
else
  ok "installer rejected a worktree-scoped hook override"
fi
if git config --local --get core.hooksPath >/dev/null 2>&1; then
  fail "rejected worktree override left a local hooksPath"
else
  ok "rejected worktree override left local configuration unchanged"
fi
git config --worktree --unset-all core.hooksPath

# Repeat from a sibling: checking only the invoking worktree would miss this
# higher-precedence bypass.
git -C "$LINKED1" config --worktree core.hooksPath "$WORKTREE_HOOKS"
if WAFT="$WAFT" bash "$REPO_ROOT/scripts/install-hooks.sh"; then
  fail "installer accepted a sibling worktree-scoped hook override"
else
  ok "installer rejected a sibling worktree-scoped hook override"
fi
if git config --local --get core.hooksPath >/dev/null 2>&1; then
  fail "rejected sibling worktree override left a local hooksPath"
else
  ok "rejected sibling override left local configuration unchanged"
fi
git -C "$LINKED1" config --worktree --unset-all core.hooksPath

# -----------------------------------------------------------------------
# Test 10 — prior hook symlinks can cross back into branch-controlled data.
# -----------------------------------------------------------------------
section "Test 10: hook installer rejects symlinked prior hooks"

EXTERNAL_HOOKS="$TMP/external-hooks"
mkdir -p "$EXTERNAL_HOOKS"
ln -s "$LINKED1/sibling-hooks/post-checkout" "$EXTERNAL_HOOKS/post-checkout"
git config --local core.hooksPath "$EXTERNAL_HOOKS"
if WAFT="$WAFT" bash "$REPO_ROOT/scripts/install-hooks.sh"; then
  fail "installer accepted a symlinked prior hook"
else
  ok "installer rejected a symlinked prior hook"
fi
if [ "$(git config --local --get core.hooksPath)" = "$EXTERNAL_HOOKS" ]; then
  ok "rejected prior hook symlink left Git configuration unchanged"
else
  fail "rejected prior hook symlink changed Git configuration"
fi
git config --local --unset-all core.hooksPath

# -----------------------------------------------------------------------
# Summary
# -----------------------------------------------------------------------
printf '\n%d passed, %d failed\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ]
