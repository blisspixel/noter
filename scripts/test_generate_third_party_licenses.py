"""Tests for deterministic third-party license rendering."""

from __future__ import annotations

import copy
import hashlib
import io
import json
import os
import stat
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

import generate_third_party_licenses as generator
from generate_third_party_licenses import (
    InventoryError,
    _load_bounded_json,
    _write_atomically,
    generate_inventory,
    render_inventory,
)


def package(name: str, version: str, repository: str | None = None) -> dict:
    return {"name": name, "version": version, "repository": repository}


def usage(name: str, version: str) -> dict:
    return {"crate": package(name, version), "path": "LICENSE"}


def sample_evidence() -> dict:
    return {
        "crates": [
            {
                "package": package("gamma", "3.0.0", "javascript:alert(1)"),
                "license": "MIT",
            },
            {
                "package": package("beta", "2.0.0"),
                "license": "Apache-2.0",
            },
            {
                "package": package(
                    "alpha", "1.0.0", "https://example.com/alpha?a=1&b=2"
                ),
                "license": "MIT",
            },
        ],
        "licenses": [
            {
                "id": "MIT",
                "name": "MIT License",
                "text": "MIT text\r\nwith <terms>\r\n",
                "source_path": "/volatile/path/one",
                "used_by": [usage("gamma", "3.0.0")],
            },
            {
                "id": "Apache-2.0",
                "name": "Apache License 2.0",
                "text": "Apache text\n",
                "source_path": "/volatile/path/two",
                "used_by": [usage("beta", "2.0.0")],
            },
            {
                "id": "MIT",
                "name": "MIT License",
                "text": "MIT text\nwith <terms>\n",
                "source_path": "C:\\volatile\\path\\three",
                "used_by": [usage("alpha", "1.0.0")],
            },
        ],
    }


class ThirdPartyLicenseGenerationTests(unittest.TestCase):
    def test_rendering_is_stable_across_input_order_and_line_endings(self) -> None:
        evidence = sample_evidence()
        shuffled = copy.deepcopy(evidence)
        shuffled["crates"].reverse()
        shuffled["licenses"].reverse()
        for license_record in shuffled["licenses"]:
            license_record["used_by"].reverse()

        expected = render_inventory(evidence)
        self.assertEqual(render_inventory(shuffled), expected)
        self.assertNotIn("\r", expected)
        self.assertLess(expected.index("alpha</a>"), expected.index(">beta</td>"))
        self.assertLess(expected.index(">beta</td>"), expected.index(">gamma</td>"))

    def test_equivalent_license_texts_are_merged_with_stable_mappings(self) -> None:
        rendered = render_inventory(sample_evidence())
        self.assertEqual(rendered.count("MIT text"), 1)
        self.assertEqual(rendered.count('class="license-user"'), 3)
        self.assertLess(rendered.index("alpha 1.0.0"), rendered.index("gamma 3.0.0"))

    def test_rendering_escapes_content_and_omits_unsafe_links(self) -> None:
        rendered = render_inventory(sample_evidence())
        self.assertIn('href="https://example.com/alpha?a=1&amp;b=2"', rendered)
        self.assertNotIn("javascript:", rendered)
        self.assertIn("with &lt;terms&gt;", rendered)

    def test_repository_links_reject_credentials_and_whitespace(self) -> None:
        evidence = sample_evidence()
        evidence["crates"][2]["package"]["repository"] = (
            "https://user@example.com/private"
        )
        evidence["crates"][1]["package"]["repository"] = "https://example.com/not valid"
        rendered = render_inventory(evidence)
        self.assertNotIn("user@example.com", rendered)
        self.assertNotIn("not valid", rendered)

    def test_atomic_replacement_preserves_existing_permissions(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "notices.html"
            output.write_text("old", encoding="utf-8")
            os.chmod(output, 0o640)
            expected_mode = stat.S_IMODE(output.stat().st_mode)

            _write_atomically(output, "new\n")

            self.assertEqual(output.read_bytes(), b"new\n")
            self.assertEqual(stat.S_IMODE(output.stat().st_mode), expected_mode)

    def test_atomic_replacement_rejects_a_directory_target(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "notices.html"
            output.mkdir()
            with self.assertRaisesRegex(InventoryError, "must be a regular file"):
                _write_atomically(output, "new\n")

    def test_rejects_duplicate_component_identity(self) -> None:
        evidence = sample_evidence()
        evidence["crates"].append(copy.deepcopy(evidence["crates"][0]))
        with self.assertRaisesRegex(InventoryError, "duplicate component identity"):
            render_inventory(evidence)

    def test_rejects_unknown_license_mapping(self) -> None:
        evidence = sample_evidence()
        evidence["licenses"][0]["used_by"].append(usage("unknown", "1.0.0"))
        with self.assertRaisesRegex(InventoryError, "unknown component"):
            render_inventory(evidence)

    def test_rejects_component_without_license_text(self) -> None:
        evidence = sample_evidence()
        evidence["licenses"][0]["used_by"].clear()
        with self.assertRaisesRegex(InventoryError, "has no package mappings"):
            render_inventory(evidence)

    def test_rejects_inconsistent_license_names(self) -> None:
        evidence = sample_evidence()
        evidence["licenses"][2]["name"] = "Different MIT name"
        with self.assertRaisesRegex(InventoryError, "inconsistent display names"):
            render_inventory(evidence)

    def test_rejects_malformed_shapes_and_bounded_text(self) -> None:
        with self.assertRaisesRegex(InventoryError, "must be an object"):
            render_inventory([])

        evidence = sample_evidence()
        evidence["crates"] = {}
        with self.assertRaisesRegex(InventoryError, "must be an array"):
            render_inventory(evidence)

        evidence = sample_evidence()
        evidence["crates"][0]["package"]["name"] = ""
        with self.assertRaisesRegex(InventoryError, "non-empty string"):
            render_inventory(evidence)

        evidence = sample_evidence()
        evidence["crates"][0]["package"]["name"] = "x" * 257
        with self.assertRaisesRegex(InventoryError, "256-byte limit"):
            render_inventory(evidence)

        evidence = sample_evidence()
        evidence["crates"][0]["package"]["name"] = "bad\x00name"
        with self.assertRaisesRegex(InventoryError, "null byte"):
            render_inventory(evidence)

    def test_rejects_item_and_total_mapping_limits(self) -> None:
        with patch.object(generator, "MAX_COMPONENTS", 1):
            with self.assertRaisesRegex(InventoryError, "1-item limit"):
                render_inventory(sample_evidence())

        with patch.object(generator, "MAX_MAPPINGS", 1):
            with self.assertRaisesRegex(InventoryError, "license mappings exceed"):
                render_inventory(sample_evidence())

    def test_rejects_unmapped_component(self) -> None:
        evidence = sample_evidence()
        evidence["licenses"] = [
            record for record in evidence["licenses"] if record["id"] != "Apache-2.0"
        ]
        with self.assertRaisesRegex(InventoryError, "components without license"):
            render_inventory(evidence)

    def test_rejects_malformed_repository_and_oversized_output(self) -> None:
        evidence = sample_evidence()
        evidence["crates"][2]["package"]["repository"] = "https://[invalid"
        self.assertNotIn("https://[invalid", render_inventory(evidence))

        with patch.object(generator, "MAX_OUTPUT_BYTES", 10):
            with self.assertRaisesRegex(InventoryError, "inventory exceeds"):
                render_inventory(sample_evidence())

    def test_bounded_json_loader_accepts_valid_and_rejects_invalid_input(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence_path = root / "evidence.json"
            evidence = sample_evidence()
            evidence_path.write_text(json.dumps(evidence), encoding="utf-8")
            self.assertEqual(_load_bounded_json(evidence_path), evidence)

            evidence_path.write_text("{", encoding="utf-8")
            with self.assertRaisesRegex(InventoryError, "invalid UTF-8 JSON"):
                _load_bounded_json(evidence_path)

            with self.assertRaisesRegex(InventoryError, "bounded regular file"):
                _load_bounded_json(root)

            evidence_path.write_bytes(b"{}")
            with (
                patch.object(generator, "MAX_JSON_BYTES", 1),
                self.assertRaisesRegex(InventoryError, "bounded regular file"),
            ):
                _load_bounded_json(evidence_path)

            with (
                patch.object(generator, "MAX_JSON_BYTES", 2),
                patch.object(Path, "read_bytes", return_value=b"{}x"),
                self.assertRaisesRegex(InventoryError, "exceeds the size limit"),
            ):
                _load_bounded_json(evidence_path)

    def test_atomic_replacement_rejects_a_non_directory_parent(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary) / "parent"
            parent.write_text("not a directory", encoding="utf-8")
            with self.assertRaisesRegex(InventoryError, "parent is not a directory"):
                _write_atomically(parent / "notices.html", "new\n")

    def test_atomic_replacement_cleans_up_after_replace_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "notices.html"
            with (
                patch.object(generator.os, "replace", side_effect=OSError("failure")),
                self.assertRaisesRegex(OSError, "failure"),
            ):
                _write_atomically(output, "new\n")
            self.assertEqual(list(Path(temporary).iterdir()), [])

    def test_generate_inventory_uses_fixed_cargo_about_contract(self) -> None:
        evidence = sample_evidence()

        def emit_evidence(command: list[str], **options) -> None:
            output_index = command.index("--output-file") + 1
            Path(command[output_index]).write_text(
                json.dumps(evidence), encoding="utf-8"
            )
            self.assertEqual(options["cwd"], generator.REPOSITORY_ROOT)
            self.assertTrue(options["check"])
            self.assertFalse(options["shell"])

        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "notices.html"
            with patch.object(generator.subprocess, "run", side_effect=emit_evidence):
                digest = generate_inventory(output)
            rendered = render_inventory(evidence)
            self.assertEqual(output.read_text(encoding="utf-8"), rendered)
            self.assertEqual(digest, hashlib.sha256(rendered.encode()).hexdigest())

    def test_main_reports_success_and_controlled_failure(self) -> None:
        output = Path("notices.html")
        stdout = io.StringIO()
        with (
            patch.object(sys, "argv", ["generator", "--output", str(output)]),
            patch.object(generator, "generate_inventory", return_value="a" * 64),
            redirect_stdout(stdout),
        ):
            generator.main()
        self.assertIn("SHA-256 " + "a" * 64, stdout.getvalue())

        with (
            patch.object(sys, "argv", ["generator", "--output", str(output)]),
            patch.object(
                generator,
                "generate_inventory",
                side_effect=InventoryError("bad evidence"),
            ),
            self.assertRaisesRegex(SystemExit, "bad evidence"),
        ):
            generator.main()


if __name__ == "__main__":
    unittest.main()
