#!/usr/bin/env bash
# Exercise an extracted release binary in temporary Git worktrees.
set -euo pipefail

binary="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
expected_version="$2"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/waft-artifact.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_COUNT=0
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE WAFT_GIT_BACKEND

"$binary" --version | tee "$scratch/version"
test "$(cat "$scratch/version")" = "waft $expected_version"
"$binary" --help > "$scratch/help"
test -s "$scratch/help"
git init --quiet --initial-branch=main "$scratch/source"
git -C "$scratch/source" config user.name 'waft artifact check'
git -C "$scratch/source" config user.email 'artifact@waft.local'
printf '.env\n' > "$scratch/source/.gitignore"
printf '.env\n' > "$scratch/source/.worktreeinclude"
git -C "$scratch/source" add .gitignore .worktreeinclude
git -C "$scratch/source" -c commit.gpgsign=false commit --quiet -m fixture
git -C "$scratch/source" worktree add --quiet -b destination "$scratch/destination"
printf 'source secret\n' > "$scratch/source/.env"
printf 'keep unrelated\n' > "$scratch/destination/unrelated"
copy=("$binary" --isolated copy --source "$scratch/source" --dest "$scratch/destination")
"${copy[@]}"
cmp "$scratch/source/.env" "$scratch/destination/.env"
printf 'destination secret\n' > "$scratch/destination/.env"
"${copy[@]}"
test "$(cat "$scratch/destination/.env")" = 'destination secret'
"${copy[@]}" --overwrite
cmp "$scratch/source/.env" "$scratch/destination/.env"
printf 'tracked secret\n' > "$scratch/destination/.env"
git -C "$scratch/destination" add --force .env
"${copy[@]}" --overwrite
test "$(cat "$scratch/destination/.env")" = 'tracked secret'
test "$(cat "$scratch/destination/unrelated")" = 'keep unrelated'
printf 'Artifact smoke passed: %s / %s\n' "$(uname -s)" "$(uname -m)"
