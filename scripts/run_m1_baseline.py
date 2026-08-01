#!/usr/bin/env python3
"""Generate reproducible M1 trust-kernel benchmark evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import re
import signal
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
from concurrent.futures import ThreadPoolExecutor, TimeoutError as FutureTimeout
from contextlib import nullcontext
from datetime import UTC, datetime
from pathlib import Path
from typing import BinaryIO

ROOT = Path(__file__).resolve().parents[1]
MIB = 1024 * 1024
MINIMUM_REFERENCE_SAMPLES = 30
MAXIMUM_SAMPLES = 100
MAXIMUM_WARMUP = 100
MAXIMUM_ARTIFACT_BYTES = 4 * MIB
MAXIMUM_COMMAND_OUTPUT_BYTES = 16 * MIB
WINDOWS_CREATE_SUSPENDED = 0x00000004
WINDOWS_JOB_TERMINATION_TIMEOUT_SECONDS = 30.0
WINDOWS_JOB_POLL_INTERVAL_SECONDS = 0.01
WINDOWS_MAXIMUM_JOB_PROCESSES = 4_096
SUPPORTED_TARGETS = (
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
)
SEARCH_MARKERS = {
    "early": "NOTER-SEARCH-EARLY-7CDA1B9F",
    "middle": "NOTER-SEARCH-MIDDLE-932E8A04",
    "late": "NOTER-SEARCH-LATE-45F016BC",
}


def _validate_windows_process_list_counts(assigned: int, returned: int) -> int:
    """Return a complete bounded Job Object process-list length."""
    if assigned != returned or returned > WINDOWS_MAXIMUM_JOB_PROCESSES:
        raise RuntimeError("Windows command process list is incomplete or invalid")
    return returned


ADVERSARIAL_QUERY = ("a" * 63) + "b"
BENCHMARK_CASES = (
    ("load-empty", True, "Stable-handle load of an empty document"),
    (
        "load-prose-1mib",
        True,
        "Stable-handle load and Rope construction for ordinary prose",
    ),
    (
        "load-mixed-unicode-eol-1mib",
        True,
        "Stable-handle load of mixed Unicode and line endings",
    ),
    ("load-newline-1mib", True, "Stable-handle load of newline-only input"),
    ("load-long-line-1mib", True, "Stable-handle load of one pathological long line"),
    ("load-source-50mib", True, "Stable-handle trust-kernel load of source-like input"),
    ("load-log-50mib", True, "Stable-handle trust-kernel load of log-like input"),
    ("search-early-50mib", False, "Literal search with one early match"),
    ("search-middle-50mib", False, "Literal search with one middle match"),
    ("search-late-50mib", False, "Literal search with one late match"),
    ("search-absent-50mib", False, "Literal search with no match"),
    (
        "search-adversarial-50mib",
        False,
        "Linear literal search over repeated near matches",
    ),
    (
        "serialize-prose-1mib",
        False,
        "Exact document serialization including preserved metadata",
    ),
    (
        "save-new-prose-1mib",
        False,
        "Verified exclusive new-file save with persistence barriers",
    ),
    (
        "save-replace-prose-1mib",
        False,
        "Verified atomic existing-file replacement with persistence barriers",
    ),
)
REFERENCE_CORPUS_FILES = (
    (
        "empty.txt",
        0,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "Empty UTF-8 document",
    ),
    (
        "prose-1mib.txt",
        MIB,
        "4bcd49f5d09912db6e2d1d2bfe253ace2bd1167f42106f7c48765caa3bb04020",
        "Ordinary prose with CRLF",
    ),
    (
        "mixed-unicode-eol-1mib.txt",
        MIB,
        "6c1e4a69e8549101dfbcd6d30e64bf1b7e17f85cf106ad22540122c2d3a4382d",
        "Mixed Unicode normalization forms and LF, CRLF, and CR",
    ),
    (
        "newline-1mib.txt",
        MIB,
        "b3a2d81c390e0531dbcf0dec082c4ca96d26f26aa9a26a26e80b5f00fa9f48e3",
        "Newline-only input",
    ),
    (
        "long-line-1mib.txt",
        MIB,
        "8f990ba0b577b51cf009ea049368c16bbda1b21e1b93be07a824758bb253c39b",
        "One pathological long line",
    ),
    (
        "source-large.txt",
        50 * MIB,
        "638d87a447a1db3bd906f22a0ce600ca26f60b24cac3fe99f3e54e0f3104a25d",
        "Source-like large document",
    ),
    (
        "log-large.txt",
        50 * MIB,
        "d88c001941130af0b09c1d9ab6bbb87ede680db22825ccfd879fa8f36edc2342",
        "Log-like repeated near-match document",
    ),
)
REFERENCE_CORPUS_SHA256 = (
    "62f09d6fe9972e1ca36d66142fdfca0e1bfdcdeac1697da117b536a6a0815016"
)
ADVERSARIAL_QUERY_SHA256 = (
    "97aa7c540da474936ff8bedd71acb8a59ff1d41b71fa52c0f4680a8e17b16ad6"
)
SCOPE_CLAIM = (
    "Baseline only; GUI launch, frame, IME, accessibility, and 50 MiB edit "
    "requirements are not verified."
)
TIMING_STATES = {
    "process-cold": (
        "One measured operation in each fresh worker; operating-system file "
        "cache is uncontrolled."
    ),
    "warm-in-process": "Measured after explicit in-process warmup in one worker.",
}
PROVENANCE = {
    "kind": "self-reported-local-run",
    "authentication": "none",
    "claim": (
        "The artifact is internally reproducible evidence from the named system, "
        "not cryptographically authenticated telemetry. Exact-head CI can validate "
        "the committed bytes and workflow, not the historical timing event."
    ),
}
HEX_40 = re.compile(r"[0-9a-f]{40}\Z")
HEX_64 = re.compile(r"[0-9a-f]{64}\Z")


def nearest_rank(samples: list[int], percentile: float) -> int:
    """Return the nearest-rank percentile from positive integer samples."""
    if not samples or not 0.0 < percentile <= 1.0:
        raise ValueError(
            "percentiles require non-empty samples and a probability in (0, 1]"
        )
    if any(
        isinstance(sample, bool) or not isinstance(sample, int) or sample <= 0
        for sample in samples
    ):
        raise ValueError("samples must be positive integers")
    ordered = sorted(samples)
    rank = math.ceil(percentile * len(ordered))
    return ordered[rank - 1]


def summarize_nanoseconds(samples: list[int]) -> dict[str, int]:
    """Summarize raw nanosecond observations without discarding outliers."""
    return {
        "minimum_ns": min(samples),
        "p50_ns": nearest_rank(samples, 0.50),
        "p95_ns": nearest_rank(samples, 0.95),
        "p99_ns": nearest_rank(samples, 0.99),
        "maximum_ns": max(samples),
    }


def corpus_sizes(scale: str) -> tuple[int, int]:
    """Return the ordinary and large corpus sizes for one evidence class."""
    if scale == "full":
        return MIB, 50 * MIB
    if scale == "smoke":
        return 64 * 1024, MIB
    raise ValueError(f"unsupported corpus scale: {scale}")


def write_repeated_file(path: Path, pattern: bytes, size: int) -> None:
    """Create one exact-size file by repeating a non-empty byte pattern."""
    if not pattern:
        raise ValueError("corpus patterns must not be empty")
    if size < 0:
        raise ValueError("corpus sizes must not be negative")
    chunk = (pattern * max(1, MIB // len(pattern) + 1))[:MIB]
    remaining = size
    with path.open("xb") as output:
        while remaining:
            current = chunk[: min(remaining, len(chunk))]
            output.write(current)
            remaining -= len(current)


def _patch_ascii(path: Path, offset: int, text: str) -> None:
    encoded = text.encode("ascii")
    size = path.stat().st_size
    if offset < 0 or offset + len(encoded) > size:
        raise ValueError("corpus marker does not fit at the requested offset")
    with path.open("r+b") as output:
        output.seek(offset)
        output.write(encoded)


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(MIB), b""):
            digest.update(chunk)
    return digest.hexdigest()


def corpus_manifest_digest(files: list[dict[str, object]]) -> str:
    """Hash the canonical ordered file manifest."""
    encoded = json.dumps(
        files, ensure_ascii=True, separators=(",", ":"), sort_keys=True
    ).encode()
    return hashlib.sha256(encoded).hexdigest()


def reference_corpus_files() -> list[dict[str, object]]:
    """Return a fresh copy of the exact full-corpus file contract."""
    return [
        {"name": name, "bytes": size, "sha256": digest, "description": description}
        for name, size, digest, description in REFERENCE_CORPUS_FILES
    ]


def evidence_scope() -> dict[str, object]:
    """Return the exact scope and limitations attached to every baseline."""
    return {
        "kind": "ui-independent-trust-kernel-baseline",
        "claim": SCOPE_CLAIM,
        "smoke_scaling": (
            "Smoke artifacts retain canonical 50 MiB case identifiers but use the "
            "smaller exact sizes recorded in their corpus manifest."
        ),
        "timing_states": dict(TIMING_STATES),
        "cases": [
            {"name": case, "description": description}
            for case, _, description in BENCHMARK_CASES
        ],
    }


def generate_corpus(directory: Path, scale: str) -> dict[str, object]:
    """Generate the deterministic M1 corpus into an empty directory."""
    ordinary_size, large_size = corpus_sizes(scale)
    directory.mkdir(parents=True, exist_ok=True)
    if any(directory.iterdir()):
        raise FileExistsError("corpus directory must be empty")

    prose = (
        b"Noter keeps ordinary text authoritative, private, and portable. "
        b"Every save preserves explicit source bytes and line endings.\r\n"
    )
    mixed_core = (
        "ASCII|naive|cafe\u0301|\u6771\u4eac|\U0001f642\r\nnext\nlegacy\r".encode()
    )
    mixed = mixed_core + (b"x" * (64 - len(mixed_core)))
    source = (
        b"fn compute_record(value: u64) -> u64 { value.rotate_left(7) ^ 0x9e3779b9 }\n"
    )
    log = (("a" * 63) + "c 2026-07-31T00:00:00Z INFO durable checkpoint\n").encode()
    specifications = (
        ("empty.txt", b"0", 0, "Empty UTF-8 document"),
        ("prose-1mib.txt", prose, ordinary_size, "Ordinary prose with CRLF"),
        (
            "mixed-unicode-eol-1mib.txt",
            mixed,
            ordinary_size,
            "Mixed Unicode normalization forms and LF, CRLF, and CR",
        ),
        ("newline-1mib.txt", b"\n", ordinary_size, "Newline-only input"),
        ("long-line-1mib.txt", b"x", ordinary_size, "One pathological long line"),
        ("source-large.txt", source, large_size, "Source-like large document"),
        ("log-large.txt", log, large_size, "Log-like repeated near-match document"),
    )
    records: list[dict[str, object]] = []
    for name, pattern, size, description in specifications:
        path = directory / name
        write_repeated_file(path, pattern, size)
        records.append(
            {
                "name": name,
                "bytes": size,
                "sha256": _sha256_file(path),
                "description": description,
            }
        )

    source_path = directory / "source-large.txt"
    offsets = {
        "early": min(4096, large_size // 8),
        "middle": large_size // 2,
        "late": large_size - 4096,
    }
    for location, marker in SEARCH_MARKERS.items():
        _patch_ascii(source_path, offsets[location], marker)
    source_record = next(
        record for record in records if record["name"] == source_path.name
    )
    source_record["sha256"] = _sha256_file(source_path)

    return {
        "generator_version": 1,
        "scale": scale,
        "files": records,
        "corpus_sha256": corpus_manifest_digest(records),
        "search_markers": dict(SEARCH_MARKERS),
        "adversarial_query_sha256": hashlib.sha256(
            ADVERSARIAL_QUERY.encode()
        ).hexdigest(),
    }


def parse_worker_result(
    payload: str,
    expected_case: str,
    expected_samples: int,
    expected_warmup: int | None = None,
) -> dict[str, object]:
    """Validate one benchmark worker result before accepting its timings."""
    try:
        result = json.loads(payload)
    except json.JSONDecodeError as error:
        raise ValueError("benchmark worker returned malformed JSON") from error
    if not isinstance(result, dict) or result.get("case") != expected_case:
        raise ValueError("benchmark worker returned an unexpected case")
    if (
        not isinstance(result.get("warmup"), int)
        or result["warmup"] < 0
        or (expected_warmup is not None and result["warmup"] != expected_warmup)
    ):
        raise ValueError("benchmark worker returned an unexpected warmup count")
    samples = result.get("samples_ns")
    if not isinstance(samples, list) or len(samples) != expected_samples:
        raise ValueError("benchmark worker returned an unexpected sample count")
    nearest_rank(samples, 1.0)
    checksum = result.get("checksum")
    if isinstance(checksum, bool) or not isinstance(checksum, int) or checksum < 0:
        raise ValueError("benchmark worker returned an invalid checksum")
    return result


def parse_linux_peak_rss(status: str) -> int:
    """Parse Linux VmHWM, whose documented unit is KiB."""
    match = re.search(r"^VmHWM:\s+(\d+)\s+kB$", status, flags=re.MULTILINE)
    if not match:
        raise ValueError("Linux process status does not contain VmHWM in KiB")
    return int(match.group(1)) * 1024


def parse_findmnt(output: str) -> dict[str, str]:
    """Classify a findmnt filesystem result without publishing device names."""
    fields = output.strip().split(maxsplit=1)
    if not fields:
        raise ValueError("findmnt returned no filesystem type")
    filesystem = fields[0]
    source = fields[1] if len(fields) > 1 else ""
    if filesystem.lower() in {"9p", "drvfs", "virtiofs"}:
        source_class = "host-bridge"
    elif source.startswith("/dev/"):
        source_class = "block-device"
    elif source.startswith("//") or filesystem.lower() in {"cifs", "nfs", "smbfs"}:
        source_class = "network"
    else:
        source_class = "other"
    return {"type": filesystem, "source_class": source_class}


def summarize_dependencies(
    metadata_by_target: dict[str, dict[str, object]], root_package_id: str
) -> dict[str, object]:
    """Summarize the union of all declared release-target dependency graphs."""
    packages_by_id: dict[str, dict[str, object]] = {}
    counts: dict[str, int] = {}
    root_package: dict[str, object] | None = None
    for target, metadata in sorted(metadata_by_target.items()):
        packages = metadata.get("packages")
        if not isinstance(packages, list):
            raise ValueError(f"cargo metadata for {target} has no package list")
        counts[target] = len(packages)
        for package in packages:
            if not isinstance(package, dict) or not isinstance(package.get("id"), str):
                raise ValueError(
                    f"cargo metadata for {target} contains an invalid package"
                )
            packages_by_id[package["id"]] = package
            if package["id"] == root_package_id:
                root_package = package
    if root_package is None:
        raise ValueError("cargo metadata does not contain the Noter root package")

    direct_names = {"runtime": set(), "development": set(), "build": set()}
    dependencies = root_package.get("dependencies")
    if not isinstance(dependencies, list):
        raise ValueError("Noter package metadata has no dependency list")
    for dependency in dependencies:
        kind = dependency.get("kind")
        category = (
            "development"
            if kind == "dev"
            else "build"
            if kind == "build"
            else "runtime"
        )
        direct_names[category].add(dependency["name"])

    versions_by_name: dict[str, set[str]] = {}
    for package in packages_by_id.values():
        name = package.get("name")
        version = package.get("version")
        if isinstance(name, str) and isinstance(version, str):
            versions_by_name.setdefault(name, set()).add(version)
    duplicates = {
        name: sorted(versions)
        for name, versions in sorted(versions_by_name.items())
        if len(versions) > 1
    }
    return {
        "release_targets": sorted(metadata_by_target),
        "resolved_packages_by_target": counts,
        "resolved_package_union": len(packages_by_id),
        "direct_dependencies": {key: len(value) for key, value in direct_names.items()},
        "duplicate_versions": duplicates,
    }


def locked_package_count(lock_path: Path) -> int:
    """Count exact package records in the committed Cargo lockfile."""
    with lock_path.open("rb") as lock_file:
        parsed = tomllib.load(lock_file)
    packages = parsed.get("package")
    if not isinstance(packages, list) or not packages:
        raise ValueError("Cargo.lock contains no package records")
    return len(packages)


class _WindowsProcessJob:
    """Own a Win32 job that kills every assigned process when closed."""

    def __init__(self) -> None:
        import ctypes
        from ctypes import wintypes

        class BasicLimitInformation(ctypes.Structure):
            _fields_ = [
                ("per_process_user_time_limit", ctypes.c_longlong),
                ("per_job_user_time_limit", ctypes.c_longlong),
                ("limit_flags", wintypes.DWORD),
                ("minimum_working_set_size", ctypes.c_size_t),
                ("maximum_working_set_size", ctypes.c_size_t),
                ("active_process_limit", wintypes.DWORD),
                ("affinity", ctypes.c_size_t),
                ("priority_class", wintypes.DWORD),
                ("scheduling_class", wintypes.DWORD),
            ]

        class IoCounters(ctypes.Structure):
            _fields_ = [
                ("read_operation_count", ctypes.c_ulonglong),
                ("write_operation_count", ctypes.c_ulonglong),
                ("other_operation_count", ctypes.c_ulonglong),
                ("read_transfer_count", ctypes.c_ulonglong),
                ("write_transfer_count", ctypes.c_ulonglong),
                ("other_transfer_count", ctypes.c_ulonglong),
            ]

        class ExtendedLimitInformation(ctypes.Structure):
            _fields_ = [
                ("basic_limit_information", BasicLimitInformation),
                ("io_info", IoCounters),
                ("process_memory_limit", ctypes.c_size_t),
                ("job_memory_limit", ctypes.c_size_t),
                ("peak_process_memory_used", ctypes.c_size_t),
                ("peak_job_memory_used", ctypes.c_size_t),
            ]

        class BasicAccountingInformation(ctypes.Structure):
            _fields_ = [
                ("total_user_time", ctypes.c_longlong),
                ("total_kernel_time", ctypes.c_longlong),
                ("this_period_total_user_time", ctypes.c_longlong),
                ("this_period_total_kernel_time", ctypes.c_longlong),
                ("total_page_fault_count", wintypes.DWORD),
                ("total_processes", wintypes.DWORD),
                ("active_processes", wintypes.DWORD),
                ("total_terminated_processes", wintypes.DWORD),
            ]

        class BasicProcessIdList(ctypes.Structure):
            _fields_ = [
                ("number_of_assigned_processes", wintypes.DWORD),
                ("number_of_process_ids_in_list", wintypes.DWORD),
                (
                    "process_id_list",
                    ctypes.c_size_t * WINDOWS_MAXIMUM_JOB_PROCESSES,
                ),
            ]

        class ThreadEntry(ctypes.Structure):
            _fields_ = [
                ("size", wintypes.DWORD),
                ("usage_count", wintypes.DWORD),
                ("thread_id", wintypes.DWORD),
                ("owner_process_id", wintypes.DWORD),
                ("base_priority", wintypes.LONG),
                ("priority_delta", wintypes.LONG),
                ("flags", wintypes.DWORD),
            ]

        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel32.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
        kernel32.CreateJobObjectW.restype = wintypes.HANDLE
        kernel32.SetInformationJobObject.argtypes = [
            wintypes.HANDLE,
            ctypes.c_int,
            ctypes.c_void_p,
            wintypes.DWORD,
        ]
        kernel32.SetInformationJobObject.restype = wintypes.BOOL
        kernel32.QueryInformationJobObject.argtypes = [
            wintypes.HANDLE,
            ctypes.c_int,
            ctypes.c_void_p,
            wintypes.DWORD,
            ctypes.POINTER(wintypes.DWORD),
        ]
        kernel32.QueryInformationJobObject.restype = wintypes.BOOL
        kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel32.OpenProcess.restype = wintypes.HANDLE
        kernel32.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
        kernel32.AssignProcessToJobObject.restype = wintypes.BOOL
        kernel32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
        kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
        kernel32.Thread32First.argtypes = [
            wintypes.HANDLE,
            ctypes.POINTER(ThreadEntry),
        ]
        kernel32.Thread32First.restype = wintypes.BOOL
        kernel32.Thread32Next.argtypes = [
            wintypes.HANDLE,
            ctypes.POINTER(ThreadEntry),
        ]
        kernel32.Thread32Next.restype = wintypes.BOOL
        kernel32.OpenThread.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel32.OpenThread.restype = wintypes.HANDLE
        kernel32.ResumeThread.argtypes = [wintypes.HANDLE]
        kernel32.ResumeThread.restype = wintypes.DWORD
        kernel32.TerminateJobObject.argtypes = [wintypes.HANDLE, wintypes.UINT]
        kernel32.TerminateJobObject.restype = wintypes.BOOL
        kernel32.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
        kernel32.TerminateProcess.restype = wintypes.BOOL
        kernel32.IsProcessInJob.argtypes = [
            wintypes.HANDLE,
            wintypes.HANDLE,
            ctypes.POINTER(wintypes.BOOL),
        ]
        kernel32.IsProcessInJob.restype = wintypes.BOOL
        kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        kernel32.WaitForSingleObject.restype = wintypes.DWORD
        kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel32.CloseHandle.restype = wintypes.BOOL

        self._ctypes = ctypes
        self._kernel32 = kernel32
        self._accounting_type = BasicAccountingInformation
        self._bool_type = wintypes.BOOL
        self._process_id_list_type = BasicProcessIdList
        self._thread_entry_type = ThreadEntry
        self._handle = kernel32.CreateJobObjectW(None, None)
        if not self._handle:
            raise OSError(ctypes.get_last_error(), "CreateJobObjectW failed")

        information = ExtendedLimitInformation()
        information.basic_limit_information.limit_flags = 0x00002000
        if not kernel32.SetInformationJobObject(
            self._handle,
            9,
            ctypes.byref(information),
            ctypes.sizeof(information),
        ):
            error = ctypes.get_last_error()
            kernel32.CloseHandle(self._handle)
            self._handle = None
            raise OSError(error, "SetInformationJobObject failed")

    def __enter__(self) -> _WindowsProcessJob:
        return self

    def __exit__(self, *_exception: object) -> None:
        self.close()

    def activate(self, process: subprocess.Popen[bytes]) -> None:
        """Assign a newly suspended process to this job, then run it."""
        process_set_quota = 0x0100
        process_terminate = 0x0001
        process_handle = self._kernel32.OpenProcess(
            process_set_quota | process_terminate,
            False,
            process.pid,
        )
        if not process_handle:
            raise OSError(self._ctypes.get_last_error(), "OpenProcess failed")
        try:
            if not self._kernel32.AssignProcessToJobObject(
                self._handle, process_handle
            ):
                raise OSError(
                    self._ctypes.get_last_error(),
                    "AssignProcessToJobObject failed",
                )
        finally:
            if not self._kernel32.CloseHandle(process_handle):
                raise OSError(self._ctypes.get_last_error(), "CloseHandle failed")
        self._resume_suspended_process(process.pid)

    def _resume_suspended_process(self, process_id: int) -> None:
        snapshot_threads = 0x00000004
        thread_suspend_resume = 0x0002
        invalid_handle = self._ctypes.c_void_p(-1).value
        no_more_files = 18
        resume_failed = 0xFFFFFFFF
        snapshot = self._kernel32.CreateToolhelp32Snapshot(snapshot_threads, 0)
        if snapshot == invalid_handle:
            raise OSError(
                self._ctypes.get_last_error(),
                "CreateToolhelp32Snapshot failed",
            )
        resumed_threads = 0
        try:
            entry = self._thread_entry_type()
            entry.size = self._ctypes.sizeof(entry)
            if not self._kernel32.Thread32First(snapshot, self._ctypes.byref(entry)):
                raise OSError(self._ctypes.get_last_error(), "Thread32First failed")
            while True:
                if entry.owner_process_id == process_id:
                    thread = self._kernel32.OpenThread(
                        thread_suspend_resume,
                        False,
                        entry.thread_id,
                    )
                    if not thread:
                        raise OSError(
                            self._ctypes.get_last_error(),
                            "OpenThread failed",
                        )
                    try:
                        previous_count = self._kernel32.ResumeThread(thread)
                        if previous_count == resume_failed:
                            raise OSError(
                                self._ctypes.get_last_error(),
                                "ResumeThread failed",
                            )
                        if previous_count == 0:
                            raise RuntimeError(
                                "new Windows command was not created suspended"
                            )
                        resumed_threads += 1
                    finally:
                        if not self._kernel32.CloseHandle(thread):
                            raise OSError(
                                self._ctypes.get_last_error(),
                                "CloseHandle failed",
                            )
                if self._kernel32.Thread32Next(snapshot, self._ctypes.byref(entry)):
                    continue
                error = self._ctypes.get_last_error()
                if error != no_more_files:
                    raise OSError(error, "Thread32Next failed")
                break
        finally:
            if not self._kernel32.CloseHandle(snapshot):
                raise OSError(self._ctypes.get_last_error(), "CloseHandle failed")
        if resumed_threads != 1:
            raise RuntimeError(
                "new Windows command did not expose exactly one primary thread"
            )

    def _active_process_count(self) -> int:
        information = self._accounting_type()
        if not self._kernel32.QueryInformationJobObject(
            self._handle,
            1,
            self._ctypes.byref(information),
            self._ctypes.sizeof(information),
            None,
        ):
            raise OSError(
                self._ctypes.get_last_error(),
                "QueryInformationJobObject failed",
            )
        return information.active_processes

    def _open_process_handles(self) -> tuple[int, list[int]]:
        information = self._process_id_list_type()
        if not self._kernel32.QueryInformationJobObject(
            self._handle,
            3,
            self._ctypes.byref(information),
            self._ctypes.sizeof(information),
            None,
        ):
            raise OSError(
                self._ctypes.get_last_error(),
                "QueryInformationJobObject process list failed",
            )
        process_count = _validate_windows_process_list_counts(
            information.number_of_assigned_processes,
            information.number_of_process_ids_in_list,
        )

        process_query_limited_information = 0x1000
        process_terminate = 0x0001
        synchronize = 0x00100000
        handles = []
        try:
            for process_id in information.process_id_list[:process_count]:
                process_handle = self._kernel32.OpenProcess(
                    synchronize | process_query_limited_information | process_terminate,
                    False,
                    process_id,
                )
                if not process_handle:
                    error = self._ctypes.get_last_error()
                    if error == 87:
                        continue
                    raise OSError(error, "OpenProcess for shutdown wait failed")
                handles.append(process_handle)
                in_job = self._bool_type()
                if not self._kernel32.IsProcessInJob(
                    process_handle,
                    self._handle,
                    self._ctypes.byref(in_job),
                ):
                    raise OSError(
                        self._ctypes.get_last_error(),
                        "IsProcessInJob failed",
                    )
                if in_job.value:
                    continue
                handles.pop()
                if not self._kernel32.CloseHandle(process_handle):
                    raise OSError(
                        self._ctypes.get_last_error(),
                        "CloseHandle failed",
                    )
            return process_count, handles
        except (OSError, RuntimeError) as error:
            close_failure = self._close_process_handles(handles)
            if close_failure is not None:
                error.add_note(str(close_failure))
            raise

    def _close_process_handles(self, handles: list[int]) -> OSError | None:
        first_error = None
        for process_handle in handles:
            if not self._kernel32.CloseHandle(process_handle) and first_error is None:
                first_error = OSError(
                    self._ctypes.get_last_error(),
                    "CloseHandle failed",
                )
        return first_error

    def _terminate_process_handles(self, handles: list[int]) -> None:
        wait_object_0 = 0
        for process_handle in handles:
            if self._kernel32.TerminateProcess(process_handle, 1):
                continue
            error = self._ctypes.get_last_error()
            if self._kernel32.WaitForSingleObject(process_handle, 0) == wait_object_0:
                continue
            raise OSError(error, "TerminateProcess failed")

    def _wait_for_process_handles(self, handles: list[int], deadline: float) -> None:
        wait_object_0 = 0
        wait_failed = 0xFFFFFFFF
        for process_handle in handles:
            remaining = deadline - time.monotonic()
            timeout_ms = max(0, math.ceil(remaining * 1_000))
            result = self._kernel32.WaitForSingleObject(process_handle, timeout_ms)
            if result == wait_object_0:
                continue
            if result == wait_failed:
                raise OSError(
                    self._ctypes.get_last_error(),
                    "WaitForSingleObject failed",
                )
            raise RuntimeError(
                "Windows command descendants did not terminate within "
                f"{WINDOWS_JOB_TERMINATION_TIMEOUT_SECONDS:g} seconds"
            )

    def terminate(self) -> None:
        """Terminate every assigned process and wait for complete shutdown."""
        unsettled_handles = []
        capture_failure = None
        deadline = time.monotonic() + WINDOWS_JOB_TERMINATION_TIMEOUT_SECONDS
        if self._handle:
            try:
                while True:
                    if time.monotonic() >= deadline:
                        raise RuntimeError(
                            "Windows command process capture did not converge within "
                            f"{WINDOWS_JOB_TERMINATION_TIMEOUT_SECONDS:g} seconds"
                        )
                    member_count, batch = self._open_process_handles()
                    if member_count == 0:
                        break
                    if not batch:
                        continue
                    unsettled_handles = batch
                    self._terminate_process_handles(batch)
                    self._wait_for_process_handles(batch, deadline)
                    close_failure = self._close_process_handles(batch)
                    unsettled_handles = []
                    if close_failure is not None:
                        raise close_failure
            except (OSError, RuntimeError) as error:
                capture_failure = error
        failure = None
        try:
            if self._handle and not self._kernel32.TerminateJobObject(self._handle, 1):
                raise OSError(
                    self._ctypes.get_last_error(),
                    "TerminateJobObject failed",
                )
            self._wait_for_process_handles(unsettled_handles, deadline)
            while self._handle and self._active_process_count() != 0:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise RuntimeError(
                        "Windows command descendants did not terminate within "
                        f"{WINDOWS_JOB_TERMINATION_TIMEOUT_SECONDS:g} seconds"
                    )
                time.sleep(min(WINDOWS_JOB_POLL_INTERVAL_SECONDS, remaining))
            if capture_failure is not None:
                raise RuntimeError(
                    "Windows command process handles could not be retained"
                ) from capture_failure
        except (OSError, RuntimeError) as error:
            failure = error
            raise
        finally:
            close_failure = self._close_process_handles(unsettled_handles)
            if close_failure is not None and failure is None:
                raise close_failure

    def close(self) -> None:
        if not self._handle:
            return
        handle = self._handle
        self._handle = None
        if not self._kernel32.CloseHandle(handle):
            raise OSError(self._ctypes.get_last_error(), "CloseHandle failed")


def _terminate_process_tree(
    process: subprocess.Popen[bytes],
    windows_job: _WindowsProcessJob | None = None,
) -> None:
    """Terminate a command and its ordinary descendants."""
    if windows_job is not None:
        windows_job.terminate()
    elif os.name == "nt":
        try:
            subprocess.run(
                ["taskkill", "/PID", str(process.pid), "/T", "/F"],
                check=False,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=30,
            )
        except subprocess.TimeoutExpired as error:
            process.kill()
            process.wait(timeout=30)
            raise RuntimeError(
                "command process tree could not be terminated"
            ) from error
    else:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            return
    if process.poll() is None:
        process.kill()
    try:
        process.wait(timeout=30)
    except subprocess.TimeoutExpired as error:
        raise RuntimeError("command could not be terminated") from error


def _decode_command_output(payload: bytes, command: str, stream: str) -> str:
    if len(payload) > MAXIMUM_COMMAND_OUTPUT_BYTES:
        raise RuntimeError(f"{command} {stream} exceeded the bounded output limit")
    try:
        return payload.decode("utf-8").replace("\r\n", "\n").replace("\r", "\n")
    except UnicodeDecodeError as error:
        raise RuntimeError(f"{command} emitted non-UTF-8 {stream}") from error


def _capture_bounded_stream(
    stream: BinaryIO,
    output: bytearray,
    limit_exceeded: threading.Event,
) -> None:
    """Drain one child stream without allowing its captured output to grow."""
    try:
        while not limit_exceeded.is_set():
            remaining = MAXIMUM_COMMAND_OUTPUT_BYTES + 1 - len(output)
            if remaining <= 0:
                limit_exceeded.set()
                return
            chunk = stream.read(min(64 * 1024, remaining))
            if not chunk:
                return
            output.extend(chunk)
            if len(output) > MAXIMUM_COMMAND_OUTPUT_BYTES:
                limit_exceeded.set()
                return
    finally:
        stream.close()


def _run_checked(arguments: list[str], *, cwd: Path = ROOT, timeout: int = 600) -> str:
    creation_flags = (
        subprocess.CREATE_NEW_PROCESS_GROUP | WINDOWS_CREATE_SUSPENDED
        if os.name == "nt"
        else 0
    )
    job_context = _WindowsProcessJob() if os.name == "nt" else nullcontext()
    with job_context as windows_job:
        process = subprocess.Popen(
            arguments,
            cwd=cwd,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            creationflags=creation_flags,
            start_new_session=os.name != "nt",
        )
        if windows_job is not None:
            try:
                windows_job.activate(process)
            except (OSError, RuntimeError):
                _terminate_process_tree(process, windows_job)
                raise
        if process.stdout is None or process.stderr is None:
            _terminate_process_tree(process, windows_job)
            raise RuntimeError(f"{arguments[0]} output pipes were not created")

        stdout_payload = bytearray()
        stderr_payload = bytearray()
        limit_exceeded = threading.Event()
        capture_threads = (
            threading.Thread(
                target=_capture_bounded_stream,
                args=(process.stdout, stdout_payload, limit_exceeded),
                daemon=True,
            ),
            threading.Thread(
                target=_capture_bounded_stream,
                args=(process.stderr, stderr_payload, limit_exceeded),
                daemon=True,
            ),
        )
        for thread in capture_threads:
            thread.start()

        deadline = time.monotonic() + timeout
        timed_out = False
        while process.poll() is None or any(
            thread.is_alive() for thread in capture_threads
        ):
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                timed_out = True
                break
            if limit_exceeded.wait(timeout=min(0.05, remaining)):
                break

        if limit_exceeded.is_set() or timed_out:
            try:
                _terminate_process_tree(process, windows_job)
            finally:
                for thread in capture_threads:
                    thread.join(timeout=5)
            if limit_exceeded.is_set():
                stream = (
                    "stdout"
                    if len(stdout_payload) > MAXIMUM_COMMAND_OUTPUT_BYTES
                    else "stderr"
                )
                raise RuntimeError(
                    f"{arguments[0]} {stream} exceeded the bounded output limit"
                )
            raise RuntimeError(f"{arguments[0]} exceeded its {timeout}-second deadline")

        for thread in capture_threads:
            thread.join()
        return_code = process.wait()
        stdout = _decode_command_output(bytes(stdout_payload), arguments[0], "stdout")
        stderr = _decode_command_output(bytes(stderr_payload), arguments[0], "stderr")
        if return_code != 0:
            detail = stderr.strip().splitlines()
            suffix = detail[-1] if detail else "no diagnostic"
            raise RuntimeError(
                f"{arguments[0]} failed with exit {return_code}: {suffix}"
            )
        return stdout


def _cargo_metadata() -> tuple[dict[str, dict[str, object]], str]:
    metadata_by_target: dict[str, dict[str, object]] = {}
    root_id = ""
    for target in SUPPORTED_TARGETS:
        raw = _run_checked(
            [
                "cargo",
                "metadata",
                "--locked",
                "--all-features",
                "--format-version",
                "1",
                "--filter-platform",
                target,
            ]
        )
        metadata = json.loads(raw)
        metadata_by_target[target] = metadata
        for package in metadata["packages"]:
            if (
                package["name"] == "noter"
                and Path(package["manifest_path"]).resolve() == ROOT / "Cargo.toml"
            ):
                root_id = package["id"]
    if not root_id:
        raise RuntimeError("could not identify the Noter package in cargo metadata")
    return metadata_by_target, root_id


def _build_release_binary() -> dict[str, object]:
    _run_checked(["cargo", "build", "--release", "--locked", "--bin", "noter"])
    metadata = json.loads(
        _run_checked(
            ["cargo", "metadata", "--locked", "--format-version", "1", "--no-deps"]
        )
    )
    suffix = ".exe" if platform.system() == "Windows" else ""
    binary = Path(metadata["target_directory"]) / "release" / f"noter{suffix}"
    if not binary.is_file():
        raise RuntimeError("release build did not produce the expected Noter binary")
    return {
        "profile": "release",
        "bytes": binary.stat().st_size,
        "sha256": _sha256_file(binary),
    }


def _build_benchmark_worker() -> Path:
    output = _run_checked(
        [
            "cargo",
            "bench",
            "--locked",
            "--bench",
            "m1_baseline",
            "--no-run",
            "--message-format=json",
        ],
        timeout=900,
    )
    executable: Path | None = None
    for line in output.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        target = message.get("target", {})
        candidate = message.get("executable")
        if (
            message.get("reason") == "compiler-artifact"
            and target.get("name") == "m1_baseline"
            and candidate
        ):
            executable = Path(candidate)
    if executable is None or not executable.is_file():
        raise RuntimeError("Cargo did not report the M1 benchmark executable")
    listed = json.loads(_run_checked([str(executable), "--list"], timeout=30))
    expected = [case for case, _, _ in BENCHMARK_CASES]
    if listed != expected:
        raise RuntimeError(
            "benchmark worker cases do not match the evidence orchestrator"
        )
    return executable


def _windows_peak_working_set(process_id: int) -> int:
    import ctypes
    from ctypes import wintypes

    class ProcessMemoryCounters(ctypes.Structure):
        _fields_ = [
            ("cb", wintypes.DWORD),
            ("PageFaultCount", wintypes.DWORD),
            ("PeakWorkingSetSize", ctypes.c_size_t),
            ("WorkingSetSize", ctypes.c_size_t),
            ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
            ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
            ("PagefileUsage", ctypes.c_size_t),
            ("PeakPagefileUsage", ctypes.c_size_t),
        ]

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    psapi = ctypes.WinDLL("psapi", use_last_error=True)
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL
    psapi.GetProcessMemoryInfo.argtypes = [
        wintypes.HANDLE,
        ctypes.POINTER(ProcessMemoryCounters),
        wintypes.DWORD,
    ]
    psapi.GetProcessMemoryInfo.restype = wintypes.BOOL

    process = kernel32.OpenProcess(0x0400 | 0x0010, False, process_id)
    if not process:
        raise OSError(ctypes.get_last_error(), "OpenProcess failed")
    try:
        counters = ProcessMemoryCounters()
        counters.cb = ctypes.sizeof(counters)
        if not psapi.GetProcessMemoryInfo(process, ctypes.byref(counters), counters.cb):
            raise OSError(ctypes.get_last_error(), "GetProcessMemoryInfo failed")
        return int(counters.PeakWorkingSetSize)
    finally:
        kernel32.CloseHandle(process)


def _process_memory(process_id: int) -> tuple[str, int]:
    system = platform.system()
    if system == "Windows":
        return "peak-working-set", _windows_peak_working_set(process_id)
    if system == "Linux":
        status = Path(f"/proc/{process_id}/status").read_text(encoding="utf-8")
        return "peak-resident-set-vmhwm", parse_linux_peak_rss(status)
    if system == "Darwin":
        raw = _run_checked(["ps", "-o", "rss=", "-p", str(process_id)], timeout=10)
        return "held-resident-set-snapshot", int(raw.strip()) * 1024
    raise RuntimeError(f"memory measurement is unsupported on {system}")


def _run_worker(
    executable: Path,
    case: str,
    corpus: Path,
    work: Path,
    samples: int,
    warmup: int,
    timeout: int,
) -> tuple[dict[str, object], str, int]:
    work.mkdir(parents=True, exist_ok=False)
    process = subprocess.Popen(
        [
            str(executable),
            "--case",
            case,
            "--corpus-dir",
            str(corpus),
            "--work-dir",
            str(work),
            "--samples",
            str(samples),
            "--warmup",
            str(warmup),
            "--hold",
        ],
        cwd=ROOT,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if process.stdout is None or process.stderr is None or process.stdin is None:
        process.kill()
        process.wait(timeout=10)
        raise RuntimeError("benchmark worker pipes were not created")
    with ThreadPoolExecutor(max_workers=1) as executor:
        future = executor.submit(process.stdout.readline)
        try:
            line = future.result(timeout=timeout)
        except FutureTimeout as error:
            process.kill()
            process.wait(timeout=10)
            raise TimeoutError(
                f"benchmark case {case} exceeded {timeout} seconds"
            ) from error
    if not line:
        process.wait(timeout=10)
        diagnostic = process.stderr.read().strip().splitlines()
        suffix = diagnostic[-1] if diagnostic else "no diagnostic"
        raise RuntimeError(f"benchmark case {case} exited before reporting: {suffix}")
    try:
        result = parse_worker_result(line, case, samples, warmup)
        metric, memory_bytes = _process_memory(process.pid)
        process.stdin.write("\n")
        process.stdin.flush()
        process.stdin.close()
    except Exception:
        process.kill()
        process.wait(timeout=10)
        raise
    try:
        return_code = process.wait(timeout=10)
    except subprocess.TimeoutExpired as error:
        process.kill()
        process.wait(timeout=10)
        raise RuntimeError(
            f"benchmark case {case} did not leave its evidence hold"
        ) from error
    trailing_output = process.stdout.read().strip()
    diagnostic = process.stderr.read().strip()
    if return_code != 0 or trailing_output or diagnostic:
        raise RuntimeError(
            f"benchmark case {case} ended inconsistently: exit={return_code}, "
            f"stdout={bool(trailing_output)}, stderr={diagnostic or 'empty'}"
        )
    return result, metric, memory_bytes


def _benchmark_result(
    case: str,
    state: str,
    samples: list[int],
    warmup: int,
    memory_metric: str,
    memory_samples: list[int],
    checksums: list[int],
) -> dict[str, object]:
    return {
        "case": case,
        "state": state,
        "sample_count": len(samples),
        "warmup_count": warmup,
        "raw_samples_ns": samples,
        "summary": summarize_nanoseconds(samples),
        "worker_checksums": checksums,
        "memory": {
            "metric": memory_metric,
            "raw_samples_bytes": memory_samples,
            "maximum_bytes": max(memory_samples),
        },
    }


def _run_benchmarks(
    executable: Path,
    corpus: Path,
    work: Path,
    samples: int,
    warmup: int,
) -> list[dict[str, object]]:
    results: list[dict[str, object]] = []
    for case, include_process_cold, _ in BENCHMARK_CASES:
        if include_process_cold:
            cold_samples: list[int] = []
            cold_memory: list[int] = []
            cold_checksums: list[int] = []
            cold_metric = ""
            for sample in range(samples):
                worker, metric, memory = _run_worker(
                    executable,
                    case,
                    corpus,
                    work / f"{case}-cold-{sample:03d}",
                    1,
                    0,
                    180,
                )
                cold_samples.extend(worker["samples_ns"])
                cold_memory.append(memory)
                cold_checksums.append(worker["checksum"])
                if cold_metric and cold_metric != metric:
                    raise RuntimeError(
                        "memory metric changed within one benchmark case"
                    )
                cold_metric = metric
            results.append(
                _benchmark_result(
                    case,
                    "process-cold",
                    cold_samples,
                    0,
                    cold_metric,
                    cold_memory,
                    cold_checksums,
                )
            )

        worker, warm_metric, warm_memory = _run_worker(
            executable,
            case,
            corpus,
            work / f"{case}-warm",
            samples,
            warmup,
            300,
        )
        results.append(
            _benchmark_result(
                case,
                "warm-in-process",
                worker["samples_ns"],
                warmup,
                warm_metric,
                [warm_memory],
                [worker["checksum"]],
            )
        )
    return results


def _memory_bytes() -> int | None:
    system = platform.system()
    if system == "Windows":
        import ctypes

        class MemoryStatus(ctypes.Structure):
            _fields_ = [
                ("length", ctypes.c_ulong),
                ("memory_load", ctypes.c_ulong),
                ("total_physical", ctypes.c_ulonglong),
                ("available_physical", ctypes.c_ulonglong),
                ("total_page_file", ctypes.c_ulonglong),
                ("available_page_file", ctypes.c_ulonglong),
                ("total_virtual", ctypes.c_ulonglong),
                ("available_virtual", ctypes.c_ulonglong),
                ("available_extended_virtual", ctypes.c_ulonglong),
            ]

        status = MemoryStatus()
        status.length = ctypes.sizeof(status)
        if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
            return int(status.total_physical)
        return None
    if system == "Linux":
        content = Path("/proc/meminfo").read_text(encoding="utf-8")
        match = re.search(r"^MemTotal:\s+(\d+)\s+kB$", content, flags=re.MULTILINE)
        return int(match.group(1)) * 1024 if match else None
    if system == "Darwin":
        return int(_run_checked(["sysctl", "-n", "hw.memsize"], timeout=10).strip())
    return None


def _cpu_name() -> str:
    if platform.system() == "Windows":
        try:
            import winreg

            key_path = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0"
            with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, key_path) as key:
                value, _ = winreg.QueryValueEx(key, "ProcessorNameString")
                return str(value).strip()
        except OSError:
            return platform.processor() or platform.machine()
    if platform.system() == "Linux":
        content = Path("/proc/cpuinfo").read_text(encoding="utf-8", errors="replace")
        match = re.search(r"^model name\s*:\s*(.+)$", content, flags=re.MULTILINE)
        if match:
            return match.group(1).strip()
    if platform.system() == "Darwin":
        for key in ("machdep.cpu.brand_string", "hw.model"):
            try:
                value = _run_checked(["sysctl", "-n", key], timeout=10).strip()
                if value:
                    return value
            except RuntimeError:
                continue
    return platform.processor() or platform.machine()


def _storage_environment(path: Path) -> dict[str, str]:
    system = platform.system()
    if system == "Windows":
        import ctypes

        volume_path = ctypes.create_unicode_buffer(261)
        filesystem = ctypes.create_unicode_buffer(261)
        if not ctypes.windll.kernel32.GetVolumePathNameW(str(path), volume_path, 261):
            return {"type": "unknown", "source_class": "unknown"}
        if not ctypes.windll.kernel32.GetVolumeInformationW(
            volume_path.value, None, 0, None, None, None, filesystem, 261
        ):
            return {"type": "unknown", "source_class": "unknown"}
        drive_type = ctypes.windll.kernel32.GetDriveTypeW(volume_path.value)
        source_class = {2: "removable", 3: "local-volume", 4: "network"}.get(
            drive_type, "other"
        )
        return {"type": filesystem.value, "source_class": source_class}
    if system == "Linux":
        output = _run_checked(
            ["findmnt", "-n", "-T", str(path), "-o", "FSTYPE,SOURCE"], timeout=10
        )
        return parse_findmnt(output)
    if system == "Darwin":
        filesystem = _run_checked(["stat", "-f", "%T", str(path)], timeout=10).strip()
        return {"type": filesystem, "source_class": "local-or-mounted"}
    return {"type": "unknown", "source_class": "unknown"}


def _display_refresh_hz() -> int | None:
    system = platform.system()
    try:
        if system == "Windows":
            command = [
                "powershell",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "(Get-CimInstance Win32_VideoController | Where-Object CurrentRefreshRate | "
                "Select-Object -First 1 -ExpandProperty CurrentRefreshRate)",
            ]
            value = _run_checked(command, timeout=30).strip()
            return int(value) if value else None
        if system == "Linux" and os.environ.get("DISPLAY"):
            output = _run_checked(["xrandr", "--current"], timeout=10)
            match = re.search(r"\s(\d+(?:\.\d+)?)\*", output)
            return round(float(match.group(1))) if match else None
        if system == "Darwin":
            output = _run_checked(["system_profiler", "SPDisplaysDataType"], timeout=30)
            match = re.search(r"Refresh Rate:\s*(\d+)\s*Hz", output)
            return int(match.group(1)) if match else None
    except (FileNotFoundError, RuntimeError, ValueError):
        return None
    return None


def collect_environment(storage_path: Path) -> dict[str, object]:
    """Collect non-identifying reference-system evidence."""
    return {
        "operating_system": platform.system(),
        "operating_system_release": platform.release(),
        "operating_system_version": platform.version(),
        "architecture": platform.machine(),
        "cpu": _cpu_name(),
        "logical_processors": os.cpu_count(),
        "memory_bytes": _memory_bytes(),
        "storage": _storage_environment(storage_path),
        "display_refresh_hz": _display_refresh_hz(),
        "rustc": _run_checked(["rustc", "--version"], timeout=30).strip(),
        "cargo": _run_checked(["cargo", "--version"], timeout=30).strip(),
        "python": platform.python_version(),
        "build_profile": "bench",
    }


def _git_source(allow_dirty: bool) -> dict[str, object]:
    commit = _run_checked(["git", "rev-parse", "HEAD"], timeout=30).strip()
    tree = _run_checked(["git", "rev-parse", "HEAD^{tree}"], timeout=30).strip()
    git_directory = Path(
        _run_checked(["git", "rev-parse", "--git-dir"], timeout=30).strip()
    )
    common_directory = Path(
        _run_checked(["git", "rev-parse", "--git-common-dir"], timeout=30).strip()
    )
    if not git_directory.is_absolute():
        git_directory = ROOT / git_directory
    if not common_directory.is_absolute():
        common_directory = ROOT / common_directory
    branch = _run_checked(["git", "branch", "--show-current"], timeout=30).strip()
    checkout = (
        "detached-worktree"
        if git_directory.resolve() != common_directory.resolve() and not branch
        else "worktree"
    )
    status = _run_checked(
        ["git", "status", "--porcelain", "--untracked-files=normal"], timeout=30
    ).strip()
    clean = not status
    if not clean and not allow_dirty:
        raise RuntimeError(
            "reference evidence requires a clean tracked and untracked worktree"
        )
    return {
        "commit": commit,
        "tree": tree,
        "worktree_clean": clean,
        "checkout": checkout,
    }


def _valid_hex(value: object, pattern: re.Pattern[str]) -> bool:
    return isinstance(value, str) and pattern.fullmatch(value) is not None


def _is_integer(value: object, *, minimum: int = 0, maximum: int | None = None) -> bool:
    if isinstance(value, bool) or not isinstance(value, int) or value < minimum:
        return False
    return maximum is None or value <= maximum


def _validate_environment(environment: object, *, require_reference: bool) -> str:
    required = {
        "operating_system",
        "operating_system_release",
        "operating_system_version",
        "architecture",
        "cpu",
        "logical_processors",
        "memory_bytes",
        "storage",
        "display_refresh_hz",
        "rustc",
        "cargo",
        "python",
        "build_profile",
    }
    if not isinstance(environment, dict) or set(environment) != required:
        raise ValueError("artifact environment is incomplete or has unknown fields")
    operating_system = environment["operating_system"]
    if operating_system not in {"Windows", "Linux", "Darwin"}:
        raise ValueError("artifact operating system is unsupported")
    for field in (
        "operating_system_release",
        "operating_system_version",
        "architecture",
        "cpu",
    ):
        value = environment[field]
        if not isinstance(value, str) or not value.strip() or len(value) > 512:
            raise ValueError(f"artifact environment field {field} is invalid")
    if not _is_integer(environment["logical_processors"], minimum=1, maximum=4096):
        raise ValueError("artifact logical processor count is invalid")
    if not _is_integer(environment["memory_bytes"], minimum=1):
        raise ValueError("artifact physical memory is invalid")
    refresh = environment["display_refresh_hz"]
    if refresh is not None and not _is_integer(refresh, minimum=1, maximum=1000):
        raise ValueError("artifact display refresh is invalid")
    storage = environment["storage"]
    if not isinstance(storage, dict) or set(storage) != {"type", "source_class"}:
        raise ValueError("artifact storage evidence is invalid")
    if any(
        not isinstance(storage[field], str)
        or not storage[field].strip()
        or len(storage[field]) > 128
        for field in ("type", "source_class")
    ):
        raise ValueError("artifact storage evidence is invalid")
    if require_reference and (
        storage["type"].lower() == "unknown" or storage["source_class"] == "unknown"
    ):
        raise ValueError("reference storage must be identified")
    if (
        not isinstance(environment["rustc"], str)
        or not environment["rustc"].startswith("rustc ")
        or not isinstance(environment["cargo"], str)
        or not environment["cargo"].startswith("cargo ")
        or not isinstance(environment["python"], str)
        or re.fullmatch(r"\d+\.\d+\.\d+", environment["python"]) is None
        or environment["build_profile"] != "bench"
    ):
        raise ValueError("artifact toolchain evidence is invalid")
    return operating_system


def _validate_corpus(corpus: object, *, require_reference: bool) -> None:
    required_fields = {
        "generator_version",
        "scale",
        "files",
        "corpus_sha256",
        "search_markers",
        "adversarial_query_sha256",
    }
    if not isinstance(corpus, dict) or set(corpus) != required_fields:
        raise ValueError("artifact corpus evidence is invalid")
    scale = corpus.get("scale")
    if scale not in {"full", "smoke"}:
        raise ValueError("artifact corpus scale is invalid")
    if require_reference and scale != "full":
        raise ValueError("reference evidence requires the full corpus")
    if (
        corpus.get("generator_version") != 1
        or corpus.get("search_markers") != SEARCH_MARKERS
        or corpus.get("adversarial_query_sha256") != ADVERSARIAL_QUERY_SHA256
    ):
        raise ValueError("artifact corpus generator contract is invalid")
    files = corpus.get("files")
    if not isinstance(files, list) or len(files) != len(REFERENCE_CORPUS_FILES):
        raise ValueError("artifact corpus file manifest is incomplete")
    ordinary_size, large_size = corpus_sizes(scale)
    expected_shape = [
        {
            "name": name,
            "bytes": (
                0
                if name == "empty.txt"
                else large_size
                if name in {"source-large.txt", "log-large.txt"}
                else ordinary_size
            ),
            "description": description,
        }
        for name, _, _, description in REFERENCE_CORPUS_FILES
    ]
    for record, shape in zip(files, expected_shape, strict=True):
        if (
            not isinstance(record, dict)
            or set(record) != {"name", "bytes", "sha256", "description"}
            or {key: record.get(key) for key in shape} != shape
            or not _valid_hex(record.get("sha256"), HEX_64)
        ):
            raise ValueError("artifact corpus contains an invalid file record")
    if corpus.get("corpus_sha256") != corpus_manifest_digest(files):
        raise ValueError("artifact corpus digest does not match its file manifest")
    if require_reference and (
        files != reference_corpus_files()
        or corpus.get("corpus_sha256") != REFERENCE_CORPUS_SHA256
    ):
        raise ValueError("reference evidence does not contain the exact full corpus")


def _validate_build(build: object, *, require_reference: bool) -> None:
    if not isinstance(build, dict) or set(build) != {
        "binary",
        "benchmark_worker",
        "dependencies",
    }:
        raise ValueError("artifact build evidence is incomplete")
    for field, profile in (("binary", "release"), ("benchmark_worker", "bench")):
        artifact = build[field]
        if (
            not isinstance(artifact, dict)
            or set(artifact) != {"profile", "bytes", "sha256"}
            or artifact.get("profile") != profile
            or not _is_integer(artifact.get("bytes"), minimum=1)
            or not _valid_hex(artifact.get("sha256"), HEX_64)
        ):
            raise ValueError(f"artifact {field} evidence is invalid")
    if require_reference and build["binary"]["bytes"] > 12 * MIB:
        raise ValueError("reference release binary exceeds the 12 MiB ceiling")
    dependencies = build["dependencies"]
    required_dependency_fields = {
        "release_targets",
        "resolved_packages_by_target",
        "resolved_package_union",
        "direct_dependencies",
        "duplicate_versions",
        "locked_package_records",
    }
    if (
        not isinstance(dependencies, dict)
        or set(dependencies) != required_dependency_fields
    ):
        raise ValueError("artifact dependency evidence is incomplete")
    targets = sorted(SUPPORTED_TARGETS)
    if dependencies["release_targets"] != targets:
        raise ValueError("artifact release target evidence is invalid")
    target_counts = dependencies["resolved_packages_by_target"]
    if (
        not isinstance(target_counts, dict)
        or sorted(target_counts) != targets
        or any(not _is_integer(value, minimum=1) for value in target_counts.values())
    ):
        raise ValueError("artifact per-target dependency counts are invalid")
    union = dependencies["resolved_package_union"]
    locked = dependencies["locked_package_records"]
    if (
        not _is_integer(union, minimum=1)
        or not _is_integer(locked, minimum=1)
        or union > locked
    ):
        raise ValueError("artifact dependency totals are invalid")
    direct = dependencies["direct_dependencies"]
    if (
        not isinstance(direct, dict)
        or set(direct) != {"runtime", "development", "build"}
        or any(not _is_integer(value) for value in direct.values())
        or direct["runtime"] == 0
    ):
        raise ValueError("artifact direct dependency evidence is invalid")
    duplicates = dependencies["duplicate_versions"]
    if not isinstance(duplicates, dict):
        raise ValueError("artifact duplicate dependency evidence is invalid")
    for name, versions in duplicates.items():
        if (
            not isinstance(name, str)
            or not name
            or not isinstance(versions, list)
            or len(versions) < 2
            or versions != sorted(set(versions))
            or any(not isinstance(version, str) or not version for version in versions)
        ):
            raise ValueError("artifact duplicate dependency evidence is invalid")


def validate_artifact(artifact: dict[str, object], *, require_reference: bool) -> None:
    """Validate raw evidence and recompute every published percentile."""
    required_fields = {
        "schema_version",
        "evidence_class",
        "generated_at",
        "source",
        "scope",
        "provenance",
        "environment",
        "corpus",
        "build",
        "benchmarks",
    }
    if set(artifact) != required_fields:
        raise ValueError("artifact is incomplete or has unknown fields")
    if artifact.get("schema_version") != 2:
        raise ValueError("unsupported baseline evidence schema")
    generated_at = artifact.get("generated_at")
    try:
        parsed_time = datetime.fromisoformat(str(generated_at).replace("Z", "+00:00"))
    except ValueError as error:
        raise ValueError("artifact generation time is not ISO 8601") from error
    if (
        not isinstance(generated_at, str)
        or parsed_time.tzinfo is None
        or parsed_time.utcoffset() != UTC.utcoffset(None)
    ):
        raise ValueError("artifact generation time must use UTC")
    evidence_class = artifact.get("evidence_class")
    if evidence_class not in {"reference", "smoke"}:
        raise ValueError("invalid evidence class")
    source = artifact.get("source")
    if (
        not isinstance(source, dict)
        or set(source) != {"commit", "tree", "worktree_clean", "checkout"}
        or not _valid_hex(source.get("commit"), HEX_40)
        or not _valid_hex(source.get("tree"), HEX_40)
        or not isinstance(source.get("worktree_clean"), bool)
        or source.get("checkout") not in {"worktree", "detached-worktree"}
    ):
        raise ValueError("artifact source commit is invalid")
    if require_reference and (
        evidence_class != "reference"
        or source["worktree_clean"] is not True
        or source["checkout"] != "detached-worktree"
    ):
        raise ValueError("reference evidence must come from a clean detached commit")
    if artifact.get("scope") != evidence_scope():
        raise ValueError("artifact scope or limitations are invalid")
    if artifact.get("provenance") != PROVENANCE:
        raise ValueError("artifact provenance disclosure is invalid")
    _validate_corpus(artifact.get("corpus"), require_reference=require_reference)
    operating_system = _validate_environment(
        artifact.get("environment"), require_reference=require_reference
    )
    _validate_build(artifact.get("build"), require_reference=require_reference)

    benchmarks = artifact.get("benchmarks")
    if not isinstance(benchmarks, list) or not benchmarks:
        raise ValueError("artifact has no benchmark observations")
    observed_states = set()
    observed_keys = set()
    required_benchmark_fields = {
        "case",
        "state",
        "sample_count",
        "warmup_count",
        "raw_samples_ns",
        "summary",
        "worker_checksums",
        "memory",
    }
    for benchmark in benchmarks:
        if (
            not isinstance(benchmark, dict)
            or set(benchmark) != required_benchmark_fields
        ):
            raise ValueError("artifact benchmark entry is invalid")
        key = (benchmark.get("case"), benchmark.get("state"))
        if key in observed_keys or not all(isinstance(item, str) for item in key):
            raise ValueError("artifact benchmark cases must be unique and named")
        if benchmark["state"] not in {"process-cold", "warm-in-process"}:
            raise ValueError("artifact benchmark state is invalid")
        observed_keys.add(key)
        observed_states.add(benchmark["state"])
        raw = benchmark.get("raw_samples_ns")
        count = benchmark.get("sample_count")
        if (
            not isinstance(raw, list)
            or not _is_integer(count, minimum=1, maximum=MAXIMUM_SAMPLES)
            or count != len(raw)
            or any(not _is_integer(value, minimum=1) for value in raw)
        ):
            raise ValueError("artifact benchmark sample count does not match raw data")
        if require_reference and count < MINIMUM_REFERENCE_SAMPLES:
            raise ValueError("reference percentiles require at least 30 raw samples")
        if benchmark.get("summary") != summarize_nanoseconds(raw):
            raise ValueError("artifact benchmark summary does not match raw data")
        checksums = benchmark.get("worker_checksums")
        process_cold = benchmark["state"] == "process-cold"
        expected_workers = count if process_cold else 1
        if (
            not isinstance(checksums, list)
            or len(checksums) != expected_workers
            or any(
                isinstance(value, bool) or not isinstance(value, int) or value < 0
                for value in checksums
            )
        ):
            raise ValueError("artifact benchmark worker checksums are invalid")
        memory = benchmark.get("memory")
        memory_samples = (
            memory.get("raw_samples_bytes") if isinstance(memory, dict) else None
        )
        if (
            not isinstance(memory, dict)
            or set(memory) != {"metric", "raw_samples_bytes", "maximum_bytes"}
            or not isinstance(memory_samples, list)
            or len(memory_samples) != expected_workers
            or any(not _is_integer(value, minimum=1) for value in memory_samples)
            or memory.get("maximum_bytes") != max(memory_samples)
        ):
            raise ValueError("artifact benchmark memory evidence is invalid")
        expected_metric = {
            "Windows": "peak-working-set",
            "Linux": "peak-resident-set-vmhwm",
            "Darwin": "held-resident-set-snapshot",
        }[operating_system]
        if memory["metric"] != expected_metric:
            raise ValueError("artifact benchmark memory metric is invalid")
        warmup_count = benchmark.get("warmup_count")
        if (
            not _is_integer(warmup_count, maximum=MAXIMUM_WARMUP)
            or (process_cold and warmup_count != 0)
            or (require_reference and not process_cold and warmup_count == 0)
        ):
            raise ValueError("artifact benchmark warmup evidence is invalid")
    if require_reference and not {"process-cold", "warm-in-process"}.issubset(
        observed_states
    ):
        raise ValueError(
            "reference evidence must distinguish process-cold and warm states"
        )
    if require_reference:
        expected_keys = {
            (case, state)
            for case, include_process_cold, _ in BENCHMARK_CASES
            for state in (
                ("process-cold", "warm-in-process")
                if include_process_cold
                else ("warm-in-process",)
            )
        }
        if observed_keys != expected_keys:
            raise ValueError(
                "reference evidence does not contain the exact benchmark case matrix"
            )


def verify_source_commit(source: dict[str, object]) -> None:
    """Require the recorded commit and tree to exist in this repository."""
    commit = str(source["commit"])
    try:
        _run_checked(["git", "cat-file", "-e", f"{commit}^{{commit}}"], timeout=30)
        tree = _run_checked(
            ["git", "rev-parse", f"{commit}^{{tree}}"], timeout=30
        ).strip()
    except RuntimeError as error:
        raise ValueError("artifact source commit is not present locally") from error
    if tree != source["tree"]:
        raise ValueError("artifact source tree does not match its commit")


def encode_artifact(artifact: dict[str, object]) -> bytes:
    """Encode one artifact in the repository's canonical JSON representation."""
    return (
        json.dumps(artifact, ensure_ascii=True, indent=2, sort_keys=True) + "\n"
    ).encode("utf-8")


def read_artifact(path: Path) -> tuple[dict[str, object], str]:
    """Read, parse, and hash the same bounded canonical artifact bytes."""
    with path.open("rb") as artifact_file:
        encoded = artifact_file.read(MAXIMUM_ARTIFACT_BYTES + 1)
    if len(encoded) > MAXIMUM_ARTIFACT_BYTES:
        raise ValueError("M1 evidence artifact exceeds the bounded size limit")
    try:
        artifact = json.loads(encoded.decode("utf-8"))
    except UnicodeDecodeError as error:
        raise ValueError("M1 evidence artifact is not UTF-8") from error
    if not isinstance(artifact, dict):
        raise ValueError("M1 evidence artifact root must be an object")
    if encoded != encode_artifact(artifact):
        raise ValueError("M1 evidence artifact is not canonical JSON")
    return artifact, hashlib.sha256(encoded).hexdigest()


def write_artifact(destination: Path, artifact: dict[str, object]) -> str:
    """Durably promote canonical evidence without overwriting an existing leaf."""
    destination.parent.mkdir(parents=True, exist_ok=True)
    encoded = encode_artifact(artifact)
    descriptor, temporary_name = tempfile.mkstemp(
        dir=destination.parent,
        prefix=f".{destination.name}.",
        suffix=".pending",
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(encoded)
            output.flush()
            os.fsync(output.fileno())
        os.link(temporary, destination)
    finally:
        temporary.unlink(missing_ok=True)
    return hashlib.sha256(encoded).hexdigest()


def _trusted_reference_destination(destination: Path) -> Path:
    expected_root = (ROOT / "docs" / "evidence").resolve()
    resolved = destination.resolve()
    try:
        resolved.relative_to(expected_root)
    except ValueError as error:
        raise ValueError(
            "reference artifacts must be written under docs/evidence"
        ) from error
    if resolved.suffix.lower() != ".json":
        raise ValueError("reference artifacts must use a .json extension")
    return resolved


def _produce_artifact(
    destination: Path,
    evidence_class: str,
    samples: int,
    warmup: int,
    allow_dirty: bool,
    *,
    isolated_reference: bool = False,
) -> dict[str, object]:
    if evidence_class == "reference" and samples < MINIMUM_REFERENCE_SAMPLES:
        raise ValueError("reference evidence requires at least 30 samples")
    if not 0 < samples <= MAXIMUM_SAMPLES or not 0 <= warmup <= MAXIMUM_WARMUP:
        raise ValueError(
            "sample and warmup counts are outside the bounded supported range"
        )
    if evidence_class == "reference" and allow_dirty:
        raise ValueError("dirty runs cannot be labeled as reference evidence")
    if evidence_class == "reference" and not isolated_reference:
        raise ValueError("reference evidence must run in a detached commit worktree")

    source = _git_source(allow_dirty)
    if isolated_reference and source["checkout"] != "detached-worktree":
        raise RuntimeError(
            "isolated reference mode requires a detached linked worktree"
        )
    with tempfile.TemporaryDirectory(prefix="noter-m1-baseline-") as temporary:
        temporary_path = Path(temporary)
        corpus = generate_corpus(
            temporary_path / "corpus",
            "full" if evidence_class == "reference" else "smoke",
        )
        environment = collect_environment(temporary_path)
        metadata_by_target, root_id = _cargo_metadata()
        build = {
            "binary": _build_release_binary(),
            "dependencies": summarize_dependencies(metadata_by_target, root_id),
        }
        build["dependencies"]["locked_package_records"] = locked_package_count(
            ROOT / "Cargo.lock"
        )
        executable = _build_benchmark_worker()
        build["benchmark_worker"] = {
            "profile": "bench",
            "bytes": executable.stat().st_size,
            "sha256": _sha256_file(executable),
        }
        benchmarks = _run_benchmarks(
            executable,
            temporary_path / "corpus",
            temporary_path / "work",
            samples,
            warmup,
        )

    if evidence_class == "reference":
        ending_source = _git_source(False)
        if ending_source != source:
            raise RuntimeError(
                "source state changed while reference evidence was running"
            )

    artifact = {
        "schema_version": 2,
        "evidence_class": evidence_class,
        "generated_at": datetime.now(UTC).isoformat().replace("+00:00", "Z"),
        "source": source,
        "scope": evidence_scope(),
        "provenance": dict(PROVENANCE),
        "environment": environment,
        "corpus": corpus,
        "build": build,
        "benchmarks": benchmarks,
    }
    validate_artifact(artifact, require_reference=evidence_class == "reference")
    write_artifact(destination, artifact)
    return artifact


def _run_reference_in_detached_worktree(
    destination: Path, samples: int, warmup: int
) -> tuple[dict[str, object], str]:
    """Build and measure the recorded commit from an isolated Git worktree."""
    source = _git_source(False)
    destination = _trusted_reference_destination(destination)
    with tempfile.TemporaryDirectory(prefix="noter-m1-reference-source-") as temporary:
        checkout = Path(temporary) / "source"
        _run_checked(
            [
                "git",
                "worktree",
                "add",
                "--detach",
                str(checkout),
                str(source["commit"]),
            ],
            timeout=120,
        )
        try:
            _run_checked(
                [
                    sys.executable,
                    "-I",
                    str(checkout / "scripts" / "run_m1_baseline.py"),
                    "--output",
                    str(destination),
                    "--evidence-class",
                    "reference",
                    "--samples",
                    str(samples),
                    "--warmup",
                    str(warmup),
                    "--isolated-reference",
                ],
                cwd=checkout,
                timeout=7_200,
            )
        finally:
            _run_checked(
                ["git", "worktree", "remove", "--force", str(checkout)],
                timeout=120,
            )
    artifact, digest = read_artifact(destination)
    validate_artifact(artifact, require_reference=True)
    verify_source_commit(artifact["source"])
    if (
        artifact["source"]["commit"] != source["commit"]
        or artifact["source"]["tree"] != source["tree"]
    ):
        raise RuntimeError("detached benchmark source differs from the selected commit")
    return artifact, digest


def _argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument(
        "--output", type=Path, help="new JSON evidence artifact to create"
    )
    action.add_argument(
        "--validate-artifact", type=Path, help="existing JSON artifact to validate"
    )
    parser.add_argument(
        "--evidence-class", choices=("reference", "smoke"), default="reference"
    )
    parser.add_argument("--samples", type=int, default=MINIMUM_REFERENCE_SAMPLES)
    parser.add_argument("--warmup", type=int, default=5)
    parser.add_argument("--allow-dirty", action="store_true")
    parser.add_argument(
        "--isolated-reference", action="store_true", help=argparse.SUPPRESS
    )
    return parser


def main(arguments: list[str] | None = None) -> int:
    """Run or validate the M1 baseline evidence workflow."""
    options = _argument_parser().parse_args(arguments)
    try:
        if options.validate_artifact is not None:
            artifact, digest = read_artifact(options.validate_artifact)
            validate_artifact(artifact, require_reference=True)
            verify_source_commit(artifact["source"])
            print(
                "Checked M1 reference evidence structure, internal consistency, "
                f"and local source commit: SHA-256 {digest}"
            )
            return 0
        if options.evidence_class == "reference" and not options.isolated_reference:
            artifact, digest = _run_reference_in_detached_worktree(
                options.output, options.samples, options.warmup
            )
        else:
            destination = (
                options.output
                if options.isolated_reference
                else _trusted_reference_destination(options.output)
                if options.evidence_class == "reference"
                else options.output
            )
            artifact = _produce_artifact(
                destination,
                options.evidence_class,
                options.samples,
                options.warmup,
                options.allow_dirty,
                isolated_reference=options.isolated_reference,
            )
            digest = hashlib.sha256(encode_artifact(artifact)).hexdigest()
        print(
            f"Created {artifact['evidence_class']} M1 evidence with "
            f"{len(artifact['benchmarks'])} result sets: SHA-256 {digest}"
        )
        return 0
    except (
        FileExistsError,
        OSError,
        RuntimeError,
        ValueError,
    ) as error:
        print(f"M1 baseline failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
