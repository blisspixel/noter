#!/usr/bin/env python3
"""Measure the real bounded Text Mode adapter's headless CPU work only."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import stat
from datetime import UTC, datetime
from pathlib import Path

import run_m1_baseline as baseline

ROOT = Path(__file__).resolve().parents[1]
REFERENCE = ROOT / "docs/evidence/m5-corpus-v1.json"
TEST = "app::measurement::cpu_probe"
PREFIX = "NOTER_M5_CPU="
LIMIT = 8 * baseline.MIB
SCOPE = (
    "Monotonic wall elapsed around headless Text Mode CPU-side work through the actual bounded widget, source "
    "projection, transaction, history, and recovery scheduling seams. No native "
    "input delivery, GPU paint/presentation, screen-reader interoperability, "
    "IME, memory, launch, or expanded file-size acceptance claim."
)


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def select_executable(output: str) -> Path:
    """Require the one optimized Noter binary test executable reported by Cargo."""
    candidates = []
    for line in output.splitlines():
        item = json.loads(line)
        if (
            item.get("reason") == "compiler-artifact"
            and item.get("target", {}).get("name") == "noter"
            and item["target"].get("kind") == ["bin"]
            and item.get("profile", {}).get("test") is True
            and item["profile"].get("opt_level") == "3"
            and item.get("executable")
        ):
            candidates.append(Path(item["executable"]))
    if len(candidates) != 1 or not candidates[0].is_file():
        raise ValueError("Cargo did not report exactly one optimized binary test")
    return candidates[0]


def validate_input(directory: Path, record: dict) -> Path:
    """Read only exact frozen, bounded, ordinary UTF-8 synthetic inputs."""
    if record["bytes"] > LIMIT:
        raise ValueError("CPU baseline cannot bypass the 8 MiB editor limit")
    path = directory / record["name"]
    metadata = path.lstat()
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_size != record["bytes"]:
        raise ValueError("input must be a regular file with the frozen byte size")
    if digest(path) != record["sha256"]:
        raise ValueError("input differs from the frozen synthetic corpus")
    source = path.read_bytes().decode("utf-8")
    marker = baseline.SEARCH_MARKERS["middle"]
    if source.count(marker) != 1:
        raise ValueError("input must contain one synthetic middle marker")
    return path.resolve()


def validate_worker(output: str, case: dict, samples: int, warmup: int) -> dict:
    reports = [
        line.partition(PREFIX)[2] for line in output.splitlines() if PREFIX in line
    ]
    if len(reports) != 1 or output.count(PREFIX) != 1:
        raise ValueError("worker must report exactly one measurement")
    report = json.loads(reports[0])
    if not isinstance(report, dict):
        raise ValueError("worker measurement must be an object")
    expected = {
        "schema": 1,
        "bytes": case["bytes"],
        "wrapped": case["wrapped"],
        "accesskit": case["accesskit"],
        "warmup_count": warmup,
        "sample_count": samples,
    }
    for key, value in expected.items():
        if type(report.get(key)) is not type(value) or report[key] != value:
            raise ValueError(f"worker returned an inconsistent {key}")
    if (
        type(report.get("first_frame_cpu_ns")) is not int
        or report["first_frame_cpu_ns"] <= 0
    ):
        raise ValueError("worker returned an invalid first frame duration")
    for field in (
        "clock_ns",
        "idle_frame_cpu_ns",
        "edit_frame_cpu_ns",
        "idle_accesskit_nodes",
        "edit_accesskit_nodes",
        "edit_revisions",
    ):
        values = report.get(field)
        if (
            not isinstance(values, list)
            or len(values) != samples
            or any(type(value) is not int or value < 0 for value in values)
        ):
            raise ValueError(f"worker returned invalid {field} samples")
    for field in ("idle_frame_cpu_ns", "edit_frame_cpu_ns"):
        report[field.removesuffix("_ns") + "_summary"] = baseline.summarize_nanoseconds(
            report[field]
        )
    if report["edit_revisions"] != list(range(warmup + 1, warmup + samples + 1)):
        raise ValueError("worker did not apply one real revision per input")
    if any(
        (value > 0) != case["accesskit"] for value in report["edit_accesskit_nodes"]
    ):
        raise ValueError("worker accessibility updates do not match the requested mode")
    if not case["accesskit"] and any(report["idle_accesskit_nodes"]):
        raise ValueError("inactive accessibility emitted idle updates")
    return report


def run_case(
    executable: Path, path: Path, case: dict, samples: int, warmup: int, timeout: int
) -> dict:
    settings = {
        "NOTER_M5_CPU_INPUT": str(path),
        "NOTER_M5_CPU_SAMPLES": str(samples),
        "NOTER_M5_CPU_WARMUP": str(warmup),
        "NOTER_M5_CPU_WRAP": str(case["wrapped"]).lower(),
        "NOTER_M5_CPU_ACCESSKIT": str(case["accesskit"]).lower(),
    }
    previous = {key: os.environ.get(key) for key in settings}
    try:
        os.environ.update(settings)
        output = baseline._run_checked(
            [str(executable), TEST, "--exact", "--nocapture", "--test-threads=1"],
            cwd=ROOT,
            timeout=timeout,
        )
    finally:
        for key, value in previous.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value
    return validate_worker(output, case, samples, warmup)


def source_identity() -> dict:
    def git(*arguments: str) -> str:
        return baseline._run_checked(["git", *arguments], cwd=ROOT, timeout=30).strip()

    if git("status", "--porcelain", "--untracked-files=normal"):
        raise ValueError("measurement requires a clean source checkout")
    return {"commit": git("rev-parse", "HEAD"), "tree": git("rev-parse", "HEAD^{tree}")}


def run(args: argparse.Namespace) -> dict:
    source = source_identity()
    reference = json.loads(REFERENCE.read_text(encoding="utf-8"))
    records = [
        record
        for record in reference["files"]
        if record["profile"] in args.profiles and record["bytes"] in args.sizes
    ]
    inputs = [(record, validate_input(args.corpus, record)) for record in records]
    if len(inputs) != len(args.profiles) * len(args.sizes):
        raise ValueError("requested cases do not match the frozen corpus")
    build = [
        "cargo",
        "test",
        "--locked",
        "--release",
        "--bin",
        "noter",
        "--no-run",
        "--message-format=json",
    ]
    executable = select_executable(baseline._run_checked(build, cwd=ROOT, timeout=1800))
    executable_sha256 = digest(executable)
    environment = baseline.collect_environment(args.corpus)
    environment.pop("display_refresh_hz")
    environment["build_profile"] = "release binary test (opt-level 3)"
    environment["ui"] = {
        "backend": "headless egui; no native window or GPU",
        "logical_dimensions": [1200, 760],
        "pixels_per_point": 1,
        "fonts": "Noter bundled Inter and egui defaults; no system fallback discovery",
    }
    receipt = {
        "schema": 1,
        "recorded_at_utc": datetime.now(UTC).isoformat(),
        "scope": SCOPE,
        "source": source,
        "cargo_lock_sha256": digest(ROOT / "Cargo.lock"),
        "build_command": build,
        "executable_sha256": executable_sha256,
        "corpus_sha256": reference["corpus_sha256"],
        "environment": environment,
        "sample_count": args.samples,
        "warmup_count_per_phase": args.warmup,
        "percentile_method": "nearest-rank; no outlier removal",
        "process_state": "fresh worker per case; warmup before each measured in-process phase",
        "os_cache_state": "uncontrolled; no cold-cache claim",
        "status": "complete",
        "attempts": [],
    }
    for record, path in inputs:
        for wrapped in (False, True):
            for accessible in (False, True):
                case = {**record, "wrapped": wrapped, "accesskit": accessible}
                attempt = {"case": case, "status": "complete"}
                receipt["attempts"].append(attempt)
                try:
                    attempt["measurement"] = run_case(
                        executable, path, case, args.samples, args.warmup, args.timeout
                    )
                    validate_input(args.corpus, record)
                except (OSError, ValueError, RuntimeError) as error:
                    attempt.update(status="failed", error=str(error))
                    receipt["status"] = "failed"
                    return receipt
    try:
        if source_identity() != source or digest(executable) != executable_sha256:
            raise ValueError("source or executable changed during measurement")
    except (OSError, ValueError, RuntimeError) as error:
        receipt.update(status="failed", finalization_error=str(error))
    return receipt


def main(arguments: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    reference = json.loads(REFERENCE.read_text(encoding="utf-8"))
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument(
        "--profiles",
        nargs="+",
        choices=sorted({r["profile"] for r in reference["files"]}),
        default=["prose"],
    )
    parser.add_argument(
        "--sizes",
        nargs="+",
        type=int,
        choices=(baseline.MIB, LIMIT),
        default=[baseline.MIB, LIMIT],
    )
    parser.add_argument("--samples", type=int, default=30)
    parser.add_argument("--warmup", type=int, default=5)
    parser.add_argument("--timeout", type=int, default=600)
    args = parser.parse_args(arguments)
    if (
        not 30 <= args.samples <= 100
        or not 0 <= args.warmup <= 100
        or not 1 <= args.timeout <= 1800
    ):
        parser.error(
            "samples must be 30..100, warmup 0..100, and timeout 1..1800 seconds"
        )
    if (
        args.output.exists()
        or len(set(args.profiles)) != len(args.profiles)
        or len(set(args.sizes)) != len(args.sizes)
    ):
        parser.error("output must be new and case selections must be unique")
    try:
        receipt = run(args)
        with args.output.open("x", encoding="utf-8", newline="\n") as output:
            output.write(json.dumps(receipt, indent=2, ensure_ascii=True) + "\n")
    except (OSError, ValueError, RuntimeError) as error:
        parser.exit(1, f"CPU measurement failed: {ascii(str(error))}\n")
    print(f"{len(receipt['attempts'])} attempts; status {receipt['status']}")
    return 0 if receipt["status"] == "complete" else 1


if __name__ == "__main__":
    raise SystemExit(main())
