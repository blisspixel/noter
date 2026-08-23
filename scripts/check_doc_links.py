#!/usr/bin/env python3
"""Fail when a Markdown inline link points to a missing local path or heading."""

from __future__ import annotations

import os
import re
import stat
import sys
from pathlib import Path
from urllib.parse import unquote, urlsplit


LINK = re.compile(r"!?\[[^\]]*\]\(([^)]+)\)")
HEADING = re.compile(r"^ {0,3}#{1,6}[ \t]+(.+?)[ \t]*#*[ \t]*$")
IGNORED_DIRECTORIES = {".agent", ".git", "logs", "target"}
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


def heading_slug(heading: str) -> str:
    """Return the GitHub-style fragment used by the repository's headings."""
    normalized = heading.casefold()
    supported = "".join(
        character
        for character in normalized
        if character.isalnum() or character in {" ", "\t", "-", "_"}
    )
    return re.sub(r"[ \t]+", "-", supported).strip("-")


def heading_anchors(content: str) -> set[str]:
    """Collect GitHub-style anchors, including deterministic duplicate suffixes."""
    anchors: set[str] = set()
    occurrences: dict[str, int] = {}
    in_fence = False
    for line in content.splitlines():
        stripped = line.lstrip()
        if stripped.startswith(("```", "~~~")):
            in_fence = not in_fence
            continue
        if in_fence:
            continue

        match = HEADING.match(line)
        if match is None:
            continue
        base = heading_slug(match.group(1))
        if not base:
            continue
        duplicate = occurrences.get(base, 0)
        occurrences[base] = duplicate + 1
        anchors.add(base if duplicate == 0 else f"{base}-{duplicate}")
    return anchors


def normalized_path(path: Path) -> Path:
    """Normalize lexical path segments without resolving a possible symlink."""
    return Path(os.path.abspath(path))


def first_symlink_component(root: Path, destination: Path) -> Path | None:
    """Return the first symlink below a lexical repository root, if any."""

    current = root
    for component in destination.relative_to(root).parts:
        current /= component
        if stat.S_ISLNK(current.lstat().st_mode):
            return current
    return None


def terminal_safe_diagnostic(diagnostic: str) -> str:
    """Render non-printing Unicode code points as visible terminal-safe escapes."""

    rendered: list[str] = []
    for character in diagnostic:
        if character.isprintable():
            rendered.append(character)
            continue

        code_point = ord(character)
        if code_point <= 0xFF:
            rendered.append(f"\\x{code_point:02x}")
        elif code_point <= 0xFFFF:
            rendered.append(f"\\u{code_point:04x}")
        else:
            rendered.append(f"\\U{code_point:08x}")
    return "".join(rendered)


def missing_links(root: Path) -> list[str]:
    """Return diagnostics for missing local inline-link paths and fragments."""
    diagnostics: list[str] = []
    repository_root = normalized_path(root)
    resolved_repository_root = repository_root.resolve(strict=True)
    contents: dict[Path, str] = {}

    for document in markdown_files(root):
        relative = document.relative_to(root)
        try:
            content = read_markdown(document)
        except (OSError, ValueError) as error:
            diagnostics.append(f"{relative.as_posix()}: {error}")
            continue
        contents[normalized_path(document)] = content

    anchors = {
        document: heading_anchors(content) for document, content in contents.items()
    }

    for document_path, content in contents.items():
        document = Path(document_path)
        relative = document.relative_to(repository_root).as_posix()
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
                if parsed.scheme or target.startswith("//"):
                    continue

                decoded_path = unquote(parsed.path)
                if not decoded_path:
                    destination = document
                elif decoded_path.startswith("/"):
                    destination = root / decoded_path.lstrip("/")
                else:
                    destination = document.parent / decoded_path
                destination = normalized_path(destination)

                try:
                    destination.relative_to(repository_root)
                except ValueError:
                    diagnostics.append(
                        f"{relative}:{line_number}: local link leaves repository: {target}"
                    )
                    continue

                if not destination.exists():
                    diagnostics.append(f"{relative}:{line_number}: missing {target}")
                    continue

                try:
                    resolved_destination = destination.resolve(strict=True)
                    resolved_destination.relative_to(resolved_repository_root)
                except ValueError:
                    diagnostics.append(
                        f"{relative}:{line_number}: local link leaves repository: {target}"
                    )
                    continue
                except (OSError, RuntimeError) as error:
                    diagnostics.append(
                        f"{relative}:{line_number}: local link cannot be resolved: "
                        f"{target}: {error}"
                    )
                    continue

                # Repository links must behave identically on hosts where Git
                # checks symlinks out as links or as plain target-text files.
                # Reject every symlink component, including links that happen to
                # resolve inside the repository, instead of validating one host's
                # checkout representation and publishing a different result.
                try:
                    symlink = first_symlink_component(repository_root, destination)
                except OSError as error:
                    diagnostics.append(
                        f"{relative}:{line_number}: local link cannot be inspected: "
                        f"{target}: {error}"
                    )
                    continue
                if symlink is not None:
                    diagnostics.append(
                        f"{relative}:{line_number}: local link uses a symbolic path: {target}"
                    )
                    continue

                fragment = unquote(parsed.fragment)
                if (
                    fragment
                    and destination.suffix.casefold() == ".md"
                    and destination in anchors
                    and fragment not in anchors[destination]
                ):
                    diagnostics.append(
                        f"{relative}:{line_number}: missing heading #{fragment} in "
                        f"{destination.relative_to(repository_root).as_posix()}"
                    )

    return diagnostics


def main() -> int:
    """Check the repository containing this script."""
    root = Path(__file__).resolve().parent.parent
    diagnostics = missing_links(root)
    if diagnostics:
        print(
            "\n".join(terminal_safe_diagnostic(item) for item in diagnostics),
            file=sys.stderr,
        )
        return 1

    print("All local Markdown link targets exist.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
