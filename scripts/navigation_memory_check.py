"""Bound peak process memory for backward word navigation on Linux and Windows.

The existing macOS baseline samples held RSS rather than transient peak memory,
so macOS runs the Rust fixture's semantic checks without this memory assertion.
"""

import json
import platform
import subprocess
import unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import run_m1_baseline as baseline


ROOT = Path(__file__).resolve().parents[1]
MAX_PEAK_BYTES = 160 * 1024 * 1024


@unittest.skipUnless(
    platform.system() in {"Linux", "Windows"},
    "peak process memory measurement is supported on Linux and Windows",
)
class NavigationMemoryTests(unittest.TestCase):
    def test_backward_word_movement_stays_within_bounded_memory(self):
        build = subprocess.run(
            [
                "cargo",
                "build",
                "--locked",
                "--bench",
                "navigation_memory",
                "--message-format=json",
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=True,
            timeout=300,
        )
        artifacts = [
            json.loads(line)
            for line in build.stdout.splitlines()
            if line.startswith("{")
        ]
        executable = next(
            Path(item["executable"])
            for item in artifacts
            if item.get("reason") == "compiler-artifact"
            and item.get("target", {}).get("name") == "navigation_memory"
            and item.get("executable")
        )

        with subprocess.Popen(
            [str(executable), "--hold"],
            cwd=ROOT,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        ) as process:
            self.assertIsNotNone(process.stdin)
            self.assertIsNotNone(process.stdout)
            with ThreadPoolExecutor(max_workers=1) as executor:
                try:
                    ready = executor.submit(process.stdout.readline).result(timeout=30)
                except TimeoutError:
                    process.kill()
                    raise
            try:
                self.assertEqual(ready, "ready\n")
                metric, peak_bytes = baseline._process_memory(process.pid)
                _, stderr = process.communicate("\n", timeout=10)
                self.assertEqual(process.returncode, 0, stderr)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=10)
            print(f"navigation memory: {metric} {peak_bytes} bytes")
            self.assertLess(
                peak_bytes,
                MAX_PEAK_BYTES,
                f"{metric} used {peak_bytes} bytes for a 16 MiB document",
            )


if __name__ == "__main__":
    unittest.main()
