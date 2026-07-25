#!/usr/bin/env python3
"""Fail when a Markdown inline link points to a missing local path."""

from __future__ import annotations

import re
import sys
from pathlib import Path
from urllib.parse import unquote, urlsplit


LINK = re.compile(r"!?\[[^\]]*\]\(([^)]+)\)")
IGNORED_DIRECTORIES = {".git", "target"}


def target_from(raw: str) -> str:
    """Remove an optional Markdown title or angle brackets from a target."""
    value = raw.strip()
    if value.startswith("<") and ">" in value:
        return value[1:].split(">", 1)[0]
    return value.split(maxsplit=1)[0]


def markdown_files(root: Path) -> list[Path]:
    """Return repository Markdown files while ignoring generated directories."""
    return [
        path
        for path in sorted(root.rglob("*.md"))
        if not IGNORED_DIRECTORIES.intersection(path.relative_to(root).parts)
    ]


def missing_links(root: Path) -> list[str]:
    """Return diagnostics for missing local inline-link targets."""
    diagnostics: list[str] = []

    for document in markdown_files(root):
        in_fence = False
        for line_number, line in enumerate(
            document.read_text(encoding="utf-8").splitlines(), start=1
        ):
            stripped = line.lstrip()
            if stripped.startswith(("```", "~~~")):
                in_fence = not in_fence
                continue
            if in_fence:
                continue

            for match in LINK.finditer(line):
                target = target_from(match.group(1))
                parsed = urlsplit(target)
                if parsed.scheme or target.startswith(("#", "//")):
                    continue

                decoded_path = unquote(parsed.path)
                if not decoded_path:
                    continue

                if decoded_path.startswith("/"):
                    destination = root / decoded_path.lstrip("/")
                else:
                    destination = document.parent / decoded_path

                if not destination.exists():
                    relative = document.relative_to(root)
                    diagnostics.append(f"{relative}:{line_number}: missing {target}")

    return diagnostics


def main() -> int:
    """Check the repository containing this script."""
    root = Path(__file__).resolve().parent.parent
    diagnostics = missing_links(root)
    if diagnostics:
        print("\n".join(diagnostics), file=sys.stderr)
        return 1

    print("All local Markdown link targets exist.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
