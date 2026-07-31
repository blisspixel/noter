"""Tests for deterministic third-party license rendering."""

from __future__ import annotations

import copy
import hashlib
import io
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

import generate_third_party_licenses as generator
from generate_third_party_licenses import (
    InventoryError,
    _collect_packaged_notices,
    _load_bounded_json,
    _write_atomically,
    generate_inventory,
    render_inventory,
)


def package(name: str, version: str, repository: str | None = None) -> dict:
    return {"name": name, "version": version, "repository": repository}


def usage(name: str, version: str) -> dict:
    return {"crate": package(name, version), "path": "LICENSE"}


def notice(name: str, version: str, path: str, text: str) -> dict:
    return {
        "package": package(name, version),
        "path": path,
        "text": text,
    }


def create_junction(link: Path, target: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["cmd.exe", "/d", "/c", "mklink", "/J", str(link), str(target)],
        check=False,
        capture_output=True,
        text=True,
    )


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


def sample_notices() -> list[dict]:
    return [
        notice("gamma", "3.0.0", "LICENSE", "MIT text\r\nwith <terms>\r\n"),
        notice("beta", "2.0.0", "COPYING", "Apache text\n"),
        notice("alpha", "1.0.0", "LICENSE-MIT", "MIT text\nwith <terms>\n"),
    ]


class ThirdPartyLicenseGenerationTests(unittest.TestCase):
    def test_rendering_is_stable_across_input_order_and_line_endings(self) -> None:
        evidence = sample_evidence()
        shuffled = copy.deepcopy(evidence)
        shuffled["crates"].reverse()
        shuffled["licenses"].reverse()
        for license_record in shuffled["licenses"]:
            license_record["used_by"].reverse()

        expected = render_inventory(evidence, sample_notices())
        self.assertEqual(
            render_inventory(shuffled, list(reversed(sample_notices()))), expected
        )
        self.assertNotIn("\r", expected)
        self.assertLess(expected.index("alpha</a>"), expected.index(">beta</td>"))
        self.assertLess(expected.index(">beta</td>"), expected.index(">gamma</td>"))

    def test_host_sensitive_about_selection_does_not_change_notice_union(self) -> None:
        windows_evidence = sample_evidence()
        linux_evidence = copy.deepcopy(windows_evidence)
        alternate_text = "Alternate MIT text\nwith a preserved notice\n"
        for record in linux_evidence["licenses"]:
            if record["id"] == "MIT":
                record["text"] = alternate_text
        notices = sample_notices() + [
            notice(
                "gamma",
                "3.0.0",
                "src/backend/LICENSE",
                alternate_text,
            )
        ]

        self.assertEqual(
            render_inventory(windows_evidence, notices),
            render_inventory(linux_evidence, list(reversed(notices))),
        )

    def test_equivalent_license_texts_are_merged_with_stable_mappings(self) -> None:
        rendered = render_inventory(sample_evidence(), sample_notices())
        self.assertEqual(rendered.count("MIT text"), 1)
        self.assertEqual(rendered.count('class="notice-source"'), 3)
        self.assertLess(rendered.index("alpha 1.0.0:"), rendered.index("gamma 3.0.0:"))

    def test_rendering_escapes_content_and_omits_unsafe_links(self) -> None:
        rendered = render_inventory(sample_evidence(), sample_notices())
        self.assertIn('href="https://example.com/alpha?a=1&amp;b=2"', rendered)
        self.assertNotIn("javascript:", rendered)
        self.assertIn("with &lt;terms&gt;", rendered)

    def test_repository_links_reject_credentials_and_whitespace(self) -> None:
        evidence = sample_evidence()
        evidence["crates"][2]["package"]["repository"] = (
            "https://user@example.com/private"
        )
        evidence["crates"][1]["package"]["repository"] = "https://example.com/not valid"
        rendered = render_inventory(evidence, sample_notices())
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
            render_inventory(evidence, sample_notices())

    def test_rejects_unknown_license_mapping(self) -> None:
        evidence = sample_evidence()
        evidence["licenses"][0]["used_by"].append(usage("unknown", "1.0.0"))
        with self.assertRaisesRegex(InventoryError, "unknown component"):
            render_inventory(evidence, sample_notices())

    def test_rejects_component_without_license_text(self) -> None:
        evidence = sample_evidence()
        evidence["licenses"][0]["used_by"].clear()
        with self.assertRaisesRegex(InventoryError, "has no package mappings"):
            render_inventory(evidence, sample_notices())

    def test_rejects_inconsistent_license_names(self) -> None:
        evidence = sample_evidence()
        evidence["licenses"][2]["name"] = "Different MIT name"
        with self.assertRaisesRegex(InventoryError, "inconsistent display names"):
            render_inventory(evidence, sample_notices())

    def test_rejects_malformed_shapes_and_bounded_text(self) -> None:
        with self.assertRaisesRegex(InventoryError, "must be an object"):
            render_inventory([], sample_notices())

        evidence = sample_evidence()
        evidence["crates"] = {}
        with self.assertRaisesRegex(InventoryError, "must be an array"):
            render_inventory(evidence, sample_notices())

        evidence = sample_evidence()
        evidence["crates"][0]["package"]["name"] = ""
        with self.assertRaisesRegex(InventoryError, "non-empty string"):
            render_inventory(evidence, sample_notices())

        evidence = sample_evidence()
        evidence["crates"][0]["package"]["name"] = "x" * 257
        with self.assertRaisesRegex(InventoryError, "256-byte limit"):
            render_inventory(evidence, sample_notices())

        evidence = sample_evidence()
        evidence["crates"][0]["package"]["name"] = "bad\x00name"
        with self.assertRaisesRegex(InventoryError, "null byte"):
            render_inventory(evidence, sample_notices())

    def test_rejects_item_and_total_mapping_limits(self) -> None:
        with patch.object(generator, "MAX_COMPONENTS", 1):
            with self.assertRaisesRegex(InventoryError, "1-item limit"):
                render_inventory(sample_evidence(), sample_notices())

        with patch.object(generator, "MAX_MAPPINGS", 1):
            with self.assertRaisesRegex(InventoryError, "license mappings exceed"):
                render_inventory(sample_evidence(), sample_notices())

    def test_rejects_unmapped_component(self) -> None:
        evidence = sample_evidence()
        evidence["licenses"] = [
            record for record in evidence["licenses"] if record["id"] != "Apache-2.0"
        ]
        with self.assertRaisesRegex(InventoryError, "components without license"):
            render_inventory(evidence, sample_notices())

    def test_rejects_malformed_repository_and_oversized_output(self) -> None:
        evidence = sample_evidence()
        evidence["crates"][2]["package"]["repository"] = "https://[invalid"
        self.assertNotIn(
            "https://[invalid", render_inventory(evidence, sample_notices())
        )

        with patch.object(generator, "MAX_OUTPUT_BYTES", 10):
            with self.assertRaisesRegex(InventoryError, "inventory exceeds"):
                render_inventory(sample_evidence(), sample_notices())

    def test_rejects_unknown_and_noncanonical_packaged_notice_paths(self) -> None:
        unknown = sample_notices()
        unknown[0]["package"]["name"] = "unknown"
        with self.assertRaisesRegex(InventoryError, "unknown component"):
            render_inventory(sample_evidence(), unknown)

        traversal = sample_notices()
        traversal[0]["path"] = "../LICENSE"
        with self.assertRaisesRegex(InventoryError, "canonical and relative"):
            render_inventory(sample_evidence(), traversal)

        with self.assertRaisesRegex(InventoryError, "must not be empty"):
            render_inventory(sample_evidence(), [])

    def test_collects_bounded_packaged_legal_files_without_host_paths(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            package_root = Path(temporary) / "alpha-1.0.0"
            nested_root = package_root / "src" / "backend"
            legal_root = package_root / "LICENSES"
            nested_root.mkdir(parents=True)
            legal_root.mkdir()
            manifest = package_root / "Cargo.toml"
            manifest.write_text("[package]\nname='alpha'\n", encoding="utf-8")
            (package_root / "LICENSE-MIT").write_bytes(b"\xef\xbb\xbfMIT\r\n")
            (package_root / "EULA.txt").write_bytes(b"Explicit terms\n")
            (package_root / "THIRDPARTYNOTICES").write_bytes(b"Third-party terms\n")
            (nested_root / "NOTICE.md").write_bytes(b"Nested notice\r")
            (nested_root / "emoji-icon-font-mit-license.txt").write_bytes(
                b"Nested font license\n"
            )
            (nested_root / "Hack-Regular.ttf").write_bytes(b"synthetic font")
            (nested_root / "Hack-Regular.txt").write_bytes(b"Font terms\n")
            (nested_root / "OFL.txt").write_bytes(b"Open font terms\n")
            (nested_root / "UFL.txt").write_bytes(b"Ubuntu font terms\n")
            (nested_root / "copying.rs").write_text(
                "pub trait Copying {}\n", encoding="utf-8"
            )
            (legal_root / "MIT.txt").write_bytes(b"Directory notice\n")
            (package_root / "README.md").write_text("ignored", encoding="utf-8")
            test_root = package_root / "tests"
            test_root.mkdir()
            (test_root / "COPYRIGHT").write_text(
                "Test fixture attribution\n", encoding="utf-8"
            )
            evidence = sample_evidence()
            alpha = evidence["crates"][2]["package"]
            alpha["source"] = "registry+https://github.com/rust-lang/crates.io-index"
            alpha["manifest_path"] = str(manifest)
            alpha["license_file"] = "EULA.txt"

            collected = _collect_packaged_notices(evidence)

            self.assertEqual(
                {(item["path"], item["text"]) for item in collected},
                {
                    ("LICENSE-MIT", "MIT\n"),
                    ("LICENSES/MIT.txt", "Directory notice\n"),
                    ("EULA.txt", "Explicit terms\n"),
                    ("THIRDPARTYNOTICES", "Third-party terms\n"),
                    ("src/backend/NOTICE.md", "Nested notice\n"),
                    (
                        "src/backend/emoji-icon-font-mit-license.txt",
                        "Nested font license\n",
                    ),
                    ("src/backend/Hack-Regular.txt", "Font terms\n"),
                    ("src/backend/OFL.txt", "Open font terms\n"),
                    ("src/backend/UFL.txt", "Ubuntu font terms\n"),
                },
            )
            self.assertNotIn(str(package_root), json.dumps(collected))

    def test_rejects_legal_file_replaced_between_check_and_open(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            container = Path(temporary)
            package_root = container / "alpha-1.0.0"
            package_root.mkdir()
            manifest = package_root / "Cargo.toml"
            manifest.write_text("[package]\nname='alpha'\n", encoding="utf-8")
            legal_file = package_root / "LICENSE"
            legal_file.write_text("Expected terms\n", encoding="utf-8")
            outside = container / "outside.txt"
            outside.write_text("Synthetic private data\n", encoding="utf-8")
            evidence = sample_evidence()
            alpha = evidence["crates"][2]["package"]
            alpha["source"] = "registry+https://github.com/rust-lang/crates.io-index"
            alpha["manifest_path"] = str(manifest)
            original_open = os.open
            swapped = False

            def swap_checked_file() -> None:
                nonlocal swapped
                if swapped:
                    return
                legal_file.unlink()
                os.link(outside, legal_file)
                swapped = True

            def racing_open(
                path: os.PathLike[str] | str,
                flags: int,
                mode: int = 0o777,
                *,
                dir_fd: int | None = None,
            ) -> int:
                if Path(path) == legal_file:
                    swap_checked_file()
                return original_open(path, flags, mode, dir_fd=dir_fd)

            with (
                patch.object(os, "open", racing_open),
                self.assertRaisesRegex(
                    InventoryError, "changed while it was being opened"
                ),
            ):
                _collect_packaged_notices(evidence)

            self.assertTrue(swapped)

    def test_rejects_legal_file_mutated_in_place_between_check_and_open(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            package_root = Path(temporary) / "alpha-1.0.0"
            package_root.mkdir()
            manifest = package_root / "Cargo.toml"
            manifest.write_text("[package]\nname='alpha'\n", encoding="utf-8")
            legal_file = package_root / "LICENSE"
            legal_file.write_text("Expected terms\n", encoding="utf-8")
            evidence = sample_evidence()
            alpha = evidence["crates"][2]["package"]
            alpha["source"] = "registry+https://github.com/rust-lang/crates.io-index"
            alpha["manifest_path"] = str(manifest)
            original_open = os.open
            mutated = False

            def racing_open(
                path: os.PathLike[str] | str,
                flags: int,
                mode: int = 0o777,
                *,
                dir_fd: int | None = None,
            ) -> int:
                nonlocal mutated
                if Path(path) == legal_file and not mutated:
                    legal_file.write_text("Replaced terms\n", encoding="utf-8")
                    metadata = legal_file.stat()
                    os.utime(
                        legal_file,
                        ns=(metadata.st_atime_ns, metadata.st_mtime_ns + 1_000_000_000),
                    )
                    mutated = True
                return original_open(path, flags, mode, dir_fd=dir_fd)

            with (
                patch.object(os, "open", racing_open),
                self.assertRaisesRegex(
                    InventoryError, "changed while it was being opened"
                ),
            ):
                _collect_packaged_notices(evidence)

            self.assertTrue(mutated)

    def test_rejects_legal_file_mutated_after_the_descriptor_read(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            package_root = Path(temporary) / "alpha-1.0.0"
            package_root.mkdir()
            manifest = package_root / "Cargo.toml"
            manifest.write_text("[package]\nname='alpha'\n", encoding="utf-8")
            legal_file = package_root / "LICENSE"
            legal_file.write_text("Expected terms\n", encoding="utf-8")
            evidence = sample_evidence()
            alpha = evidence["crates"][2]["package"]
            alpha["source"] = "registry+https://github.com/rust-lang/crates.io-index"
            alpha["manifest_path"] = str(manifest)
            original_lstat = Path.lstat
            legal_lstat_count = 0
            mutated = False

            def racing_lstat(path: Path) -> os.stat_result:
                nonlocal legal_lstat_count, mutated
                if path == legal_file:
                    legal_lstat_count += 1
                    if legal_lstat_count == 2:
                        legal_file.write_text("Replaced terms\n", encoding="utf-8")
                        metadata = original_lstat(legal_file)
                        os.utime(
                            legal_file,
                            ns=(
                                metadata.st_atime_ns,
                                metadata.st_mtime_ns + 1_000_000_000,
                            ),
                        )
                        mutated = True
                return original_lstat(path)

            with (
                patch.object(Path, "lstat", racing_lstat),
                self.assertRaisesRegex(
                    InventoryError, "changed while it was being read"
                ),
            ):
                _collect_packaged_notices(evidence)

            self.assertTrue(mutated)

    @unittest.skipUnless(os.name == "nt", "NTFS junction semantics require Windows")
    def test_rejects_a_junction_that_escapes_the_package_root(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            container = Path(temporary)
            package_root = container / "alpha-1.0.0"
            package_root.mkdir()
            manifest = package_root / "Cargo.toml"
            manifest.write_text("[package]\nname='alpha'\n", encoding="utf-8")
            outside = container / "outside"
            outside.mkdir()
            (outside / "LICENSE").write_text(
                "Synthetic outside data\n", encoding="utf-8"
            )
            junction = package_root / "LICENSES"
            result = create_junction(junction, outside)
            self.assertEqual(
                result.returncode,
                0,
                f"junction creation failed: {result.stderr.strip()}",
            )
            try:
                self.assertTrue(generator._is_link_like(junction))
                evidence = sample_evidence()
                alpha = evidence["crates"][2]["package"]
                alpha["source"] = (
                    "registry+https://github.com/rust-lang/crates.io-index"
                )
                alpha["manifest_path"] = str(manifest)

                with self.assertRaisesRegex(
                    InventoryError, "link-like directory|outside its package"
                ):
                    _collect_packaged_notices(evidence)
            finally:
                junction.rmdir()

    @unittest.skipUnless(os.name == "nt", "NTFS junction semantics require Windows")
    def test_rejects_a_directory_replaced_by_a_junction_during_resolution(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            container = Path(temporary)
            package_root = container / "alpha-1.0.0"
            legal_root = package_root / "LEGAL"
            legal_root.mkdir(parents=True)
            manifest = package_root / "Cargo.toml"
            manifest.write_text("[package]\nname='alpha'\n", encoding="utf-8")
            (legal_root / "LICENSE").write_text("Expected terms\n", encoding="utf-8")
            outside = container / "outside"
            outside.mkdir()
            (outside / "LICENSE").write_text(
                "Synthetic outside data\n", encoding="utf-8"
            )
            probe = container / "junction-probe"
            probe_result = create_junction(probe, outside)
            self.assertEqual(
                probe_result.returncode,
                0,
                f"junction creation failed: {probe_result.stderr.strip()}",
            )
            probe.rmdir()
            saved_legal_root = package_root / "LEGAL.saved"
            evidence = sample_evidence()
            alpha = evidence["crates"][2]["package"]
            alpha["source"] = "registry+https://github.com/rust-lang/crates.io-index"
            alpha["manifest_path"] = str(manifest)
            original_resolve = Path.resolve
            replaced = False

            def racing_resolve(path: Path, strict: bool = False) -> Path:
                nonlocal replaced
                result = original_resolve(path, strict=strict)
                if path == legal_root and not replaced:
                    legal_root.rename(saved_legal_root)
                    junction_result = create_junction(legal_root, outside)
                    if junction_result.returncode != 0:
                        raise RuntimeError(junction_result.stderr.strip())
                    replaced = True
                return result

            try:
                with (
                    patch.object(Path, "resolve", racing_resolve),
                    self.assertRaisesRegex(
                        InventoryError, "link-like directory|changed during traversal"
                    ),
                ):
                    _collect_packaged_notices(evidence)
            finally:
                if generator._is_link_like(legal_root):
                    legal_root.rmdir()
                if saved_legal_root.exists():
                    saved_legal_root.rename(legal_root)

            self.assertTrue(replaced)

    def test_rejects_a_hard_link_to_a_file_outside_the_package_root(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            container = Path(temporary)
            package_root = container / "alpha-1.0.0"
            package_root.mkdir()
            manifest = package_root / "Cargo.toml"
            manifest.write_text("[package]\nname='alpha'\n", encoding="utf-8")
            outside = container / "outside.txt"
            outside.write_text("Synthetic outside data\n", encoding="utf-8")
            os.link(outside, package_root / "LICENSE")
            evidence = sample_evidence()
            alpha = evidence["crates"][2]["package"]
            alpha["source"] = "registry+https://github.com/rust-lang/crates.io-index"
            alpha["manifest_path"] = str(manifest)

            with self.assertRaisesRegex(InventoryError, "must not be hard-linked"):
                _collect_packaged_notices(evidence)

    def test_rejects_unsafe_packaged_legal_file_inputs(self) -> None:
        evidence = sample_evidence()
        alpha = evidence["crates"][2]["package"]
        alpha["source"] = "registry+https://github.com/rust-lang/crates.io-index"
        alpha["manifest_path"] = "Cargo.toml"
        with self.assertRaisesRegex(InventoryError, "absolute regular file"):
            _collect_packaged_notices(evidence)

        with tempfile.TemporaryDirectory() as temporary:
            package_root = Path(temporary)
            manifest = package_root / "Cargo.toml"
            manifest.write_text("[package]\nname='alpha'\n", encoding="utf-8")
            (package_root / "LICENSE").write_bytes(b"\xff")
            alpha["manifest_path"] = str(manifest)
            with self.assertRaisesRegex(InventoryError, "is not UTF-8"):
                _collect_packaged_notices(evidence)

            (package_root / "LICENSE").write_bytes(b"ab")
            with (
                patch.object(generator, "MAX_LICENSE_TEXT_BYTES", 1),
                self.assertRaisesRegex(InventoryError, "per-file limit"),
            ):
                _collect_packaged_notices(evidence)

            with (
                patch.object(generator, "MAX_NOTICE_INPUT_BYTES", 1),
                self.assertRaisesRegex(InventoryError, "total byte limit"),
            ):
                _collect_packaged_notices(evidence)

            with (
                patch.object(generator, "MAX_SOURCE_ENTRIES", 1),
                self.assertRaisesRegex(InventoryError, "filesystem-entry limit"),
            ):
                _collect_packaged_notices(evidence)

    def test_explicit_license_file_cannot_escape_its_package(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            container = Path(temporary)
            package_root = container / "alpha-1.0.0"
            package_root.mkdir()
            manifest = package_root / "Cargo.toml"
            manifest.write_text("[package]\nname='alpha'\n", encoding="utf-8")
            (package_root / "LICENSE").write_text("MIT\n", encoding="utf-8")
            (container / "outside.txt").write_text("private\n", encoding="utf-8")
            evidence = sample_evidence()
            alpha = evidence["crates"][2]["package"]
            alpha["source"] = "registry+https://github.com/rust-lang/crates.io-index"
            alpha["manifest_path"] = str(manifest)
            alpha["license_file"] = "../outside.txt"

            with self.assertRaisesRegex(InventoryError, "remain inside its package"):
                _collect_packaged_notices(evidence)

    def test_requires_at_least_one_packaged_third_party_legal_file(self) -> None:
        with self.assertRaisesRegex(InventoryError, "no packaged third-party"):
            _collect_packaged_notices(sample_evidence())

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
                patch.object(os, "read", return_value=b"{}x"),
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
            with (
                patch.object(generator.subprocess, "run", side_effect=emit_evidence),
                patch.object(
                    generator,
                    "_collect_packaged_notices",
                    return_value=sample_notices(),
                ),
            ):
                digest = generate_inventory(output)
            rendered = render_inventory(evidence, sample_notices())
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
