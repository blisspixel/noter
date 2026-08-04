"""Tests for mutation infrastructure report validation."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from check_mutation_infrastructure import infrastructure_failures

REPOSITORY_ROOT = Path(__file__).resolve().parents[1]


class MutationInfrastructureTests(unittest.TestCase):
    """Exercise clean, corrupted, and infrastructure-failure reports."""

    def write_report(self, root: Path, summary: str, log: str) -> None:
        """Write the smallest realistic cargo-mutants report and build log."""
        log_directory = root / "log"
        log_directory.mkdir(parents=True)
        (log_directory / "mutation.log").write_text(log, encoding="utf-8")
        report = {
            "outcomes": [
                {
                    "scenario": {"Mutant": {"name": "src/core/save.rs:1: sample"}},
                    "summary": summary,
                    "log_path": "log/mutation.log",
                }
            ]
        }
        (root / "outcomes.json").write_text(json.dumps(report), encoding="utf-8")

    def test_semantic_compile_failure_remains_unviable(self) -> None:
        """Ordinary Rust type errors are valid unviable classifications."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_report(
                root,
                "Unviable",
                "error[E0308]: mismatched types\nerror: could not compile `noter`",
            )

            self.assertEqual(infrastructure_failures(root), [])

    def test_windows_linker_lock_is_rejected(self) -> None:
        """The observed LNK1104 lock cannot count as compiler rejection."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_report(
                root,
                "Unviable",
                "error: linking with `link.exe` failed\nLINK : fatal error LNK1104",
            )

            self.assertEqual(
                infrastructure_failures(root),
                [
                    "src/core/save.rs:1: sample: linker invocation failure in "
                    "log/mutation.log"
                ],
            )

    def test_ansi_decorated_linker_failure_is_rejected(self) -> None:
        """Cargo color escapes cannot conceal a linker invocation failure."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_report(
                root,
                "Unviable",
                "\x1b[1m\x1b[91merror\x1b[0m: linking with `cc` failed",
            )

            self.assertEqual(
                infrastructure_failures(root),
                [
                    "src/core/save.rs:1: sample: linker invocation failure in "
                    "log/mutation.log"
                ],
            )

    def test_clang_linker_process_crash_is_rejected(self) -> None:
        """A clang signal failure cannot count as a compiler rejection."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_report(
                root,
                "Unviable",
                "clang: error: linker command failed due to signal "
                "(use -v to see invocation)",
            )

            self.assertEqual(
                infrastructure_failures(root),
                [
                    "src/core/save.rs:1: sample: linker process crash in "
                    "log/mutation.log"
                ],
            )

    def test_macos_mutation_job_bounds_repeated_linker_input(self) -> None:
        """Keep both reviewed bounds beside serialized macOS linking."""
        workflow = (REPOSITORY_ROOT / ".github/workflows/ci.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn(
            "          CARGO_BUILD_JOBS: 1\n"
            "          CARGO_PROFILE_TEST_CODEGEN_UNITS: '8'\n"
            "          CARGO_PROFILE_TEST_DEBUG: '0'\n"
            "          CARGO_INCREMENTAL: 1\n",
            workflow,
        )
        self.assertEqual(workflow.count("CARGO_PROFILE_TEST_CODEGEN_UNITS: '8'"), 1)
        self.assertEqual(workflow.count("CARGO_PROFILE_TEST_DEBUG: '0'"), 1)
        self.assertIn("macOS mutation infrastructure flake detected; retrying once", workflow)

    def test_caught_mutant_log_is_not_reclassified(self) -> None:
        """Only build failures already labeled unviable are inspected."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_report(root, "CaughtMutant", "fatal error LNK1104")

            self.assertEqual(infrastructure_failures(root), [])

    def test_missing_report_is_an_explicit_validation_error(self) -> None:
        """A missing report cannot silently pass the gate."""
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "mutation report is missing"):
                infrastructure_failures(Path(directory))


if __name__ == "__main__":
    unittest.main()
