import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import generate_m5_corpus as corpus


class EditorCorpusTests(unittest.TestCase):
    def test_smoke_inputs_are_reproducible_utf8_and_have_exact_search_offsets(self):
        with (
            tempfile.TemporaryDirectory() as first,
            tempfile.TemporaryDirectory() as second,
        ):
            manifest = corpus.generate_corpus(Path(first), "smoke")
            self.assertEqual(manifest, corpus.generate_corpus(Path(second), "smoke"))
            self.assertEqual(len(manifest["files"]), 7)
            self.assertEqual(
                json.loads(Path(first, "manifest.json").read_text(encoding="utf-8")),
                manifest,
            )
            for record in manifest["files"]:
                content = Path(first, record["name"]).read_bytes()
                self.assertEqual(len(content), 64 * 1024)
                self.assertEqual(hashlib.sha256(content).hexdigest(), record["sha256"])
                content.decode("utf-8")
                for anchor in record["search_anchors"]:
                    marker = manifest["search_markers"][anchor["location"]].encode()
                    self.assertEqual(content.count(marker), 1)
                    start = anchor["byte_offset"]
                    self.assertEqual(content[start : start + len(marker)], marker)
                if record["profile"] == "long-line":
                    self.assertNotIn(b"\n", content)
                    self.assertNotIn(b"\r", content)
                if record["profile"] == "mixed-eol":
                    self.assertIn(b"\r\nSecond line.\nThird line.\r", content)
                if record["profile"] == "short-lines":
                    self.assertLessEqual(max(map(len, content.splitlines())), 63)
                    self.assertGreater(len(content.splitlines()), 2000)
                if record["profile"] in (
                    "prose",
                    "short-lines",
                    "long-line",
                    "markdown",
                ):
                    content.decode("ascii")
                if record["profile"] == "unicode-edge":
                    self.assertIn("cafe\u0301".encode(), content)
                    self.assertIn("\U0001f469\u200d\U0001f4bb".encode(), content)
                if record["profile"] == "bidi-edge":
                    self.assertIn(
                        "\u2067\u05e9\u05dc\u05d5\u05dd 123\u2069".encode(), content
                    )

    def test_full_inputs_match_the_frozen_reference_manifest(self):
        reference = (
            Path(__file__).resolve().parents[1] / "docs/evidence/m5-corpus-v1.json"
        )
        with tempfile.TemporaryDirectory() as directory:
            manifest = corpus.generate_corpus(Path(directory), "full")
            self.assertEqual(
                manifest, json.loads(reference.read_text(encoding="utf-8"))
            )
            self.assertEqual(len(manifest["files"]), 21)
            self.assertEqual(
                {record["bytes"] for record in manifest["files"]},
                {corpus.MIB, 8 * corpus.MIB, 50 * corpus.MIB},
            )

    def test_existing_content_is_preserved_and_invalid_scale_writes_nothing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "not-created"
            with self.assertRaises(ValueError):
                corpus.generate_corpus(target, "invalid")
            self.assertFalse(target.exists())
            sentinel = root / "draft.txt"
            sentinel.write_bytes(b"preserve this draft")
            with self.assertRaises(FileExistsError):
                corpus.generate_corpus(root, "full")
            self.assertEqual(sentinel.read_bytes(), b"preserve this draft")
            self.assertEqual(list(root.iterdir()), [sentinel])

    def test_cli_defaults_to_smoke_and_reports_existing_directory_safely(self):
        script = Path(corpus.__file__).resolve()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "ordinary draft"
            command = [sys.executable, str(script), "--output", str(root)]
            result = subprocess.run(
                command, capture_output=True, text=True, check=False
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("7 files; corpus SHA-256", result.stdout)
            manifest_bytes = (root / "manifest.json").read_bytes()
            self.assertEqual(json.loads(manifest_bytes)["scale"], "smoke")
            retry = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(retry.returncode, 1)
            self.assertIn("corpus directory must be empty", retry.stderr)
            self.assertEqual((root / "manifest.json").read_bytes(), manifest_bytes)


if __name__ == "__main__":
    unittest.main()
