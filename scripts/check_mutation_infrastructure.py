#!/usr/bin/env python3
"""Reject infrastructure failures that cargo-mutants labels as unviable."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path
from typing import Any


INFRASTRUCTURE_PATTERNS = (
    ("linker invocation failure", re.compile(r"error: linking with .* failed", re.I)),
    ("Windows linker file error", re.compile(r"\bLNK\d{4}\b", re.I)),
    ("compiler internal error", re.compile(r"internal compiler error", re.I)),
    (
        "storage exhaustion",
        re.compile(r"no space left on device|disk (?:is )?full", re.I),
    ),
    (
        "process resource exhaustion",
        re.compile(r"resource temporarily unavailable|too many open files", re.I),
    ),
    (
        "tool lock contention",
        re.compile(
            r"timed out waiting for file lock|used by another process|text file busy",
            re.I,
        ),
    ),
)


def scenario_name(scenario: Any) -> str:
    """Return a stable human-readable scenario name from cargo-mutants JSON."""
    if isinstance(scenario, str):
        return scenario
    if isinstance(scenario, dict):
        mutant = scenario.get("Mutant")
        if isinstance(mutant, dict) and isinstance(mutant.get("name"), str):
            return mutant["name"]
    return "unknown mutation scenario"


def infrastructure_failures(output_directory: Path) -> list[str]:
    """Return unviable scenarios whose logs contain infrastructure failures."""
    root = output_directory.resolve()
    report_path = root / "outcomes.json"
    try:
        report = json.loads(report_path.read_text(encoding="utf-8"))
    except FileNotFoundError as error:
        raise ValueError(f"mutation report is missing: {report_path}") from error
    except json.JSONDecodeError as error:
        raise ValueError(f"mutation report is invalid JSON: {report_path}") from error

    outcomes = report.get("outcomes") if isinstance(report, dict) else None
    if not isinstance(outcomes, list):
        raise ValueError("mutation report must contain an outcomes list")

    diagnostics: list[str] = []
    for outcome in outcomes:
        if not isinstance(outcome, dict) or outcome.get("summary") != "Unviable":
            continue
        log_path = outcome.get("log_path")
        if not isinstance(log_path, str):
            diagnostics.append(
                f"{scenario_name(outcome.get('scenario'))}: missing build log path"
            )
            continue

        resolved_log = (root / log_path).resolve()
        if not resolved_log.is_relative_to(root):
            diagnostics.append(
                f"{scenario_name(outcome.get('scenario'))}: build log escapes output directory"
            )
            continue
        try:
            log = resolved_log.read_text(encoding="utf-8", errors="replace")
        except FileNotFoundError:
            diagnostics.append(
                f"{scenario_name(outcome.get('scenario'))}: missing build log {log_path}"
            )
            continue

        for label, pattern in INFRASTRUCTURE_PATTERNS:
            if pattern.search(log):
                diagnostics.append(
                    f"{scenario_name(outcome.get('scenario'))}: {label} in {log_path}"
                )
                break

    return diagnostics


def main(arguments: list[str] | None = None) -> int:
    """Validate one cargo-mutants output directory."""
    arguments = sys.argv[1:] if arguments is None else arguments
    if len(arguments) != 1:
        print(
            "usage: check_mutation_infrastructure.py <mutants.out>", file=sys.stderr
        )
        return 2

    try:
        diagnostics = infrastructure_failures(Path(arguments[0]))
    except (OSError, ValueError) as error:
        print(error, file=sys.stderr)
        return 1

    if diagnostics:
        print("\n".join(diagnostics), file=sys.stderr)
        return 1

    print("Mutation report contains no recognized infrastructure failures.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
