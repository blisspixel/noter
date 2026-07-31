#!/usr/bin/env python3
"""Generate the target-specific CycloneDX release artifacts declared to dist."""

from __future__ import annotations

import os
import re
import stat
import subprocess
import xml.etree.ElementTree as element_tree
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
RELEASE_TARGETS = (
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
)
MAX_SBOM_BYTES = 16 * 1024 * 1024
SHA_PATTERN = re.compile(r"[0-9a-f]{40}")


def release_revision(environment: dict[str, str]) -> str:
    """Select a validated immutable revision when CI provides one."""

    github_sha = environment.get("GITHUB_SHA")
    if github_sha is None:
        return "HEAD"
    if SHA_PATTERN.fullmatch(github_sha) is None:
        raise ValueError("GITHUB_SHA must be a full lowercase Git commit ID")
    return github_sha


def source_date_epoch(root: Path, revision: str) -> str:
    """Read the selected revision's commit time for reproducible SBOM metadata."""

    result = subprocess.run(
        ["git", "show", "-s", "--format=%ct", revision],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )
    timestamp = result.stdout.strip()
    if not timestamp.isascii() or not timestamp.isdecimal() or int(timestamp) <= 0:
        raise ValueError("Git returned an invalid commit timestamp")
    return timestamp


def output_path(root: Path, target: str) -> Path:
    """Return cargo-cyclonedx's target-qualified output path."""

    return root / f"noter_{target}.cdx.xml"


def unpublished_workspace_output(root: Path, target: str) -> Path:
    """Return the workspace-library SBOM that is not a shipped application asset."""

    return root / "crates" / "noter-platform" / f"noter-platform_{target}.cdx.xml"


def remove_previous_output(path: Path) -> None:
    """Remove only a regular prior build output, never a link or directory."""

    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return
    if not stat.S_ISREG(metadata.st_mode):
        raise ValueError(f"refusing to replace non-regular SBOM output: {path.name}")
    path.unlink()


def validate_sbom(path: Path) -> None:
    """Fail if the generator did not produce a bounded CycloneDX 1.5 document."""

    metadata = path.lstat()
    if not stat.S_ISREG(metadata.st_mode):
        raise ValueError(f"SBOM output is not a regular file: {path.name}")
    if metadata.st_size <= 0 or metadata.st_size > MAX_SBOM_BYTES:
        raise ValueError(f"SBOM output has an invalid size: {path.name}")
    with path.open("rb") as stream:
        contents = stream.read(MAX_SBOM_BYTES + 1)
    if len(contents) > MAX_SBOM_BYTES:
        raise ValueError(f"SBOM output exceeds the size limit: {path.name}")
    root = element_tree.fromstring(contents)
    if root.tag != "{http://cyclonedx.org/schema/bom/1.5}bom":
        raise ValueError(f"SBOM output is not CycloneDX 1.5: {path.name}")
    if root.attrib.get("version") != "1":
        raise ValueError(f"SBOM output has an unexpected document version: {path.name}")


def generate_release_sboms(root: Path = REPOSITORY_ROOT) -> None:
    """Generate and validate the exact SBOM set declared in Cargo.toml."""

    environment = os.environ.copy()
    revision = release_revision(environment)
    environment["SOURCE_DATE_EPOCH"] = source_date_epoch(root, revision)

    for target in RELEASE_TARGETS:
        path = output_path(root, target)
        unpublished = unpublished_workspace_output(root, target)
        remove_previous_output(path)
        remove_previous_output(unpublished)
        try:
            subprocess.run(
                [
                    "cargo",
                    "cyclonedx",
                    "--manifest-path",
                    "Cargo.toml",
                    "--all-features",
                    "--target",
                    target,
                    "--target-in-filename",
                    "--license-strict",
                    "--license-accept-named",
                    "MIT/Apache-2.0",
                    "--license-accept-named",
                    "MIT / Apache-2.0",
                    "--license-accept-named",
                    "Apache-2.0/MIT",
                    "--spec-version",
                    "1.5",
                    "--no-build-deps",
                ],
                cwd=root,
                env=environment,
                check=True,
            )
            validate_sbom(path)
        finally:
            remove_previous_output(unpublished)


def main() -> None:
    """Generate the release SBOM set or fail before cargo-dist can publish."""

    generate_release_sboms()


if __name__ == "__main__":
    main()
