import hashlib
import io
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

import run_m1_baseline as baseline


class PercentileTests(unittest.TestCase):
    def test_nearest_rank_percentiles_use_all_raw_samples(self):
        samples = list(range(1, 31))

        self.assertEqual(baseline.nearest_rank(samples, 0.50), 15)
        self.assertEqual(baseline.nearest_rank(samples, 0.95), 29)
        self.assertEqual(baseline.nearest_rank(samples, 0.99), 30)
        self.assertEqual(
            baseline.summarize_nanoseconds(samples),
            {
                "minimum_ns": 1,
                "p50_ns": 15,
                "p95_ns": 29,
                "p99_ns": 30,
                "maximum_ns": 30,
            },
        )

    def test_nearest_rank_rejects_empty_or_invalid_input(self):
        for samples, percentile in (([], 0.5), ([1], 0.0), ([1], 1.01), ([0], 0.5)):
            with self.subTest(samples=samples, percentile=percentile):
                with self.assertRaises(ValueError):
                    baseline.nearest_rank(samples, percentile)


class CorpusTests(unittest.TestCase):
    def test_smoke_corpus_is_exact_reproducible_and_utf8(self):
        with (
            tempfile.TemporaryDirectory() as first,
            tempfile.TemporaryDirectory() as second,
        ):
            first_manifest = baseline.generate_corpus(Path(first), "smoke")
            second_manifest = baseline.generate_corpus(Path(second), "smoke")

            self.assertEqual(first_manifest, second_manifest)
            self.assertEqual(first_manifest["scale"], "smoke")
            self.assertEqual(len(first_manifest["files"]), 7)
            self.assertEqual(
                first_manifest["corpus_sha256"],
                baseline.corpus_manifest_digest(first_manifest["files"]),
            )
            for record in first_manifest["files"]:
                path = Path(first, record["name"])
                content = path.read_bytes()
                self.assertEqual(len(content), record["bytes"])
                self.assertEqual(hashlib.sha256(content).hexdigest(), record["sha256"])
                content.decode("utf-8")

            source = Path(first, "source-large.txt").read_bytes()
            for marker in baseline.SEARCH_MARKERS.values():
                self.assertEqual(source.count(marker.encode("ascii")), 1)

    def test_generation_refuses_to_mix_with_existing_content(self):
        with tempfile.TemporaryDirectory() as directory:
            Path(directory, "unrelated.txt").write_text("preserve", encoding="utf-8")

            with self.assertRaises(FileExistsError):
                baseline.generate_corpus(Path(directory), "smoke")

            self.assertEqual(
                Path(directory, "unrelated.txt").read_text(encoding="utf-8"), "preserve"
            )

    def test_unknown_scale_and_invalid_pattern_fail_before_writing(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                baseline.generate_corpus(Path(directory), "mini")
            with self.assertRaises(ValueError):
                baseline.write_repeated_file(Path(directory, "bad"), b"", 12)
            with self.assertRaises(ValueError):
                baseline.write_repeated_file(Path(directory, "negative"), b"x", -1)
            marker_target = Path(directory, "marker.txt")
            baseline.write_repeated_file(marker_target, b"x", 4)
            with self.assertRaises(ValueError):
                baseline._patch_ascii(marker_target, 3, "wide")


class WorkerResultTests(unittest.TestCase):
    def test_worker_result_requires_exact_case_and_sample_contract(self):
        payload = json.dumps(
            {
                "case": "load-prose-1mib",
                "warmup": 3,
                "samples_ns": [7, 11, 13],
                "checksum": 99,
            }
        )

        parsed = baseline.parse_worker_result(payload, "load-prose-1mib", 3, 3)

        self.assertEqual(parsed["samples_ns"], [7, 11, 13])
        self.assertEqual(parsed["checksum"], 99)

    def test_worker_result_rejects_malformed_or_mismatched_payloads(self):
        invalid = (
            "not json",
            json.dumps(
                {"case": "wrong", "warmup": 0, "samples_ns": [1], "checksum": 1}
            ),
            json.dumps(
                {
                    "case": "load-prose-1mib",
                    "warmup": 0,
                    "samples_ns": [],
                    "checksum": 1,
                }
            ),
            json.dumps(
                {
                    "case": "load-prose-1mib",
                    "warmup": 0,
                    "samples_ns": [0],
                    "checksum": 1,
                }
            ),
            json.dumps(
                {
                    "case": "load-prose-1mib",
                    "warmup": 0,
                    "samples_ns": [1, 2],
                    "checksum": 1,
                }
            ),
            json.dumps(
                {
                    "case": "load-prose-1mib",
                    "warmup": -1,
                    "samples_ns": [1],
                    "checksum": 1,
                }
            ),
            json.dumps(
                {
                    "case": "load-prose-1mib",
                    "warmup": 0,
                    "samples_ns": [1],
                    "checksum": True,
                }
            ),
        )
        for payload in invalid:
            with self.subTest(payload=payload):
                with self.assertRaises(ValueError):
                    baseline.parse_worker_result(payload, "load-prose-1mib", 1, 0)


class EnvironmentParserTests(unittest.TestCase):
    def test_linux_memory_and_filesystem_parsers_are_unit_explicit(self):
        status = "Name:\tnoter\nVmRSS:\t  2048 kB\nVmHWM:\t  4096 kB\n"

        self.assertEqual(baseline.parse_linux_peak_rss(status), 4_194_304)
        self.assertEqual(
            baseline.parse_findmnt("ext4 /dev/sdd[/workspace]\n"),
            {"type": "ext4", "source_class": "block-device"},
        )
        self.assertEqual(
            baseline.parse_findmnt("9p C:\\\n"),
            {"type": "9p", "source_class": "host-bridge"},
        )
        self.assertEqual(
            baseline.parse_findmnt("cifs //server/share\n"),
            {"type": "cifs", "source_class": "network"},
        )
        self.assertEqual(
            baseline.parse_findmnt("tmpfs tmpfs\n"),
            {"type": "tmpfs", "source_class": "other"},
        )
        with self.assertRaises(ValueError):
            baseline.parse_linux_peak_rss("VmRSS:\t2048 kB\n")
        with self.assertRaises(ValueError):
            baseline.parse_findmnt("")


class DependencyTests(unittest.TestCase):
    def test_dependency_summary_uses_cross_target_union_and_declared_kinds(self):
        root_package = {
            "id": "noter",
            "dependencies": [
                {"name": "runtime", "kind": None},
                {"name": "development", "kind": "dev"},
                {"name": "builder", "kind": "build"},
                {"name": "runtime", "kind": None},
            ],
        }
        windows = {
            "packages": [
                root_package,
                {"id": "runtime@1", "name": "runtime", "version": "1.0.0"},
                {"id": "split@1", "name": "split", "version": "1.0.0"},
            ]
        }
        linux = {
            "packages": [
                root_package,
                {"id": "runtime@1", "name": "runtime", "version": "1.0.0"},
                {"id": "split@2", "name": "split", "version": "2.0.0"},
            ]
        }

        summary = baseline.summarize_dependencies(
            {"windows": windows, "linux": linux}, "noter"
        )

        self.assertEqual(summary["resolved_package_union"], 4)
        self.assertEqual(
            summary["resolved_packages_by_target"], {"linux": 3, "windows": 3}
        )
        self.assertEqual(
            summary["direct_dependencies"], {"build": 1, "development": 1, "runtime": 1}
        )
        self.assertEqual(summary["duplicate_versions"], {"split": ["1.0.0", "2.0.0"]})

    def test_locked_package_count_reads_exact_toml_records(self):
        with tempfile.TemporaryDirectory() as directory:
            lock_path = Path(directory, "Cargo.lock")
            lock_path.write_text(
                'version = 4\n\n[[package]]\nname = "one"\nversion = "1.0.0"\n'
                '\n[[package]]\nname = "two"\nversion = "2.0.0"\n',
                encoding="utf-8",
            )

            self.assertEqual(baseline.locked_package_count(lock_path), 2)

    def test_dependency_summary_rejects_incomplete_metadata(self):
        invalid_inputs = (
            {"windows": {}},
            {"windows": {"packages": ["invalid"]}},
            {"windows": {"packages": [{"id": "other", "dependencies": []}]}},
            {"windows": {"packages": [{"id": "noter"}]}},
        )
        for metadata in invalid_inputs:
            with self.subTest(metadata=metadata):
                with self.assertRaises(ValueError):
                    baseline.summarize_dependencies(metadata, "noter")


class OrchestrationTests(unittest.TestCase):
    def test_checked_command_reports_success_and_bounded_failure(self):
        self.assertEqual(
            baseline._run_checked([sys.executable, "-c", "print('result')"]),
            "result\n",
        )
        with self.assertRaisesRegex(RuntimeError, "exit 7: last diagnostic"):
            baseline._run_checked(
                [
                    sys.executable,
                    "-c",
                    "import sys; print('first', file=sys.stderr); "
                    "print('last diagnostic', file=sys.stderr); raise SystemExit(7)",
                ]
            )
        with (
            patch.object(baseline, "MAXIMUM_COMMAND_OUTPUT_BYTES", 8),
            self.assertRaisesRegex(RuntimeError, "bounded output limit"),
        ):
            baseline._run_checked([sys.executable, "-c", "print('123456789')"])
        with self.assertRaisesRegex(RuntimeError, "deadline"):
            baseline._run_checked(
                [sys.executable, "-c", "import time; time.sleep(10)"], timeout=1
            )

    def test_checked_command_terminates_noisy_process_at_output_limit(self):
        command = (
            "import sys, time\n"
            "stream = sys.stderr if sys.argv[1] == 'stderr' else sys.stdout\n"
            "while True:\n"
            "    stream.write('x' * 4096)\n"
            "    stream.flush()\n"
            "    time.sleep(0.001)\n"
        )
        for stream in ("stdout", "stderr"):
            started = time.monotonic()
            with (
                self.subTest(stream=stream),
                patch.object(baseline, "MAXIMUM_COMMAND_OUTPUT_BYTES", 8_192),
                self.assertRaisesRegex(RuntimeError, "bounded output limit"),
            ):
                baseline._run_checked(
                    [sys.executable, "-c", command, stream], timeout=10
                )
            self.assertLess(time.monotonic() - started, 5)

    @unittest.skipUnless(os.name == "nt", "Win32 process-tree behavior is required")
    def test_output_limit_terminates_windows_descendants(self):
        import ctypes
        from ctypes import wintypes

        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel32.OpenProcess.restype = wintypes.HANDLE
        kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        kernel32.WaitForSingleObject.restype = wintypes.DWORD
        kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel32.CloseHandle.restype = wintypes.BOOL

        def process_is_running(process_id):
            synchronize = 0x00100000
            wait_timeout = 258
            handle = kernel32.OpenProcess(synchronize, False, process_id)
            if not handle:
                return False
            try:
                return kernel32.WaitForSingleObject(handle, 0) == wait_timeout
            finally:
                kernel32.CloseHandle(handle)

        launcher = (
            "import pathlib, subprocess, sys, time\n"
            "child = subprocess.Popen([sys.executable, '-c', "
            "'import time; time.sleep(30)'])\n"
            "pathlib.Path(sys.argv[2]).write_text(str(child.pid), encoding='ascii')\n"
            "stream = sys.stderr if sys.argv[1] == 'stderr' else sys.stdout\n"
            "while True:\n"
            "    stream.write('x' * 4096)\n"
            "    stream.flush()\n"
            "    time.sleep(0.001)\n"
        )
        for stream in ("stdout", "stderr"):
            child_pid = None
            with tempfile.TemporaryDirectory() as directory:
                pid_path = Path(directory, "child.pid")
                try:
                    with (
                        self.subTest(stream=stream),
                        patch.object(baseline, "MAXIMUM_COMMAND_OUTPUT_BYTES", 8_192),
                        self.assertRaisesRegex(RuntimeError, "bounded output limit"),
                    ):
                        baseline._run_checked(
                            [sys.executable, "-c", launcher, stream, str(pid_path)],
                            timeout=10,
                        )
                    child_pid = int(pid_path.read_text(encoding="ascii"))
                    self.assertFalse(process_is_running(child_pid))
                finally:
                    if child_pid is not None and process_is_running(child_pid):
                        subprocess.run(
                            ["taskkill", "/PID", str(child_pid), "/T", "/F"],
                            check=False,
                            stdin=subprocess.DEVNULL,
                            stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL,
                            timeout=10,
                        )

    @unittest.skipUnless(os.name == "nt", "Win32 process-tree behavior is required")
    def test_windows_command_cannot_run_before_job_activation(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory, "started")
            with baseline._WindowsProcessJob() as job:
                process = subprocess.Popen(
                    [
                        sys.executable,
                        "-c",
                        "import pathlib, sys; pathlib.Path(sys.argv[1]).touch()",
                        str(marker),
                    ],
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    creationflags=(
                        subprocess.CREATE_NEW_PROCESS_GROUP
                        | baseline.WINDOWS_CREATE_SUSPENDED
                    ),
                )
                try:
                    time.sleep(0.1)
                    self.assertFalse(marker.exists())

                    job.activate(process)

                    self.assertEqual(process.wait(timeout=10), 0)
                    self.assertTrue(marker.is_file())
                finally:
                    if process.poll() is None:
                        baseline._terminate_process_tree(process, job)

    def test_cross_target_metadata_and_release_binary_discovery(self):
        root_package = {
            "id": "noter-id",
            "name": "noter",
            "manifest_path": str(baseline.ROOT / "Cargo.toml"),
            "dependencies": [],
        }
        metadata = json.dumps({"packages": [root_package]})
        with patch.object(baseline, "_run_checked", return_value=metadata) as run:
            by_target, root_id = baseline._cargo_metadata()
        self.assertEqual(root_id, "noter-id")
        self.assertEqual(set(by_target), set(baseline.SUPPORTED_TARGETS))
        self.assertEqual(run.call_count, len(baseline.SUPPORTED_TARGETS))
        for call in run.call_args_list:
            self.assertIn("--all-features", call.args[0])

        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory, "target")
            binary = target / "release" / "noter.exe"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"release-binary")
            outputs = [
                "",
                json.dumps({"target_directory": str(target)}),
            ]
            with (
                patch.object(baseline, "_run_checked", side_effect=outputs),
                patch.object(baseline.platform, "system", return_value="Windows"),
            ):
                result = baseline._build_release_binary()
        self.assertEqual(result["bytes"], len(b"release-binary"))
        self.assertEqual(
            result["sha256"], hashlib.sha256(b"release-binary").hexdigest()
        )

    def test_benchmark_worker_discovery_requires_exact_case_list(self):
        with tempfile.TemporaryDirectory() as directory:
            executable = Path(directory, "worker.exe")
            executable.write_bytes(b"worker")
            compiler_message = json.dumps(
                {
                    "reason": "compiler-artifact",
                    "target": {"name": "m1_baseline"},
                    "executable": str(executable),
                }
            )
            listed = json.dumps([case for case, _, _ in baseline.BENCHMARK_CASES])
            with patch.object(
                baseline, "_run_checked", side_effect=[compiler_message, listed]
            ):
                self.assertEqual(baseline._build_benchmark_worker(), executable)
            with patch.object(baseline, "_run_checked", return_value="{}"):
                with self.assertRaises(RuntimeError):
                    baseline._build_benchmark_worker()

    def test_worker_process_protocol_records_memory_and_releases_hold(self):
        payload = json.dumps(
            {
                "case": "load-empty",
                "warmup": 0,
                "samples_ns": [17],
                "checksum": 0,
            }
        )

        class FakeProcess:
            def __init__(self):
                class RetainedInput(io.StringIO):
                    def close(self):
                        self.was_closed = True

                self.stdout = io.StringIO(payload + "\n")
                self.stderr = io.StringIO("")
                self.stdin = RetainedInput()
                self.pid = os.getpid()
                self.killed = False

            def wait(self, timeout):
                self.timeout = timeout
                return 0

            def kill(self):
                self.killed = True

        process = FakeProcess()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with (
                patch.object(baseline.subprocess, "Popen", return_value=process),
                patch.object(
                    baseline,
                    "_process_memory",
                    return_value=("peak-working-set", 4_096),
                ),
            ):
                result, metric, memory = baseline._run_worker(
                    root / "worker", "load-empty", root, root / "work", 1, 0, 5
                )
        self.assertEqual(result["samples_ns"], [17])
        self.assertEqual((metric, memory), ("peak-working-set", 4_096))
        self.assertEqual(process.stdin.getvalue(), "\n")

    def test_benchmark_matrix_has_exact_cold_and_warm_shapes(self):
        def fake_worker(_executable, case, _corpus, _work, samples, warmup, _timeout):
            return (
                {
                    "case": case,
                    "warmup": warmup,
                    "samples_ns": [11] * samples,
                    "checksum": len(case),
                },
                "peak-working-set",
                8_192,
            )

        with patch.object(baseline, "_run_worker", side_effect=fake_worker):
            results = baseline._run_benchmarks(
                Path("worker"), Path("corpus"), Path("work"), 2, 1
            )

        self.assertEqual(len(results), 22)
        self.assertEqual(
            {(result["case"], result["state"]) for result in results},
            {
                (case, state)
                for case, cold, _ in baseline.BENCHMARK_CASES
                for state in (
                    ("process-cold", "warm-in-process")
                    if cold
                    else ("warm-in-process",)
                )
            },
        )
        self.assertTrue(all(result["summary"]["p95_ns"] == 11 for result in results))


class PlatformEvidenceTests(unittest.TestCase):
    @unittest.skipUnless(os.name == "nt", "Win32 diagnostics require Windows")
    def test_windows_peak_working_set_preserves_native_errors(self):
        with self.assertRaises(OSError) as raised:
            baseline._windows_peak_working_set(0xFFFFFFFF)

        self.assertNotEqual(raised.exception.errno, 0)

    def test_process_memory_adapters_name_units_explicitly(self):
        with (
            patch.object(baseline.platform, "system", return_value="Windows"),
            patch.object(baseline, "_windows_peak_working_set", return_value=12_345),
        ):
            self.assertEqual(baseline._process_memory(7), ("peak-working-set", 12_345))
        with (
            patch.object(baseline.platform, "system", return_value="Linux"),
            patch.object(
                Path,
                "read_text",
                return_value="VmHWM:\t 64 kB\n",
            ),
        ):
            self.assertEqual(
                baseline._process_memory(7),
                ("peak-resident-set-vmhwm", 65_536),
            )
        with (
            patch.object(baseline.platform, "system", return_value="Darwin"),
            patch.object(baseline, "_run_checked", return_value="32\n"),
        ):
            self.assertEqual(
                baseline._process_memory(7),
                ("held-resident-set-snapshot", 32_768),
            )
        with patch.object(baseline.platform, "system", return_value="Plan9"):
            with self.assertRaises(RuntimeError):
                baseline._process_memory(7)

    def test_memory_cpu_storage_and_display_collection_are_fallback_safe(self):
        with (
            patch.object(baseline.platform, "system", return_value="Linux"),
            patch.object(
                Path,
                "read_text",
                side_effect=[
                    "MemTotal: 2048 kB\n",
                    "model name : Example Linux CPU\n",
                ],
            ),
        ):
            self.assertEqual(baseline._memory_bytes(), 2_097_152)
            self.assertEqual(baseline._cpu_name(), "Example Linux CPU")
        with (
            patch.object(baseline.platform, "system", return_value="Darwin"),
            patch.object(
                baseline,
                "_run_checked",
                side_effect=["4096\n", "Example Apple CPU\n"],
            ),
        ):
            self.assertEqual(baseline._memory_bytes(), 4_096)
            self.assertEqual(baseline._cpu_name(), "Example Apple CPU")
        with (
            patch.object(baseline.platform, "system", return_value="Other"),
            patch.object(baseline.platform, "processor", return_value=""),
            patch.object(baseline.platform, "machine", return_value="fallback-cpu"),
        ):
            self.assertIsNone(baseline._memory_bytes())
            self.assertEqual(baseline._cpu_name(), "fallback-cpu")

        with (
            patch.object(baseline.platform, "system", return_value="Linux"),
            patch.object(baseline, "_run_checked", return_value="ext4 /dev/example\n"),
        ):
            self.assertEqual(
                baseline._storage_environment(Path(".")),
                {"type": "ext4", "source_class": "block-device"},
            )
        with (
            patch.object(baseline.platform, "system", return_value="Darwin"),
            patch.object(baseline, "_run_checked", return_value="apfs\n"),
        ):
            self.assertEqual(
                baseline._storage_environment(Path(".")),
                {"type": "apfs", "source_class": "local-or-mounted"},
            )
        with patch.object(baseline.platform, "system", return_value="Other"):
            self.assertEqual(
                baseline._storage_environment(Path(".")),
                {"type": "unknown", "source_class": "unknown"},
            )

        refresh_cases = (
            ("Windows", {}, "120\n", 120),
            ("Linux", {"DISPLAY": "display"}, "1920x1080 120.00*\n", 120),
            ("Darwin", {}, "Refresh Rate: 60 Hz\n", 60),
        )
        for system, environment, output, expected in refresh_cases:
            with (
                self.subTest(system=system),
                patch.object(baseline.platform, "system", return_value=system),
                patch.dict(os.environ, environment, clear=True),
                patch.object(baseline, "_run_checked", return_value=output),
            ):
                self.assertEqual(baseline._display_refresh_hz(), expected)
        with (
            patch.object(baseline.platform, "system", return_value="Windows"),
            patch.object(baseline, "_run_checked", side_effect=RuntimeError("missing")),
        ):
            self.assertIsNone(baseline._display_refresh_hz())

    def test_environment_and_git_records_are_non_identifying_and_fail_closed(self):
        with (
            patch.object(baseline, "_cpu_name", return_value="CPU"),
            patch.object(baseline, "_memory_bytes", return_value=1_024),
            patch.object(
                baseline,
                "_storage_environment",
                return_value={"type": "NTFS", "source_class": "local-volume"},
            ),
            patch.object(baseline, "_display_refresh_hz", return_value=60),
            patch.object(
                baseline,
                "_run_checked",
                side_effect=["rustc 1.97.1\n", "cargo 1.97.1\n"],
            ),
        ):
            environment = baseline.collect_environment(Path("."))
        self.assertEqual(environment["cpu"], "CPU")
        self.assertNotIn("hostname", environment)

        clean_git = [
            "a" * 40 + "\n",
            "b" * 40 + "\n",
            ".git\n",
            ".git\n",
            "feat/example\n",
            "",
        ]
        dirty_git = [*clean_git[:-1], " M file"]
        with patch.object(baseline, "_run_checked", side_effect=clean_git):
            self.assertTrue(baseline._git_source(False)["worktree_clean"])
        with patch.object(baseline, "_run_checked", side_effect=dirty_git):
            with self.assertRaises(RuntimeError):
                baseline._git_source(False)
        with patch.object(baseline, "_run_checked", side_effect=dirty_git):
            self.assertFalse(baseline._git_source(True)["worktree_clean"])


class ArtifactValidationTests(unittest.TestCase):
    @staticmethod
    def valid_artifact():
        raw = list(range(1, 31))
        files = baseline.reference_corpus_files()
        targets = sorted(baseline.SUPPORTED_TARGETS)
        artifact = {
            "schema_version": 2,
            "evidence_class": "reference",
            "generated_at": "2026-07-31T00:00:00Z",
            "source": {
                "commit": "a" * 40,
                "tree": "b" * 40,
                "worktree_clean": True,
                "checkout": "detached-worktree",
            },
            "scope": baseline.evidence_scope(),
            "provenance": dict(baseline.PROVENANCE),
            "environment": {
                "operating_system": "Windows",
                "operating_system_release": "11",
                "operating_system_version": "10.0.26200",
                "architecture": "AMD64",
                "cpu": "Example CPU",
                "logical_processors": 8,
                "memory_bytes": 16_000_000_000,
                "storage": {"type": "NTFS", "source_class": "local-volume"},
                "display_refresh_hz": 60,
                "rustc": "rustc 1.97.1",
                "cargo": "cargo 1.97.1",
                "python": "3.14.5",
                "build_profile": "bench",
            },
            "corpus": {
                "generator_version": 1,
                "scale": "full",
                "corpus_sha256": baseline.REFERENCE_CORPUS_SHA256,
                "files": files,
                "search_markers": dict(baseline.SEARCH_MARKERS),
                "adversarial_query_sha256": baseline.ADVERSARIAL_QUERY_SHA256,
            },
            "build": {
                "binary": {
                    "profile": "release",
                    "bytes": 9_000_000,
                    "sha256": "d" * 64,
                },
                "benchmark_worker": {
                    "profile": "bench",
                    "bytes": 5_000_000,
                    "sha256": "e" * 64,
                },
                "dependencies": {
                    "resolved_package_union": 400,
                    "locked_package_records": 416,
                    "release_targets": targets,
                    "resolved_packages_by_target": {target: 200 for target in targets},
                    "direct_dependencies": {
                        "runtime": 10,
                        "development": 2,
                        "build": 0,
                    },
                    "duplicate_versions": {"example": ["1.0.0", "2.0.0"]},
                },
            },
            "benchmarks": [],
        }
        for case, include_process_cold, _ in baseline.BENCHMARK_CASES:
            states = ["warm-in-process"]
            if include_process_cold:
                states.insert(0, "process-cold")
            for state in states:
                process_cold = state == "process-cold"
                artifact["benchmarks"].append(
                    {
                        "case": case,
                        "state": state,
                        "sample_count": 30,
                        "warmup_count": 0 if process_cold else 5,
                        "raw_samples_ns": raw,
                        "summary": baseline.summarize_nanoseconds(raw),
                        "worker_checksums": [0] * (30 if process_cold else 1),
                        "memory": {
                            "metric": "peak-working-set",
                            "raw_samples_bytes": [120_000_000]
                            * (30 if process_cold else 1),
                            "maximum_bytes": 120_000_000,
                        },
                    }
                )
        return artifact

    def test_reference_artifact_accepts_recomputable_raw_evidence(self):
        artifact = self.valid_artifact()

        baseline.validate_artifact(artifact, require_reference=True)

    def test_committed_windows_reference_is_exact_and_recomputable(self):
        path = (
            baseline.ROOT / "docs" / "evidence" / "m1-baseline-windows-2026-07-31.json"
        )

        artifact, digest = baseline.read_artifact(path)
        baseline.validate_artifact(artifact, require_reference=True)

        self.assertEqual(
            digest,
            "5da4643bf7f84c2ae37605c35a91c52e6e4f85fb0f06052f8ddfc0161bfd47e8",
        )
        self.assertEqual(
            artifact["source"],
            {
                "checkout": "detached-worktree",
                "commit": "580f16409957ecf0a3ff074a24703937231ca05d",
                "tree": "405a6d24bdb091fdc905f1a877cfd6cde8c97286",
                "worktree_clean": True,
            },
        )

    def test_reference_artifact_rejects_dirty_short_or_tampered_evidence(self):
        mutations = []
        dirty = self.valid_artifact()
        dirty["source"]["worktree_clean"] = False
        mutations.append(dirty)
        short = self.valid_artifact()
        short["benchmarks"][0]["raw_samples_ns"] = [1, 2]
        short["benchmarks"][0]["sample_count"] = 2
        short["benchmarks"][0]["summary"] = baseline.summarize_nanoseconds([1, 2])
        mutations.append(short)
        tampered = self.valid_artifact()
        tampered["benchmarks"][0]["summary"]["p95_ns"] = 1
        mutations.append(tampered)
        smoke = self.valid_artifact()
        smoke["evidence_class"] = "smoke"
        smoke["corpus"]["scale"] = "smoke"
        mutations.append(smoke)

        for artifact in mutations:
            with self.subTest(artifact=artifact):
                with self.assertRaises(ValueError):
                    baseline.validate_artifact(artifact, require_reference=True)

    def test_reference_validator_rejects_every_incomplete_evidence_layer(self):
        mutations = []

        def mutated(change):
            artifact = json.loads(json.dumps(self.valid_artifact()))
            change(artifact)
            mutations.append(artifact)

        mutated(lambda artifact: artifact.update(schema_version=3))
        mutated(lambda artifact: artifact.update(authenticated=True))
        mutated(lambda artifact: artifact.update(evidence_class="draft"))
        mutated(lambda artifact: artifact["source"].update(commit="bad"))
        mutated(lambda artifact: artifact["corpus"].update(claim="verified"))
        mutated(lambda artifact: artifact["corpus"].update(corpus_sha256="0" * 64))
        mutated(lambda artifact: artifact["corpus"]["files"][0].update(bytes=-1))
        mutated(lambda artifact: artifact["corpus"]["files"].pop())
        mutated(lambda artifact: artifact.update(environment={}))
        mutated(lambda artifact: artifact.update(build={}))
        mutated(
            lambda artifact: artifact["build"]["binary"].update(bytes=13 * baseline.MIB)
        )
        mutated(
            lambda artifact: artifact["build"]["dependencies"].pop(
                "locked_package_records"
            )
        )
        mutated(lambda artifact: artifact.update(benchmarks=[]))
        mutated(
            lambda artifact: artifact["benchmarks"][0].update(platform_verified=True)
        )
        mutated(
            lambda artifact: artifact["benchmarks"][0]["summary"].update(
                confidence="high"
            )
        )
        mutated(
            lambda artifact: artifact["benchmarks"].append(
                json.loads(json.dumps(artifact["benchmarks"][0]))
            )
        )
        mutated(lambda artifact: artifact["benchmarks"][0].update(sample_count=29))
        mutated(lambda artifact: artifact["benchmarks"][0].update(worker_checksums=[]))
        mutated(lambda artifact: artifact["benchmarks"][0].update(memory={}))
        mutated(lambda artifact: artifact["benchmarks"].pop())
        mutated(lambda artifact: artifact.pop("scope"))
        mutated(lambda artifact: artifact["environment"].update(cpu=""))
        mutated(lambda artifact: artifact["environment"].update(logical_processors=-1))
        mutated(lambda artifact: artifact["environment"].update(memory_bytes=None))
        mutated(
            lambda artifact: artifact["corpus"]["files"].__setitem__(
                0, json.loads(json.dumps(artifact["corpus"]["files"][1]))
            )
        )
        mutated(
            lambda artifact: artifact["benchmarks"][0]["memory"].update(
                raw_samples_bytes=[True] * 30, maximum_bytes=True
            )
        )
        duplicate = json.loads(json.dumps(self.valid_artifact()))
        duplicate["corpus"]["files"][0] = json.loads(
            json.dumps(duplicate["corpus"]["files"][1])
        )
        duplicate["corpus"]["corpus_sha256"] = baseline.corpus_manifest_digest(
            duplicate["corpus"]["files"]
        )
        mutations.append(duplicate)

        for index, artifact in enumerate(mutations):
            with self.subTest(index=index):
                with self.assertRaises(ValueError):
                    baseline.validate_artifact(artifact, require_reference=True)

    def test_artifact_write_is_exclusive_and_canonical(self):
        artifact = self.valid_artifact()
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory, "evidence.json")

            baseline.write_artifact(destination, artifact)

            self.assertEqual(
                json.loads(destination.read_text(encoding="utf-8")), artifact
            )
            self.assertTrue(destination.read_bytes().endswith(b"\n"))
            with self.assertRaises(FileExistsError):
                baseline.write_artifact(destination, artifact)

    def test_artifact_write_does_not_leave_a_final_file_when_promotion_fails(self):
        artifact = self.valid_artifact()
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory, "evidence.json")

            with (
                patch.object(baseline.os, "link", side_effect=OSError("injected")),
                self.assertRaises(OSError),
            ):
                baseline.write_artifact(destination, artifact)

            self.assertFalse(destination.exists())
            self.assertEqual(list(Path(directory).iterdir()), [])

    def test_artifact_reader_is_bounded_canonical_and_hashes_the_parsed_bytes(self):
        artifact = self.valid_artifact()
        expected = baseline.encode_artifact(artifact)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory, "evidence.json")
            path.write_bytes(expected)

            parsed, digest = baseline.read_artifact(path)

            self.assertEqual(parsed, artifact)
            self.assertEqual(digest, hashlib.sha256(expected).hexdigest())
            path.write_bytes(expected + b" \n")
            with self.assertRaisesRegex(ValueError, "canonical"):
                baseline.read_artifact(path)
            with (
                patch.object(baseline, "MAXIMUM_ARTIFACT_BYTES", 8),
                self.assertRaisesRegex(ValueError, "bounded size"),
            ):
                baseline.read_artifact(path)

    def test_source_commit_validation_binds_commit_to_tree(self):
        source = {"commit": "a" * 40, "tree": "b" * 40}
        with patch.object(baseline, "_run_checked", side_effect=["", "b" * 40]):
            baseline.verify_source_commit(source)
        with patch.object(baseline, "_run_checked", side_effect=["", "c" * 40]):
            with self.assertRaisesRegex(ValueError, "tree"):
                baseline.verify_source_commit(source)
        with patch.object(
            baseline, "_run_checked", side_effect=RuntimeError("missing")
        ):
            with self.assertRaisesRegex(ValueError, "not present"):
                baseline.verify_source_commit(source)

    def test_reference_destination_is_confined_to_canonical_evidence_directory(self):
        accepted = baseline.ROOT / "docs" / "evidence" / "m1.json"
        self.assertEqual(baseline._trusted_reference_destination(accepted), accepted)
        with self.assertRaisesRegex(ValueError, "docs/evidence"):
            baseline._trusted_reference_destination(baseline.ROOT / "m1.json")


class ProductionWorkflowTests(unittest.TestCase):
    @staticmethod
    def environment():
        return {
            "operating_system": "Windows",
            "operating_system_release": "11",
            "operating_system_version": "10.0.26200",
            "architecture": "AMD64",
            "cpu": "CPU",
            "logical_processors": 8,
            "memory_bytes": 16_000_000_000,
            "storage": {"type": "NTFS", "source_class": "local-volume"},
            "display_refresh_hz": 60,
            "rustc": "rustc 1.97.1",
            "cargo": "cargo 1.97.1",
            "python": "3.14.5",
            "build_profile": "bench",
        }

    def test_smoke_production_orchestration_writes_valid_exclusive_artifact(self):
        benchmark = {
            "case": "load-empty",
            "state": "warm-in-process",
            "sample_count": 1,
            "warmup_count": 0,
            "raw_samples_ns": [1],
            "summary": baseline.summarize_nanoseconds([1]),
            "worker_checksums": [0],
            "memory": {
                "metric": "peak-working-set",
                "raw_samples_bytes": [4_096],
                "maximum_bytes": 4_096,
            },
        }
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory, "smoke.json")
            worker = Path(directory, "worker.exe")
            worker.write_bytes(b"worker")
            targets = sorted(baseline.SUPPORTED_TARGETS)
            with (
                patch.object(
                    baseline,
                    "_git_source",
                    return_value={
                        "commit": "a" * 40,
                        "tree": "b" * 40,
                        "worktree_clean": False,
                        "checkout": "worktree",
                    },
                ),
                patch.object(
                    baseline, "collect_environment", return_value=self.environment()
                ),
                patch.object(baseline, "_cargo_metadata", return_value=({}, "noter")),
                patch.object(
                    baseline,
                    "summarize_dependencies",
                    return_value={
                        "release_targets": targets,
                        "resolved_packages_by_target": {
                            target: 3 for target in targets
                        },
                        "resolved_package_union": 3,
                        "direct_dependencies": {
                            "runtime": 1,
                            "development": 0,
                            "build": 0,
                        },
                        "duplicate_versions": {},
                    },
                ),
                patch.object(baseline, "locked_package_count", return_value=4),
                patch.object(
                    baseline,
                    "_build_release_binary",
                    return_value={
                        "profile": "release",
                        "bytes": 1,
                        "sha256": "b" * 64,
                    },
                ),
                patch.object(baseline, "_build_benchmark_worker", return_value=worker),
                patch.object(baseline, "_run_benchmarks", return_value=[benchmark]),
            ):
                artifact = baseline._produce_artifact(destination, "smoke", 1, 0, True)

            self.assertEqual(artifact["corpus"]["scale"], "smoke")
            self.assertEqual(
                json.loads(destination.read_text(encoding="utf-8")), artifact
            )

    def test_production_orchestration_rejects_invalid_run_labels(self):
        invalid = (
            ("reference", 1, 5, False),
            ("smoke", 0, 0, True),
            ("smoke", 1, -1, True),
            ("smoke", 10_001, 0, True),
            ("smoke", 1, 10_001, True),
            ("reference", 30, 5, True),
        )
        for evidence_class, samples, warmup, allow_dirty in invalid:
            with self.subTest(
                evidence_class=evidence_class,
                samples=samples,
                warmup=warmup,
                allow_dirty=allow_dirty,
            ):
                with (
                    patch.object(
                        baseline,
                        "_git_source",
                        side_effect=AssertionError("limits must fail before setup"),
                    ),
                    self.assertRaises(ValueError),
                ):
                    baseline._produce_artifact(
                        Path("unused.json"),
                        evidence_class,
                        samples,
                        warmup,
                        allow_dirty,
                    )

    def test_cli_validates_runs_and_reports_failures(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            reference = root / "reference.json"
            baseline.write_artifact(reference, ArtifactValidationTests.valid_artifact())
            with (
                patch.object(baseline, "verify_source_commit"),
                patch("builtins.print"),
            ):
                self.assertEqual(
                    baseline.main(["--validate-artifact", str(reference)]), 0
                )
            with (
                patch.object(baseline, "verify_source_commit"),
                patch.object(
                    baseline,
                    "_sha256_file",
                    side_effect=AssertionError("artifact path was reopened"),
                ),
                patch("builtins.print"),
            ):
                self.assertEqual(
                    baseline.main(["--validate-artifact", str(reference)]), 0
                )

            output = root / "smoke.json"

            def fake_produce(destination, *_arguments, **_keywords):
                destination.write_text("{}\n", encoding="utf-8")
                return {"evidence_class": "smoke", "benchmarks": []}

            with (
                patch.object(baseline, "_produce_artifact", side_effect=fake_produce),
                patch("builtins.print"),
            ):
                self.assertEqual(
                    baseline.main(
                        [
                            "--output",
                            str(output),
                            "--evidence-class",
                            "smoke",
                            "--samples",
                            "1",
                        ]
                    ),
                    0,
                )
            with (
                patch.object(
                    baseline, "_produce_artifact", side_effect=ValueError("bad run")
                ),
                patch("builtins.print"),
            ):
                self.assertEqual(
                    baseline.main(
                        [
                            "--output",
                            str(root / "failed.json"),
                            "--evidence-class",
                            "smoke",
                        ]
                    ),
                    1,
                )


if __name__ == "__main__":
    unittest.main()
