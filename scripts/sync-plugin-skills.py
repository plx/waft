#!/usr/bin/env python3
"""Mirror waft's reference skill, preserving each harness's frontmatter."""

import argparse
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SKILL = "copying-gitignored-files-with-waft"
SOURCE = ROOT / "plugins" / "waft-codex" / "skills" / SKILL
DEST = ROOT / "plugins" / "waft-claude-code" / "skills" / SKILL


def split_skill(path):
    text = path.read_text(encoding="utf-8")
    if not text.startswith("---\n"):
        raise ValueError(f"{path}: missing frontmatter")
    header, separator, body = text[4:].partition("\n---\n")
    if not separator:
        raise ValueError(f"{path}: unterminated frontmatter")
    return "---\n" + header + separator, body


def reference_files(root):
    return {
        path.relative_to(root): path.read_bytes()
        for path in (root / "references").rglob("*")
        if path.is_file()
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail on drift; write nothing")
    args = parser.parse_args()

    _, source_body = split_skill(SOURCE / "SKILL.md")
    dest_header, _ = split_skill(DEST / "SKILL.md")
    expected = reference_files(SOURCE)
    if not source_body.strip() or not expected:
        raise ValueError("canonical skill body and references must not be empty")
    actual = reference_files(DEST)
    expected[Path("SKILL.md")] = (dest_header + source_body).encode("utf-8")
    actual[Path("SKILL.md")] = (DEST / "SKILL.md").read_bytes()

    changed = sorted(path for path, data in expected.items() if actual.get(path) != data)
    stale = sorted(actual.keys() - expected.keys())
    if args.check:
        for path in changed + stale:
            print(f"skill drift: {(DEST / path).relative_to(ROOT)}", file=sys.stderr)
        if changed or stale:
            print("Run 'just sync-plugin-skills' after editing the Codex copy.", file=sys.stderr)
            return 1
        print("Plugin skill bodies and references match.")
        return 0

    for path in changed:
        target = DEST / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(expected[path])
    for path in stale:
        (DEST / path).unlink()
    print(f"Synced plugin skill: {len(changed)} updated, {len(stale)} stale references removed.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError) as error:
        sys.exit(str(error))
