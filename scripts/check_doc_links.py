#!/usr/bin/env python3
"""Fail when a Markdown inline link points to a missing local path."""

from __future__ import annotations

import re
import os
import stat
import sys
from pathlib import Path
from urllib.parse import unquote, urlsplit


LINK = re.compile(r"!?\[[^\]]*\]\(([^)]+)\)")
IGNORED_DIRECTORIES = {".git", "target"}
MAX_MARKDOWN_BYTES = 2 * 1024 * 1024


def target_from(raw: str) -> str:
    """Remove an optional Markdown title or angle brackets from a target."""
    value = raw.strip()
    if value.startswith("<") and ">" in value:
        return value[1:].split(">", 1)[0]
    return value.split(maxsplit=1)[0]


def markdown_files(root: Path) -> list[Path]:
    """Return repository Markdown paths while ignoring generated directories."""
    documents: list[Path] = []
    for path in sorted(root.rglob("*.md")):
        relative = path.relative_to(root)
        if IGNORED_DIRECTORIES.intersection(relative.parts):
            continue
        documents.append(path)
    return documents


def read_markdown(document: Path) -> str:
    """Read one bounded regular Markdown file without accepting a path swap."""
    initial = document.lstat()
    if stat.S_ISLNK(initial.st_mode):
        raise ValueError("symbolic Markdown links are not allowed")
    if not stat.S_ISREG(initial.st_mode):
        raise ValueError("Markdown input is not a regular file")
    if initial.st_size > MAX_MARKDOWN_BYTES:
        raise ValueError(f"Markdown input exceeds the {MAX_MARKDOWN_BYTES}-byte limit")

    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_CLOEXEC", 0)
    flags |= getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(document, flags)
    try:
        opened = os.fstat(descriptor)
        if not stat.S_ISREG(opened.st_mode):
            raise ValueError("Markdown input is not a regular file")
        if not os.path.samestat(initial, opened):
            raise ValueError("Markdown input changed while it was opened")
        if opened.st_size > MAX_MARKDOWN_BYTES:
            raise ValueError(
                f"Markdown input exceeds the {MAX_MARKDOWN_BYTES}-byte limit"
            )
        with os.fdopen(descriptor, "rb", closefd=False) as stream:
            content = stream.read(MAX_MARKDOWN_BYTES + 1)
        if len(content) > MAX_MARKDOWN_BYTES:
            raise ValueError(
                f"Markdown input exceeds the {MAX_MARKDOWN_BYTES}-byte limit"
            )
    finally:
        os.close(descriptor)

    try:
        return content.decode("utf-8")
    except UnicodeDecodeError as error:
        raise ValueError("Markdown input is not valid UTF-8") from error


def missing_links(root: Path) -> list[str]:
    """Return diagnostics for missing local inline-link targets."""
    diagnostics: list[str] = []

    for document in markdown_files(root):
        relative = document.relative_to(root)
        try:
            content = read_markdown(document)
        except (OSError, ValueError) as error:
            diagnostics.append(f"{relative}: {error}")
            continue
        in_fence = False
        for line_number, line in enumerate(content.splitlines(), start=1):
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
