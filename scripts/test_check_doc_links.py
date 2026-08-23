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
    def create_symlink_or_skip(
        self, target: Path, link: Path, *, target_is_directory: bool = False
    ) -> None:
        """Create a native symlink or skip when the host forbids the fixture."""

        try:
            os.symlink(target, link, target_is_directory=target_is_directory)
        except OSError as error:
            self.skipTest(f"symbolic links are unavailable: {error}")

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

    def test_rejects_a_file_symlink_target_outside_the_repository(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "repository"
            root.mkdir()
            outside = root.parent / "outside.txt"
            outside.write_text("outside\n", encoding="utf-8")
            self.create_symlink_or_skip(outside, root / "linked.txt")
            (root / "README.md").write_text("[outside](linked.txt)\n", encoding="utf-8")

            self.assertEqual(
                missing_links(root),
                ["README.md:1: local link leaves repository: linked.txt"],
            )

    def test_rejects_a_directory_symlink_target_outside_the_repository(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "repository"
            root.mkdir()
            outside = root.parent / "outside"
            outside.mkdir()
            (outside / "guide.txt").write_text("outside\n", encoding="utf-8")
            self.create_symlink_or_skip(
                outside, root / "docs", target_is_directory=True
            )
            (root / "README.md").write_text(
                "[outside](docs/guide.txt)\n", encoding="utf-8"
            )

            self.assertEqual(
                missing_links(root),
                ["README.md:1: local link leaves repository: docs/guide.txt"],
            )

    def test_rejects_an_in_repository_symlink_target_consistently(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "target.txt"
            target.write_text("inside\n", encoding="utf-8")
            self.create_symlink_or_skip(target, root / "linked.txt")
            (root / "README.md").write_text("[inside](linked.txt)\n", encoding="utf-8")

            self.assertEqual(
                missing_links(root),
                ["README.md:1: local link uses a symbolic path: linked.txt"],
            )

    def test_checks_local_heading_fragments(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "README.md").write_text(
                "# Project\n\n[valid](docs/guide.md#quick-start)\n"
                "[missing](docs/guide.md#removed-section)\n",
                encoding="utf-8",
            )
            docs = root / "docs"
            docs.mkdir()
            (docs / "guide.md").write_text(
                "# Guide\n\n## Quick Start\n", encoding="utf-8"
            )

            self.assertEqual(
                missing_links(root),
                ["README.md:4: missing heading #removed-section in docs/guide.md"],
            )

    def test_checks_same_document_and_duplicate_heading_fragments(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "README.md").write_text(
                "# Project\n\n"
                "## Repeated Name\n\n"
                "## Repeated Name\n\n"
                "[first](#repeated-name)\n"
                "[second](#repeated-name-1)\n",
                encoding="utf-8",
            )

            self.assertEqual(missing_links(root), [])

    def test_rejects_a_local_link_outside_the_repository(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "repository"
            root.mkdir()
            outside = root.parent / "outside.md"
            outside.write_text("# Outside\n", encoding="utf-8")
            (root / "README.md").write_text(
                "[outside](../outside.md)\n", encoding="utf-8"
            )

            self.assertEqual(
                missing_links(root),
                ["README.md:1: local link leaves repository: ../outside.md"],
            )

    def test_ignores_local_working_and_generated_directories(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "README.md").write_text("# Project\n", encoding="utf-8")
            for ignored in (".agent", "logs", "target"):
                ignored_directory = root / ignored
                ignored_directory.mkdir()
                (ignored_directory / "notes.md").write_text(
                    "[missing](missing.md)\n", encoding="utf-8"
                )

            self.assertEqual(missing_links(root), [])


if __name__ == "__main__":
    unittest.main()
