"""Tests for bounded repository Markdown link validation."""

from __future__ import annotations

import os
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from check_doc_links import MAX_MARKDOWN_BYTES, missing_links


class MarkdownInputTests(unittest.TestCase):
    def test_checks_regular_markdown_files(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "README.md").write_text(
                "[missing](docs/missing.md)\n", encoding="utf-8"
            )

            self.assertEqual(
                missing_links(root),
                ["README.md:1: missing docs/missing.md"],
            )

    def test_rejects_oversized_markdown_before_reading_it(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            document = root / "oversized.md"
            with document.open("wb") as stream:
                stream.truncate(MAX_MARKDOWN_BYTES + 1)

            with patch.object(
                Path, "read_text", side_effect=AssertionError("must not read")
            ):
                diagnostics = missing_links(root)

            self.assertEqual(len(diagnostics), 1)
            self.assertIn("exceeds", diagnostics[0])

    def test_rejects_a_path_replaced_between_inspection_and_open(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            document = root / "README.md"
            replacement = root / "replacement.txt"
            document.write_text("original", encoding="utf-8")
            replacement.write_text("replacement", encoding="utf-8")
            real_open = os.open
            replaced = False

            def replace_then_open(path: Path, flags: int) -> int:
                nonlocal replaced
                if not replaced:
                    replaced = True
                    shutil.copyfile(replacement, root / "incoming.txt")
                    os.replace(root / "incoming.txt", document)
                return real_open(path, flags)

            with patch("check_doc_links.os.open", side_effect=replace_then_open):
                diagnostics = missing_links(root)

            self.assertEqual(
                diagnostics,
                ["README.md: Markdown input changed while it was opened"],
            )

    def test_rejects_invalid_utf8_without_a_traceback(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "README.md").write_bytes(b"\xff")

            self.assertEqual(
                missing_links(root),
                ["README.md: Markdown input is not valid UTF-8"],
            )

    def test_rejects_symbolic_markdown_links(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "target.txt"
            target.write_text("content", encoding="utf-8")
            link = root / "linked.md"
            try:
                os.symlink(target, link)
            except OSError as error:
                self.skipTest(f"symbolic links are unavailable: {error}")

            diagnostics = missing_links(root)

            self.assertEqual(
                diagnostics,
                ["linked.md: symbolic Markdown links are not allowed"],
            )


if __name__ == "__main__":
    unittest.main()
