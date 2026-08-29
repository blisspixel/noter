"""Tests for the fail-closed release artifact boundary."""

from __future__ import annotations

import hashlib
import json
import tempfile
import unittest
from pathlib import Path

import check_release_artifacts as artifacts


def release_plan(*, global_matrix: bool = False) -> dict[str, object]:
    github: dict[str, object] = {
        "artifacts_matrix": {
            "include": [
                {
                    "runner": artifacts.TARGET_RUNNERS[target],
                    "targets": [target],
                }
                for target in artifacts.RELEASE_TARGETS
            ]
        }
    }
    if global_matrix:
        github["global_artifacts_matrix"] = {"include": [{"runner": "ubuntu-22.04"}]}
    return {
        "dist_version": artifacts.DIST_VERSION,
        "announcement_tag": "v0.1.0-alpha.2",
        "announcement_is_prerelease": True,
        "releases": [
            {
                "app_name": artifacts.APP_NAME,
                "app_version": artifacts.APP_VERSION,
                "artifacts": list(artifacts.RELEASE_ARTIFACT_KINDS),
            }
        ],
        "artifacts": {
            name: {"name": name, "kind": kind}
            for name, kind in artifacts.RELEASE_ARTIFACT_KINDS.items()
        },
        "ci": {"github": github},
    }


def write_bytes(path: Path, contents: bytes = b"artifact\n") -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(contents)


def write_json(path: Path, value: object) -> None:
    write_bytes(path, json.dumps(value, sort_keys=True).encode("utf-8"))


def write_checksum(path: Path, target: Path) -> None:
    digest = hashlib.sha256(target.read_bytes()).hexdigest()
    write_bytes(path, f"{digest} *{target.name}\n\n".encode("ascii"))


def create_downloads(
    root: Path, *, include_global: bool = True, include_host: bool = False
) -> None:
    write_json(
        root / artifacts.PLAN_CONTAINER / "plan-dist-manifest.json",
        release_plan(),
    )
    local_by_target = {
        target: [
            name
            for name, owner in artifacts.LOCAL_RELEASE_ARTIFACT_TARGETS.items()
            if owner == target
        ]
        for target in artifacts.RELEASE_TARGETS
    }
    for target in artifacts.RELEASE_TARGETS:
        container = root / f"artifacts-build-local-{target}"
        write_json(container / f"{target}-dist-manifest.json", {})
        for name in local_by_target[target]:
            if name not in artifacts.CHECKSUM_SIDECARS:
                write_bytes(container / name, name.encode("ascii"))
        for sidecar, artifact in artifacts.CHECKSUM_SIDECARS.items():
            if sidecar in local_by_target[target]:
                write_checksum(container / sidecar, container / artifact)
    if include_global:
        container = root / artifacts.GLOBAL_CONTAINER
        write_json(container / "global-dist-manifest.json", {})
        for name in set(artifacts.RELEASE_ARTIFACT_KINDS) - set(
            artifacts.LOCAL_RELEASE_ARTIFACTS
        ):
            if name not in artifacts.CHECKSUM_SIDECARS and name != "sha256.sum":
                write_bytes(container / name, name.encode("ascii"))
        write_checksum(container / "source.tar.gz.sha256", container / "source.tar.gz")
        payload = {
            path.name: path
            for path in root.glob("artifacts-build-*/*")
            if path.is_file()
        }
        unified = "".join(
            f"{hashlib.sha256(payload[target].read_bytes()).hexdigest()} *{target}\n"
            for target in sorted(artifacts.CHECKSUM_SIDECARS.values())
        )
        write_bytes(container / "sha256.sum", f"{unified}\n".encode("ascii"))
    if include_host:
        write_json(root / artifacts.HOST_CONTAINER / artifacts.CANONICAL_MANIFEST, {})


class ReleaseArtifactTests(unittest.TestCase):
    def test_validates_the_exact_pinned_plan_schema(self) -> None:
        self.assertIsNone(artifacts.validate_plan(release_plan(), "v0.1.0-alpha.2"))

    def test_rejects_a_plan_missing_a_required_sbom(self) -> None:
        plan = release_plan()
        release = plan["releases"][0]
        self.assertIsInstance(release, dict)
        release["artifacts"].remove("noter_aarch64-apple-darwin.cdx.xml")
        with self.assertRaisesRegex(
            artifacts.ReleaseArtifactError, "required artifact set"
        ):
            artifacts.validate_plan(plan)

    def test_rejects_a_plan_with_a_wrong_target_runner(self) -> None:
        plan = release_plan()
        plan["ci"]["github"]["artifacts_matrix"]["include"][0]["runner"] = (
            "ubuntu-latest"
        )
        with self.assertRaisesRegex(artifacts.ReleaseArtifactError, "target runner"):
            artifacts.validate_plan(plan)

    def test_prepares_the_exact_local_payload_for_global_build(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            inputs = root / "inputs"
            create_downloads(inputs, include_global=False)
            destination = root / "distrib"

            artifacts.prepare_artifacts(inputs, destination, "global")

            names = {path.name for path in destination.iterdir()}
            self.assertTrue(artifacts.LOCAL_RELEASE_ARTIFACTS.issubset(names))
            self.assertIn("plan-dist-manifest.json", names)

    def test_rejects_a_cross_container_filename_collision_before_copying(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            inputs = root / "inputs"
            create_downloads(inputs)
            collision = (
                inputs / "artifacts-build-local-aarch64-apple-darwin" / "source.tar.gz"
            )
            write_bytes(collision, b"colliding source")
            destination = root / "distrib"

            with self.assertRaisesRegex(
                artifacts.ReleaseArtifactError, "filename collision"
            ):
                artifacts.prepare_artifacts(
                    inputs,
                    destination,
                    "host",
                    "v0.1.0-alpha.2",
                    "success",
                )
            self.assertFalse(destination.exists())

    def test_rejects_any_target_artifact_owned_by_another_build(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            inputs = root / "inputs"
            create_downloads(inputs, include_global=False)
            cases = (
                "noter-aarch64-apple-darwin.tar.xz",
                "noter-aarch64-apple-darwin.tar.xz.sha256",
            )
            for name in cases:
                with self.subTest(name=name):
                    source = (
                        inputs / "artifacts-build-local-aarch64-apple-darwin" / name
                    )
                    destination = (
                        inputs / "artifacts-build-local-x86_64-apple-darwin" / name
                    )
                    source.replace(destination)
                    with self.assertRaisesRegex(
                        artifacts.ReleaseArtifactError,
                        "missing from its target build",
                    ):
                        artifacts.prepare_artifacts(
                            inputs, root / f"distrib-{name}", "global"
                        )
                    destination.replace(source)

    def test_requires_every_target_sbom_from_the_global_build(self) -> None:
        """The named-per-target SBOMs are produced once, on the global runner.

        `dist` models `extra-artifacts` as a single global step, so a local
        build cannot supply them and the global container must.
        """

        for target in artifacts.RELEASE_TARGETS:
            name = f"noter_{target}.cdx.xml"
            self.assertIn(name, artifacts.GLOBAL_RELEASE_ARTIFACTS)
            self.assertNotIn(name, artifacts.LOCAL_RELEASE_ARTIFACTS)

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            inputs = root / "inputs"
            create_downloads(inputs)
            missing = (
                inputs
                / artifacts.GLOBAL_CONTAINER
                / "noter_aarch64-apple-darwin.cdx.xml"
            )
            missing.unlink()

            with self.assertRaisesRegex(
                artifacts.ReleaseArtifactError,
                "missing from its global build",
            ):
                artifacts.prepare_artifacts(
                    inputs, root / "distrib", "host", "v0.1.0-alpha.2", "success"
                )

    def test_rejects_a_global_artifact_in_a_local_build(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            inputs = root / "inputs"
            create_downloads(inputs, include_global=False)
            write_bytes(
                inputs / "artifacts-build-local-aarch64-apple-darwin" / "source.tar.gz",
                b"wrong producer",
            )

            with self.assertRaisesRegex(
                artifacts.ReleaseArtifactError,
                "planned local artifact set",
            ):
                artifacts.prepare_artifacts(inputs, root / "distrib", "global")

    def test_rejects_a_sidecar_that_does_not_match_its_artifact(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            inputs = root / "inputs"
            create_downloads(inputs, include_global=False)
            artifact = (
                inputs
                / "artifacts-build-local-aarch64-apple-darwin"
                / "noter-aarch64-apple-darwin.tar.xz"
            )
            write_bytes(artifact, b"changed after checksum")

            with self.assertRaisesRegex(
                artifacts.ReleaseArtifactError,
                "does not match its artifact",
            ):
                artifacts.prepare_artifacts(inputs, root / "distrib", "global")

    def test_rejects_an_incomplete_unified_checksum(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            inputs = root / "inputs"
            create_downloads(inputs)
            unified = inputs / artifacts.GLOBAL_CONTAINER / "sha256.sum"
            lines = unified.read_text(encoding="ascii").splitlines()
            write_bytes(unified, f"{lines[0]}\n\n".encode("ascii"))

            with self.assertRaisesRegex(
                artifacts.ReleaseArtifactError,
                "unified release checksum differs",
            ):
                artifacts.prepare_artifacts(
                    inputs,
                    root / "distrib",
                    "host",
                    "v0.1.0-alpha.2",
                    "success",
                )

    def test_rejects_a_required_global_build_that_was_skipped(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            inputs = root / "inputs"
            create_downloads(inputs, include_global=False)
            with self.assertRaisesRegex(
                artifacts.ReleaseArtifactError, "global artifact build did not succeed"
            ):
                artifacts.prepare_artifacts(
                    inputs,
                    root / "distrib",
                    "host",
                    "v0.1.0-alpha.2",
                    "skipped",
                )

    def test_publish_stage_excludes_granular_manifests(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            inputs = root / "inputs"
            create_downloads(inputs, include_host=True)
            destination = root / "publication"

            artifacts.prepare_artifacts(
                inputs,
                destination,
                "publish",
                "v0.1.0-alpha.2",
                "success",
            )

            self.assertEqual(
                {path.name for path in destination.iterdir()},
                set(artifacts.RELEASE_ARTIFACT_KINDS) | {artifacts.CANONICAL_MANIFEST},
            )

    def test_remote_draft_must_match_every_local_digest(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            inputs = root / "inputs"
            create_downloads(inputs, include_host=True)
            publication = root / "publication"
            artifacts.prepare_artifacts(
                inputs,
                publication,
                "publish",
                "v0.1.0-alpha.2",
                "success",
            )
            assets = []
            for path in publication.iterdir():
                contents = path.read_bytes()
                assets.append(
                    {
                        "name": path.name,
                        "state": "uploaded",
                        "size": len(contents),
                        "digest": f"sha256:{hashlib.sha256(contents).hexdigest()}",
                    }
                )
            release_json = root / "release.json"
            write_json(
                release_json,
                {
                    "tag_name": "v0.1.0-alpha.2",
                    "draft": True,
                    "prerelease": True,
                    "assets": assets,
                },
            )

            artifacts.verify_remote_release(publication, release_json, "v0.1.0-alpha.2")

            original = assets[0].copy()
            mutations = {
                "digest": f"sha256:{'0' * 64}",
                "size": original["size"] + 1,
                "state": "new",
            }
            for field, value in mutations.items():
                with self.subTest(field=field):
                    assets[0] = original | {field: value}
                    write_json(
                        release_json,
                        {
                            "tag_name": "v0.1.0-alpha.2",
                            "draft": True,
                            "prerelease": True,
                            "assets": assets,
                        },
                    )
                    with self.assertRaisesRegex(
                        artifacts.ReleaseArtifactError,
                        "does not match the local payload",
                    ):
                        artifacts.verify_remote_release(
                            publication, release_json, "v0.1.0-alpha.2"
                        )
            assets[0] = original

    def test_remote_draft_rejects_missing_and_pending_assets(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            publication = root / "publication"
            publication.mkdir()
            for name in set(artifacts.RELEASE_ARTIFACT_KINDS) | {
                artifacts.CANONICAL_MANIFEST
            }:
                write_bytes(publication / name, name.encode("ascii"))
            remote_assets = []
            for path in publication.iterdir():
                contents = path.read_bytes()
                remote_assets.append(
                    {
                        "name": path.name,
                        "state": "uploaded",
                        "size": len(contents),
                        "digest": f"sha256:{hashlib.sha256(contents).hexdigest()}",
                    }
                )
            remote_assets.pop()
            release_json = root / "release.json"
            write_json(
                release_json,
                {
                    "tag_name": "v0.1.0-alpha.2",
                    "draft": True,
                    "prerelease": True,
                    "assets": remote_assets,
                },
            )
            with self.assertRaisesRegex(
                artifacts.ReleaseArtifactError, "asset names differ"
            ):
                artifacts.verify_remote_release(
                    publication, release_json, "v0.1.0-alpha.2"
                )


if __name__ == "__main__":
    unittest.main()
