"""Tests for the target-specific release SBOM generator."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

import generate_release_sboms as generator


class ReleaseSbomGeneratorTests(unittest.TestCase):
    def test_release_revision_defaults_to_checked_out_head(self) -> None:
        self.assertEqual(generator.release_revision({}), "HEAD")

    def test_release_revision_accepts_a_full_lowercase_commit_id(self) -> None:
        revision = "a" * 40
        self.assertEqual(generator.release_revision({"GITHUB_SHA": revision}), revision)

    def test_release_revision_rejects_an_ambiguous_commit_id(self) -> None:
        with self.assertRaisesRegex(ValueError, "full lowercase Git commit ID"):
            generator.release_revision({"GITHUB_SHA": "abc123"})

    @mock.patch.object(generator.subprocess, "run")
    def test_source_date_epoch_uses_the_selected_revision(self, run: mock.Mock) -> None:
        run.return_value = SimpleNamespace(stdout="1777777777\n")
        root = Path("repository")

        self.assertEqual(generator.source_date_epoch(root, "a" * 40), "1777777777")
        run.assert_called_once_with(
            ["git", "show", "-s", "--format=%ct", "a" * 40],
            cwd=root,
            check=True,
            capture_output=True,
            text=True,
        )

    @mock.patch.object(generator.subprocess, "run")
    def test_source_date_epoch_rejects_invalid_git_output(self, run: mock.Mock) -> None:
        run.return_value = SimpleNamespace(stdout="not-a-timestamp\n")
        with self.assertRaisesRegex(ValueError, "invalid commit timestamp"):
            generator.source_date_epoch(Path("repository"), "HEAD")

    def test_validate_sbom_accepts_cyclonedx_1_5(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "noter_test.cdx.xml"
            path.write_text(
                '<bom xmlns="http://cyclonedx.org/schema/bom/1.5" version="1"/>',
                encoding="utf-8",
            )
            generator.validate_sbom(path)

    def test_validate_sbom_rejects_cyclonedx_1_4(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "noter_test.cdx.xml"
            path.write_text(
                '<bom xmlns="http://cyclonedx.org/schema/bom/1.4" version="1"/>',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "not CycloneDX 1.5"):
                generator.validate_sbom(path)

    def test_remove_previous_output_refuses_a_directory(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "noter_test.cdx.xml"
            path.mkdir()
            with self.assertRaisesRegex(ValueError, "non-regular SBOM output"):
                generator.remove_previous_output(path)

    @mock.patch.object(generator, "validate_sbom")
    @mock.patch.object(generator, "remove_previous_output")
    @mock.patch.object(generator, "source_date_epoch", return_value="1777777777")
    @mock.patch.object(generator.subprocess, "run")
    def test_generate_declared_target_set_without_a_shell(
        self,
        run: mock.Mock,
        _source_date_epoch: mock.Mock,
        remove_previous_output: mock.Mock,
        validate_sbom: mock.Mock,
    ) -> None:
        with mock.patch.dict(generator.os.environ, {}, clear=True):
            generator.generate_release_sboms(Path("repository"))

        self.assertEqual(run.call_count, len(generator.RELEASE_TARGETS))
        generated_targets = []
        for call in run.call_args_list:
            command = call.args[0]
            generated_targets.append(command[command.index("--target") + 1])
            self.assertNotIn("shell", call.kwargs)
            self.assertEqual(call.kwargs["env"]["SOURCE_DATE_EPOCH"], "1777777777")
        self.assertEqual(tuple(generated_targets), generator.RELEASE_TARGETS)
        self.assertEqual(
            remove_previous_output.call_count, 3 * len(generator.RELEASE_TARGETS)
        )
        self.assertEqual(validate_sbom.call_count, len(generator.RELEASE_TARGETS))


if __name__ == "__main__":
    unittest.main()
