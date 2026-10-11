#!/usr/bin/env python3
"""Freeze editor-gate inputs without making performance or native UI claims."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from run_m1_baseline import (
    MIB,
    SEARCH_MARKERS,
    corpus_manifest_digest,
    write_repeated_file,
)

BLOCK_BYTES = 1024
PROFILES = {
    "prose": "Noter keeps an ordinary English draft private and portable.\n",
    "short-lines": "One.\nTwo.\nThree.\n" * 16,
    "long-line": "An English line with no hard break. ",
    "mixed-eol": "First line.\r\nSecond line.\nThird line.\r",
    "unicode-edge": "Unicode boundaries: cafe\u0301 | \u6771\u4eac | \U0001f642 | \U0001f469\u200d\U0001f4bb\n",
    "bidi-edge": "Direction boundaries: left \u2067\u05e9\u05dc\u05d5\u05dd 123\u2069 right | \u0645\u0631\u062d\u0628\u0627\n",
    "markdown": (
        "# An ordinary draft\n\nA **strong** claim and *careful* wording.\n\n"
        "- A local file\n- An explicit save\n\n"
        "```text\nKeep source bytes.\n```\n\n"
        "[Local reference](notes.md)\n\n"
    ),
}
SCOPE = (
    "Reproducible editor-gate inputs only. No latency, memory, accessibility, "
    "IME, renderer acceptance, or supported file-size claim."
)


def profile_block(profile: str) -> bytes:
    """Keep text and terminators intact across every repeated write chunk."""
    text = PROFILES[profile].encode("utf-8")
    # The second half is ASCII text reserved for unique search anchors.
    if len(text) > BLOCK_BYTES // 2:
        raise ValueError("profile text overlaps reserved search-anchor padding")
    if profile == "long-line":
        return (text * (BLOCK_BYTES // len(text) + 1))[:BLOCK_BYTES]
    line = b"An ordinary English draft stays local, clear, and useful."
    line = line.ljust(63, b" ") + b"\n"
    prefix_padding = (line * 8)[: BLOCK_BYTES // 2 - len(text)]
    # End both halves at a hard break, including a truncated filler line.
    prefix = (text + prefix_padding)[:-1] + b"\n"
    return prefix + line * 8


def generate_corpus(directory: Path, scale: str) -> dict[str, object]:
    """Write exact-size UTF-8 inputs exclusively into an empty directory."""
    if scale not in ("smoke", "full"):
        raise ValueError("scale must be smoke or full")
    sizes = (64 * 1024,) if scale == "smoke" else (MIB, 8 * MIB, 50 * MIB)
    directory.mkdir(parents=True, exist_ok=True)
    if any(directory.iterdir()):
        raise FileExistsError("corpus directory must be empty")
    records = []
    for profile in PROFILES:
        for size in sizes:
            name = f"{profile}-{size // 1024}kib.txt"
            path = directory / name
            block = profile_block(profile)
            write_repeated_file(path, block, size)
            positions = (BLOCK_BYTES // 2, size // 2 + BLOCK_BYTES // 2, size - 512)
            anchors = []
            with path.open("r+b") as output:
                for (location, marker), offset in zip(
                    SEARCH_MARKERS.items(), positions, strict=True
                ):
                    encoded = marker.encode("ascii")
                    output.seek(offset)
                    expected = block[BLOCK_BYTES // 2 : BLOCK_BYTES // 2 + len(encoded)]
                    if output.read(len(encoded)) != expected:
                        raise ValueError("search anchor would change profile text")
                    output.seek(offset)
                    output.write(encoded)
                    anchors.append({"location": location, "byte_offset": offset})
            digest = hashlib.sha256()
            with path.open("rb") as source:
                for chunk in iter(lambda: source.read(MIB), b""):
                    digest.update(chunk)
            records.append(
                {
                    "name": name,
                    "profile": profile,
                    "bytes": size,
                    "sha256": digest.hexdigest(),
                    "search_anchors": anchors,
                }
            )
    manifest = {
        "generator_version": 1,
        "scale": scale,
        "scope": SCOPE,
        "search_markers": dict(SEARCH_MARKERS),
        "files": records,
        "corpus_sha256": corpus_manifest_digest(records),
    }
    with (directory / "manifest.json").open("x", encoding="utf-8", newline="\n") as out:
        out.write(json.dumps(manifest, indent=2, ensure_ascii=True) + "\n")
    return manifest


def main(arguments: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--scale", choices=("smoke", "full"), default="smoke")
    args = parser.parse_args(arguments)
    try:
        manifest = generate_corpus(args.output, args.scale)
    except (OSError, ValueError) as error:
        parser.exit(1, f"Corpus generation failed: {ascii(str(error))}\n")
    print(f"{len(manifest['files'])} files; corpus SHA-256 {manifest['corpus_sha256']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
