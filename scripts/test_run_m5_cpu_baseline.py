import contextlib
import io
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import generate_m5_corpus
import run_m5_cpu_baseline as runner


class CpuBaselineTests(unittest.TestCase):
    def setUp(self):
        self.case = {"bytes": 1024, "wrapped": True, "accesskit": False}
        self.report = {
            "schema": 1,
            **self.case,
            "warmup_count": 5,
            "sample_count": 30,
            "first_frame_cpu_ns": 100,
            "clock_ns": [0] * 30,
            "idle_frame_cpu_ns": list(range(1, 31)),
            "edit_frame_cpu_ns": list(range(31, 61)),
            "idle_accesskit_nodes": [0] * 30,
            "edit_accesskit_nodes": [0] * 30,
            "edit_revisions": list(range(6, 36)),
        }

    def output(self, report=None):
        return runner.PREFIX + json.dumps(self.report if report is None else report)

    def test_raw_order_and_nearest_rank_summary_are_preserved(self):
        result = runner.validate_worker(self.output(), self.case, 30, 5)
        self.assertEqual(result["edit_frame_cpu_ns"], list(range(31, 61)))
        self.assertEqual(result["edit_frame_cpu_summary"]["p95_ns"], 59)
        self.assertEqual(result["edit_frame_cpu_summary"]["p99_ns"], 60)
        self.assertEqual(result["clock_ns"], [0] * 30)
        self.assertEqual(
            runner.validate_worker(
                "test cpu_probe ... " + self.output(), self.case, 30, 5
            ),
            result,
        )

    def test_missing_duplicate_and_inconsistent_worker_reports_fail(self):
        for output in (
            "",
            runner.PREFIX + "[]",
            runner.PREFIX + "null",
            self.output() + "\n" + self.output(),
        ):
            with self.subTest(output=output[:30]), self.assertRaises(ValueError):
                runner.validate_worker(output, self.case, 30, 5)
        for key, value in (
            ("bytes", 1023),
            ("schema", True),
            ("wrapped", False),
            ("warmup_count", 0),
            ("sample_count", 29),
            ("first_frame_cpu_ns", 0),
            ("idle_frame_cpu_ns", [1] * 29),
            ("edit_frame_cpu_ns", [True] * 30),
            ("clock_ns", [-1] * 30),
            ("edit_frame_cpu_ns", [0] * 30),
            ("edit_revisions", [6] * 30),
            ("edit_accesskit_nodes", [1] * 30),
            ("idle_accesskit_nodes", [99] * 30),
        ):
            report = {**self.report, key: value}
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                runner.validate_worker(self.output(report), self.case, 30, 5)

    def test_accessibility_enabled_requires_updates_for_every_edit(self):
        case = {**self.case, "accesskit": True}
        report = {**self.report, "accesskit": True, "edit_accesskit_nodes": [10] * 30}
        self.assertEqual(
            runner.validate_worker(self.output(report), case, 30, 5)["accesskit"], True
        )
        report["edit_accesskit_nodes"] = [10] * 29 + [0]
        with self.assertRaises(ValueError):
            runner.validate_worker(self.output(report), case, 30, 5)

    def test_input_must_match_frozen_bytes_and_respect_normal_limit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = generate_m5_corpus.generate_corpus(root, "smoke")
            record = manifest["files"][0]
            path = runner.validate_input(root, record)
            self.assertEqual(path.name, record["name"])
            for replacement in (b"x", b"x" * record["bytes"]):
                path.write_bytes(replacement)
                with self.assertRaises(ValueError):
                    runner.validate_input(root, record)
            with self.assertRaisesRegex(ValueError, "cannot bypass"):
                runner.validate_input(
                    root, {**record, "bytes": 50 * runner.baseline.MIB}
                )

    def test_cargo_artifact_selection_requires_one_release_binary_test(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory, "noter-test")
            binary.touch()
            artifact = {
                "reason": "compiler-artifact",
                "target": {"name": "noter", "kind": ["bin"]},
                "profile": {"test": True, "opt_level": "3"},
                "executable": str(binary),
            }
            output = json.dumps(artifact)
            self.assertEqual(runner.select_executable(output), binary)
            for field, value in (
                ("target", {"name": "noter", "kind": ["lib"]}),
                ("profile", {"test": False, "opt_level": "3"}),
                ("profile", {"test": True, "opt_level": "1"}),
                ("executable", str(binary.with_name("missing"))),
            ):
                with (
                    self.subTest(field=field, value=value),
                    self.assertRaises(ValueError),
                ):
                    runner.select_executable(json.dumps({**artifact, field: value}))
            with self.assertRaises(ValueError):
                runner.select_executable(output + "\n" + output)

    def test_worker_failure_restores_environment_and_propagates(self):
        key = "NOTER_M5_CPU_INPUT"
        with (
            patch.dict(os.environ, {key: "previous"}),
            patch.object(
                runner.baseline,
                "_run_checked",
                side_effect=RuntimeError("deadline exceeded"),
            ),
        ):
            with self.assertRaisesRegex(RuntimeError, "deadline exceeded"):
                runner.run_case(Path("worker"), Path("synthetic"), self.case, 30, 5, 1)
            self.assertEqual(os.environ[key], "previous")

    def test_no_output_overwrite_duplicate_cases_or_50mib_selection(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory, "existing.json")
            output.write_text("retained")
            base = ["--corpus", directory, "--output", str(output)]
            with (
                patch.object(runner, "run") as execute,
                contextlib.redirect_stderr(io.StringIO()),
            ):
                with self.assertRaises(SystemExit):
                    runner.main(base)
                self.assertEqual(output.read_text(), "retained")
                output.unlink()
                for selection in (
                    ["--profiles", "prose", "prose"],
                    ["--sizes", str(50 * runner.baseline.MIB)],
                    ["--samples", "29"],
                ):
                    with (
                        self.subTest(selection=selection),
                        self.assertRaises(SystemExit),
                    ):
                        runner.main(base + selection)
                execute.assert_not_called()

    def run_fixture(
        self,
        *,
        worker_error=None,
        worker_output="",
        source_results=None,
        hashes=None,
        real_worker=False,
    ):
        reference = {
            "files": [{"name": "synthetic", "profile": "prose", "bytes": 1024}],
            "corpus_sha256": "frozen",
        }
        args = type(
            "Arguments",
            (),
            {
                "profiles": ["prose"],
                "sizes": [1024],
                "corpus": Path("."),
                "samples": 30,
                "warmup": 5,
                "timeout": 1,
            },
        )()
        with (
            patch.object(
                runner,
                "source_identity",
                return_value={"commit": "frozen"},
                side_effect=source_results,
            ),
            patch.object(
                runner.REFERENCE.__class__,
                "read_text",
                return_value=json.dumps(reference),
            ),
            patch.object(runner, "validate_input", return_value=Path("synthetic")),
            patch.object(runner, "select_executable", return_value=Path("worker")),
            patch.object(runner, "digest", return_value="hash", side_effect=hashes),
            patch.object(runner.baseline, "_run_checked", return_value=worker_output),
            patch.object(
                runner.baseline,
                "collect_environment",
                return_value={"display_refresh_hz": None},
            ),
            patch.object(runner, "run_case", wraps=runner.run_case)
            if real_worker
            else patch.object(
                runner,
                "run_case",
                side_effect=worker_error,
                return_value={"raw_samples": [1]},
            ) as worker,
        ):
            receipt = runner.run(args)
        return receipt, worker.call_count

    def test_failed_attempt_is_retained_and_stops_further_sampling(self):
        receipt, calls = self.run_fixture(worker_error=RuntimeError("worker timed out"))
        self.assertEqual(receipt["status"], "failed")
        self.assertEqual(len(receipt["attempts"]), 1)
        self.assertEqual(receipt["attempts"][0]["status"], "failed")
        self.assertEqual(receipt["attempts"][0]["error"], "worker timed out")
        self.assertNotIn("measurement", receipt["attempts"][0])
        self.assertEqual(calls, 1)

    def test_malformed_worker_report_is_retained_as_a_failed_attempt(self):
        receipt, calls = self.run_fixture(
            worker_output=runner.PREFIX + "[]", real_worker=True
        )
        self.assertEqual(receipt["status"], "failed")
        self.assertEqual(calls, 1)
        self.assertEqual(receipt["attempts"][0]["status"], "failed")
        self.assertIn("must be an object", receipt["attempts"][0]["error"])

    def test_final_source_and_binary_failures_retain_all_collected_attempts(self):
        for options in (
            {"source_results": [{"commit": "frozen"}, {"commit": "changed"}]},
            {
                "source_results": [
                    {"commit": "frozen"},
                    ValueError("checkout became dirty"),
                ]
            },
            {"hashes": ["hash", "lock", "changed"]},
            {"hashes": ["hash", "lock", OSError("binary disappeared")]},
        ):
            with self.subTest(options=options):
                receipt, calls = self.run_fixture(**options)
                self.assertEqual(receipt["status"], "failed")
                self.assertIn("finalization_error", receipt)
                self.assertEqual(calls, 4)
                self.assertEqual(len(receipt["attempts"]), 4)
                self.assertTrue(
                    all(
                        attempt["status"] == "complete"
                        for attempt in receipt["attempts"]
                    )
                )
                self.assertTrue(
                    all(
                        attempt["measurement"]["raw_samples"] == [1]
                        for attempt in receipt["attempts"]
                    )
                )


if __name__ == "__main__":
    unittest.main()
