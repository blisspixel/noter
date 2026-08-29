#!/usr/bin/env python3
"""Validate, assemble, and remotely ratify Noter's release artifact set."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import stat
from pathlib import Path
from typing import Any


DIST_VERSION = "0.32.0"
APP_NAME = "noter"
APP_VERSION = "0.1.0-alpha.2"
RELEASE_TARGETS = (
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
)
TARGET_RUNNERS = {
    "aarch64-apple-darwin": "macos-14",
    "x86_64-apple-darwin": "macos-15-intel",
    "x86_64-pc-windows-msvc": "windows-2022",
    "x86_64-unknown-linux-gnu": "ubuntu-22.04",
}
RELEASE_ARTIFACT_KINDS = {
    "source.tar.gz": "source-tarball",
    "source.tar.gz.sha256": "checksum",
    "noter_aarch64-apple-darwin.cdx.xml": "extra-artifact",
    "noter_x86_64-apple-darwin.cdx.xml": "extra-artifact",
    "noter_x86_64-pc-windows-msvc.cdx.xml": "extra-artifact",
    "noter_x86_64-unknown-linux-gnu.cdx.xml": "extra-artifact",
    "noter-installer.sh": "installer",
    "noter-installer.ps1": "installer",
    "noter.rb": "installer",
    "sha256.sum": "unified-checksum",
    "noter-aarch64-apple-darwin.tar.xz": "executable-zip",
    "noter-aarch64-apple-darwin.tar.xz.sha256": "checksum",
    "noter-x86_64-apple-darwin.tar.xz": "executable-zip",
    "noter-x86_64-apple-darwin.tar.xz.sha256": "checksum",
    "noter-x86_64-pc-windows-msvc.zip": "executable-zip",
    "noter-x86_64-pc-windows-msvc.zip.sha256": "checksum",
    "noter-x86_64-pc-windows-msvc.msi": "installer",
    "noter-x86_64-pc-windows-msvc.msi.sha256": "checksum",
    "noter-x86_64-unknown-linux-gnu.tar.xz": "executable-zip",
    "noter-x86_64-unknown-linux-gnu.tar.xz.sha256": "checksum",
}
# Only the artifacts a target's own runner can produce belong here. The
# target-specific SBOMs are named for a target but are not built by one: dist
# models `extra-artifacts` as a single global step, and
# `scripts/generate_release_sboms.py` emits all four in one invocation on the
# global runner, which is the only job that installs cargo-cyclonedx. They are
# therefore validated as global artifacts, out of the global container.
LOCAL_RELEASE_ARTIFACT_TARGETS = {
    "noter-aarch64-apple-darwin.tar.xz": "aarch64-apple-darwin",
    "noter-aarch64-apple-darwin.tar.xz.sha256": "aarch64-apple-darwin",
    "noter-x86_64-apple-darwin.tar.xz": "x86_64-apple-darwin",
    "noter-x86_64-apple-darwin.tar.xz.sha256": "x86_64-apple-darwin",
    "noter-x86_64-pc-windows-msvc.zip": "x86_64-pc-windows-msvc",
    "noter-x86_64-pc-windows-msvc.zip.sha256": "x86_64-pc-windows-msvc",
    "noter-x86_64-pc-windows-msvc.msi": "x86_64-pc-windows-msvc",
    "noter-x86_64-pc-windows-msvc.msi.sha256": "x86_64-pc-windows-msvc",
    "noter-x86_64-unknown-linux-gnu.tar.xz": "x86_64-unknown-linux-gnu",
    "noter-x86_64-unknown-linux-gnu.tar.xz.sha256": "x86_64-unknown-linux-gnu",
}
LOCAL_RELEASE_ARTIFACTS = set(LOCAL_RELEASE_ARTIFACT_TARGETS)
GLOBAL_RELEASE_ARTIFACTS = set(RELEASE_ARTIFACT_KINDS) - LOCAL_RELEASE_ARTIFACTS
CHECKSUM_SIDECARS = {
    "source.tar.gz.sha256": "source.tar.gz",
    "noter-aarch64-apple-darwin.tar.xz.sha256": ("noter-aarch64-apple-darwin.tar.xz"),
    "noter-x86_64-apple-darwin.tar.xz.sha256": ("noter-x86_64-apple-darwin.tar.xz"),
    "noter-x86_64-pc-windows-msvc.zip.sha256": ("noter-x86_64-pc-windows-msvc.zip"),
    "noter-x86_64-pc-windows-msvc.msi.sha256": ("noter-x86_64-pc-windows-msvc.msi"),
    "noter-x86_64-unknown-linux-gnu.tar.xz.sha256": (
        "noter-x86_64-unknown-linux-gnu.tar.xz"
    ),
}
CANONICAL_MANIFEST = "dist-manifest.json"
PLAN_CONTAINER = "artifacts-plan-dist-manifest"
GLOBAL_CONTAINER = "artifacts-build-global"
HOST_CONTAINER = "artifacts-dist-manifest"
MAX_MANIFEST_BYTES = 2 * 1024 * 1024
MAX_ARTIFACT_BYTES = 128 * 1024 * 1024
MAX_TOTAL_ARTIFACT_BYTES = 512 * 1024 * 1024
MAX_ARTIFACT_FILES = 64


class ReleaseArtifactError(ValueError):
    """A release payload violated its fail-closed inventory contract."""


def _regular_file_metadata(path: Path, maximum: int) -> os.stat_result:
    """Return bounded regular-file metadata without accepting a link."""

    metadata = path.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise ReleaseArtifactError(f"release input is not a regular file: {path.name}")
    if metadata.st_size <= 0 or metadata.st_size > maximum:
        raise ReleaseArtifactError(f"release input has an invalid size: {path.name}")
    return metadata


def _read_json(path: Path) -> dict[str, Any]:
    """Read one bounded regular JSON object."""

    metadata = _regular_file_metadata(path, MAX_MANIFEST_BYTES)
    with path.open("rb") as stream:
        opened = os.fstat(stream.fileno())
        if (
            not stat.S_ISREG(opened.st_mode)
            or opened.st_dev != metadata.st_dev
            or opened.st_ino != metadata.st_ino
        ):
            raise ReleaseArtifactError(
                f"release manifest changed while it was opened: {path.name}"
            )
        contents = stream.read(MAX_MANIFEST_BYTES + 1)
        final = os.fstat(stream.fileno())
    if len(contents) > MAX_MANIFEST_BYTES:
        raise ReleaseArtifactError(f"release manifest is too large: {path.name}")
    if final.st_size != opened.st_size or final.st_mtime_ns != opened.st_mtime_ns:
        raise ReleaseArtifactError(
            f"release manifest changed while it was read: {path.name}"
        )
    try:
        value = json.loads(contents)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ReleaseArtifactError(
            f"release manifest is not valid UTF-8 JSON: {path.name}"
        ) from error
    if not isinstance(value, dict):
        raise ReleaseArtifactError(f"release manifest is not an object: {path.name}")
    return value


def _validate_plain_name(name: object, description: str) -> str:
    """Reject path syntax where the release contract requires one basename."""

    if (
        not isinstance(name, str)
        or not name
        or name in {".", ".."}
        or Path(name).name != name
        or "/" in name
        or "\\" in name
    ):
        raise ReleaseArtifactError(f"{description} is not a safe artifact name")
    return name


def validate_plan(plan: dict[str, Any], expected_tag: str | None = None) -> None:
    """Validate the pinned cargo-dist schema and exact required release inventory."""

    if plan.get("dist_version") != DIST_VERSION:
        raise ReleaseArtifactError("release plan uses an unexpected cargo-dist version")
    releases = plan.get("releases")
    if not isinstance(releases, list) or len(releases) != 1:
        raise ReleaseArtifactError("release plan must contain exactly one application")
    release = releases[0]
    if not isinstance(release, dict):
        raise ReleaseArtifactError("release plan application is not an object")
    if release.get("app_name") != APP_NAME or release.get("app_version") != APP_VERSION:
        raise ReleaseArtifactError("release plan application identity is unexpected")
    names_value = release.get("artifacts")
    if not isinstance(names_value, list):
        raise ReleaseArtifactError("release plan artifact inventory is not a list")
    names = [
        _validate_plain_name(name, "release plan artifact") for name in names_value
    ]
    if len(names) != len(set(names)):
        raise ReleaseArtifactError("release plan contains duplicate artifact names")
    if set(names) != set(RELEASE_ARTIFACT_KINDS):
        raise ReleaseArtifactError(
            "release plan differs from the required artifact set"
        )

    artifacts = plan.get("artifacts")
    if not isinstance(artifacts, dict) or set(artifacts) != set(names):
        raise ReleaseArtifactError(
            "release plan artifact map differs from its inventory"
        )
    for name, expected_kind in RELEASE_ARTIFACT_KINDS.items():
        artifact = artifacts.get(name)
        if (
            not isinstance(artifact, dict)
            or artifact.get("name") != name
            or artifact.get("kind") != expected_kind
        ):
            raise ReleaseArtifactError(
                f"release plan artifact metadata is unexpected: {name}"
            )

    if expected_tag:
        if (
            plan.get("announcement_tag") != expected_tag
            or plan.get("announcement_is_prerelease") is not True
        ):
            raise ReleaseArtifactError(
                "release plan is not bound to the requested prerelease tag"
            )

    try:
        github = plan["ci"]["github"]
        include = github["artifacts_matrix"]["include"]
    except (KeyError, TypeError):
        raise ReleaseArtifactError(
            "release plan is missing its local build matrix"
        ) from None
    if not isinstance(include, list) or len(include) != len(RELEASE_TARGETS):
        raise ReleaseArtifactError("release plan local build matrix has the wrong size")
    seen_targets: set[str] = set()
    for entry in include:
        if not isinstance(entry, dict):
            raise ReleaseArtifactError(
                "release plan local matrix entry is not an object"
            )
        targets = entry.get("targets")
        if not isinstance(targets, list) or len(targets) != 1:
            raise ReleaseArtifactError(
                "each release plan local matrix entry must own one target"
            )
        target = targets[0]
        if (
            not isinstance(target, str)
            or target not in TARGET_RUNNERS
            or entry.get("runner") != TARGET_RUNNERS[target]
        ):
            raise ReleaseArtifactError("release plan local target runner is unexpected")
        if target in seen_targets:
            raise ReleaseArtifactError("release plan local target is duplicated")
        seen_targets.add(target)
    if seen_targets != set(RELEASE_TARGETS):
        raise ReleaseArtifactError("release plan local targets are incomplete")


def _expected_containers(stage: str, global_result: str | None) -> set[str]:
    """Return the exact workflow-artifact containers allowed at one stage."""

    containers = {PLAN_CONTAINER}
    containers.update(f"artifacts-build-local-{target}" for target in RELEASE_TARGETS)
    if stage in {"host", "publish"}:
        if global_result != "success":
            raise ReleaseArtifactError("required global artifact build did not succeed")
        containers.add(GLOBAL_CONTAINER)
    if stage == "publish":
        containers.add(HOST_CONTAINER)
    return containers


def _artifact_files(input_root: Path, expected_containers: set[str]) -> dict[str, Path]:
    """Inventory separated workflow artifacts and reject every collision."""

    root_metadata = input_root.lstat()
    if stat.S_ISLNK(root_metadata.st_mode) or not stat.S_ISDIR(root_metadata.st_mode):
        raise ReleaseArtifactError("release artifact input root is not a directory")
    containers: dict[str, Path] = {}
    for entry in input_root.iterdir():
        name = _validate_plain_name(entry.name, "workflow artifact container")
        metadata = entry.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
            raise ReleaseArtifactError(
                f"workflow artifact container is not a directory: {name}"
            )
        containers[name] = entry
    if set(containers) != expected_containers:
        raise ReleaseArtifactError(
            "downloaded workflow artifact containers differ from the expected set"
        )

    files: dict[str, Path] = {}
    total_size = 0
    for container_name in sorted(containers):
        entries = list(containers[container_name].iterdir())
        if not entries:
            raise ReleaseArtifactError(
                f"workflow artifact container is empty: {container_name}"
            )
        for path in entries:
            name = _validate_plain_name(path.name, "downloaded release artifact")
            metadata = _regular_file_metadata(path, MAX_ARTIFACT_BYTES)
            total_size += metadata.st_size
            if total_size > MAX_TOTAL_ARTIFACT_BYTES:
                raise ReleaseArtifactError(
                    "release artifact set exceeds its size limit"
                )
            if name in files:
                raise ReleaseArtifactError(
                    f"workflow artifact filename collision detected: {name}"
                )
            files[name] = path
            if len(files) > MAX_ARTIFACT_FILES:
                raise ReleaseArtifactError("release artifact set has too many files")
    return files


def _is_granular_manifest(name: str) -> bool:
    return name != CANONICAL_MANIFEST and name.endswith("-dist-manifest.json")


def _require_container_manifests(
    files: dict[str, Path], input_root: Path, stage: str
) -> None:
    required = {
        "plan-dist-manifest.json": input_root
        / PLAN_CONTAINER
        / "plan-dist-manifest.json"
    }
    required.update(
        {
            f"{target}-dist-manifest.json": input_root
            / f"artifacts-build-local-{target}"
            / f"{target}-dist-manifest.json"
            for target in RELEASE_TARGETS
        }
    )
    if stage in {"host", "publish"}:
        required["global-dist-manifest.json"] = (
            input_root / GLOBAL_CONTAINER / "global-dist-manifest.json"
        )
    if stage == "publish":
        required[CANONICAL_MANIFEST] = input_root / HOST_CONTAINER / CANONICAL_MANIFEST
    if any(files.get(name) != path for name, path in required.items()):
        raise ReleaseArtifactError(
            "release artifact set is missing a correctly owned manifest"
        )
    for name in required:
        _read_json(files[name])


def _copy_regular(source: Path, destination: Path) -> None:
    """Copy one already inventoried regular file without accepting replacement."""

    observed = _regular_file_metadata(source, MAX_ARTIFACT_BYTES)
    with source.open("rb") as reader, destination.open("xb") as writer:
        opened = os.fstat(reader.fileno())
        if (
            not stat.S_ISREG(opened.st_mode)
            or opened.st_dev != observed.st_dev
            or opened.st_ino != observed.st_ino
            or opened.st_size != observed.st_size
        ):
            raise ReleaseArtifactError(
                f"release artifact changed while it was opened: {source.name}"
            )
        shutil.copyfileobj(reader, writer, length=1024 * 1024)
        final = os.fstat(reader.fileno())
    if (
        final.st_size != opened.st_size
        or final.st_mtime_ns != opened.st_mtime_ns
        or destination.stat().st_size != opened.st_size
    ):
        raise ReleaseArtifactError(
            f"release artifact changed while it was copied: {source.name}"
        )


def prepare_artifacts(
    input_root: Path,
    destination: Path,
    stage: str,
    expected_tag: str | None = None,
    global_result: str | None = None,
) -> None:
    """Validate separated inputs and assemble one collision-free cargo-dist directory."""

    expected_containers = _expected_containers(stage, global_result)
    files = _artifact_files(input_root, expected_containers)
    plan_path = input_root / PLAN_CONTAINER / "plan-dist-manifest.json"
    if files.get("plan-dist-manifest.json") != plan_path:
        raise ReleaseArtifactError(
            "plan manifest is not owned by the expected workflow artifact"
        )
    plan = _read_json(plan_path)
    validate_plan(plan, expected_tag)
    _require_container_manifests(files, input_root, stage)

    for name, target in LOCAL_RELEASE_ARTIFACT_TARGETS.items():
        expected_path = input_root / f"artifacts-build-local-{target}" / name
        if files.get(name) != expected_path:
            raise ReleaseArtifactError(
                f"local release artifact is missing from its target build: {name}"
            )
    if stage in {"host", "publish"}:
        for name in GLOBAL_RELEASE_ARTIFACTS:
            expected_path = input_root / GLOBAL_CONTAINER / name
            if files.get(name) != expected_path:
                raise ReleaseArtifactError(
                    f"global release artifact is missing from its global build: {name}"
                )

    payload = {name for name in files if not _is_granular_manifest(name)}
    if stage == "global":
        if payload != LOCAL_RELEASE_ARTIFACTS:
            raise ReleaseArtifactError(
                "local build payload differs from the planned local artifact set"
            )
        selected = files
    elif stage == "host":
        if payload != set(RELEASE_ARTIFACT_KINDS):
            raise ReleaseArtifactError(
                "downloaded build payload differs from the release plan"
            )
        selected = files
    elif stage == "publish":
        expected_payload = set(RELEASE_ARTIFACT_KINDS) | {CANONICAL_MANIFEST}
        if payload != expected_payload:
            raise ReleaseArtifactError(
                "publication payload differs from the release plan and host manifest"
            )
        selected = {name: path for name, path in files.items() if name in payload}
    else:
        raise ReleaseArtifactError("unknown release artifact assembly stage")

    _validate_checksums(files, stage)
    if destination.exists() or destination.is_symlink():
        raise ReleaseArtifactError("release artifact destination already exists")
    destination.mkdir(parents=True)
    for name in sorted(selected):
        _copy_regular(selected[name], destination / name)


def _hash_file(path: Path) -> tuple[int, str]:
    metadata = _regular_file_metadata(path, MAX_ARTIFACT_BYTES)
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        opened = os.fstat(stream.fileno())
        if (
            not stat.S_ISREG(opened.st_mode)
            or opened.st_dev != metadata.st_dev
            or opened.st_ino != metadata.st_ino
        ):
            raise ReleaseArtifactError(
                f"release artifact changed while it was opened: {path.name}"
            )
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
        final = os.fstat(stream.fileno())
    if final.st_size != opened.st_size or final.st_mtime_ns != opened.st_mtime_ns:
        raise ReleaseArtifactError(
            f"release artifact changed while it was hashed: {path.name}"
        )
    return opened.st_size, f"sha256:{digest.hexdigest()}"


def _checksum_entries(path: Path) -> dict[str, str]:
    """Read one bounded sha256sum-compatible file without accepting mutation."""

    metadata = _regular_file_metadata(path, MAX_MANIFEST_BYTES)
    with path.open("rb") as stream:
        opened = os.fstat(stream.fileno())
        if (
            not stat.S_ISREG(opened.st_mode)
            or opened.st_dev != metadata.st_dev
            or opened.st_ino != metadata.st_ino
        ):
            raise ReleaseArtifactError(
                f"release checksum changed while it was opened: {path.name}"
            )
        contents = stream.read(MAX_MANIFEST_BYTES + 1)
        final = os.fstat(stream.fileno())
    if len(contents) > MAX_MANIFEST_BYTES:
        raise ReleaseArtifactError(f"release checksum is too large: {path.name}")
    if final.st_size != opened.st_size or final.st_mtime_ns != opened.st_mtime_ns:
        raise ReleaseArtifactError(
            f"release checksum changed while it was read: {path.name}"
        )
    try:
        lines = contents.decode("ascii").splitlines()
    except UnicodeDecodeError as error:
        raise ReleaseArtifactError(
            f"release checksum is not ASCII: {path.name}"
        ) from error
    entries: dict[str, str] = {}
    for line in lines:
        if not line:
            continue
        match = re.fullmatch(r"([0-9a-f]{64}) \*([A-Za-z0-9._-]+)", line)
        if match is None:
            raise ReleaseArtifactError(
                f"release checksum has an invalid entry: {path.name}"
            )
        digest, name = match.groups()
        _validate_plain_name(name, "release checksum target")
        if name in entries:
            raise ReleaseArtifactError(
                f"release checksum contains a duplicate target: {path.name}"
            )
        entries[name] = digest
    if not entries:
        raise ReleaseArtifactError(f"release checksum is empty: {path.name}")
    return entries


def _validate_checksums(files: dict[str, Path], stage: str) -> None:
    """Verify every published checksum against the exact inventoried payload."""

    sidecars = {
        sidecar: target
        for sidecar, target in CHECKSUM_SIDECARS.items()
        if stage != "global" or sidecar in LOCAL_RELEASE_ARTIFACTS
    }
    expected: dict[str, str] = {}
    for sidecar, target in sidecars.items():
        entries = _checksum_entries(files[sidecar])
        if set(entries) != {target}:
            raise ReleaseArtifactError(
                f"release checksum names an unexpected target: {sidecar}"
            )
        _, actual = _hash_file(files[target])
        digest = actual.removeprefix("sha256:")
        if entries[target] != digest:
            raise ReleaseArtifactError(
                f"release checksum does not match its artifact: {sidecar}"
            )
        expected[target] = digest
    if stage in {"host", "publish"}:
        unified = _checksum_entries(files["sha256.sum"])
        if unified != expected:
            raise ReleaseArtifactError(
                "unified release checksum differs from the verified sidecars"
            )


def verify_remote_release(
    artifact_root: Path, release_json: Path, expected_tag: str
) -> None:
    """Bind a private draft's exact server-side assets to the local attestation input."""

    local_files: dict[str, Path] = {}
    root_metadata = artifact_root.lstat()
    if stat.S_ISLNK(root_metadata.st_mode) or not stat.S_ISDIR(root_metadata.st_mode):
        raise ReleaseArtifactError("local publication payload is not a directory")
    for path in artifact_root.iterdir():
        name = _validate_plain_name(path.name, "local publication artifact")
        if name in local_files:
            raise ReleaseArtifactError("local publication artifact is duplicated")
        _regular_file_metadata(path, MAX_ARTIFACT_BYTES)
        local_files[name] = path
    expected_names = set(RELEASE_ARTIFACT_KINDS) | {CANONICAL_MANIFEST}
    if set(local_files) != expected_names:
        raise ReleaseArtifactError(
            "local publication payload is not the exact release set"
        )
    if (
        sum(path.stat().st_size for path in local_files.values())
        > MAX_TOTAL_ARTIFACT_BYTES
    ):
        raise ReleaseArtifactError("local publication payload exceeds its size limit")

    release = _read_json(release_json)
    if (
        release.get("tag_name") != expected_tag
        or release.get("draft") is not True
        or release.get("prerelease") is not True
    ):
        raise ReleaseArtifactError(
            "remote release is not the expected private prerelease draft"
        )
    assets = release.get("assets")
    if not isinstance(assets, list):
        raise ReleaseArtifactError("remote release asset inventory is not a list")
    remote: dict[str, dict[str, Any]] = {}
    for asset in assets:
        if not isinstance(asset, dict):
            raise ReleaseArtifactError("remote release asset entry is not an object")
        name = _validate_plain_name(asset.get("name"), "remote release asset")
        if name in remote:
            raise ReleaseArtifactError("remote release contains duplicate asset names")
        remote[name] = asset
    if set(remote) != expected_names:
        raise ReleaseArtifactError(
            "remote release asset names differ from the local payload"
        )
    for name in sorted(expected_names):
        size, digest = _hash_file(local_files[name])
        asset = remote[name]
        if (
            asset.get("state") != "uploaded"
            or asset.get("size") != size
            or asset.get("digest") != digest
        ):
            raise ReleaseArtifactError(
                f"remote release asset does not match the local payload: {name}"
            )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    prepare = subparsers.add_parser("prepare")
    prepare.add_argument("--input-root", type=Path, required=True)
    prepare.add_argument("--destination", type=Path, required=True)
    prepare.add_argument(
        "--stage", choices=("global", "host", "publish"), required=True
    )
    prepare.add_argument("--expected-tag")
    prepare.add_argument("--global-result", choices=("success", "skipped"))
    remote = subparsers.add_parser("verify-remote")
    remote.add_argument("--artifact-root", type=Path, required=True)
    remote.add_argument("--release-json", type=Path, required=True)
    remote.add_argument("--expected-tag", required=True)
    arguments = parser.parse_args()

    try:
        if arguments.command == "prepare":
            prepare_artifacts(
                arguments.input_root,
                arguments.destination,
                arguments.stage,
                arguments.expected_tag,
                arguments.global_result,
            )
        else:
            verify_remote_release(
                arguments.artifact_root,
                arguments.release_json,
                arguments.expected_tag,
            )
    except (OSError, ReleaseArtifactError) as error:
        parser.exit(2, f"release artifact validation failed: {error}\n")


if __name__ == "__main__":
    main()
