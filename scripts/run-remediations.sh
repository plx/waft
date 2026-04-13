#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PENDING_DIR="$REPO_ROOT/remediations/pending"

# Collect pending items
items=("$PENDING_DIR"/*.md)

if [[ ${#items[@]} -eq 0 || ! -e "${items[0]}" ]]; then
  echo "No pending remediation items found in $PENDING_DIR"
  exit 0
fi

echo "Found ${#items[@]} pending remediation item(s):"
for f in "${items[@]}"; do
  echo "  - $(basename "$f")"
done
echo ""

for item in "${items[@]}"; do
  name="$(basename "$item")"
  echo "=========================================="
  echo "Processing: $name"
  echo "=========================================="

  # Run one headless Claude session per item.
  # --dangerously-skip-permissions: required for headless mode to auto-approve
  #   file writes, edits, and bash commands without interactive prompts.
  claude -p "/handle-remediation-item $item" \
    --dangerously-skip-permissions

  echo ""
  echo "Finished: $name"
  echo ""
done

echo "All remediation items processed."
