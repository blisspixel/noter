"""Failure-path regressions for the native recovery identity fixture."""

from __future__ import annotations

import errno
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import check_recovery_identity as fixture


class RecoveryIdentityTests(unittest.TestCase):
    def test_only_access_denial_counts_as_private(self):
        fixture.expect_denied(
            mock.Mock(side_effect=PermissionError(errno.EACCES, "denied"))
        )
        for error in (
            FileNotFoundError(errno.ENOENT, "absent"),
            OSError(errno.EIO, "I/O"),
        ):
            with (
                self.subTest(error=error),
                self.assertRaisesRegex(AssertionError, "other than"),
            ):
                fixture.expect_denied(mock.Mock(side_effect=error))
        with self.assertRaisesRegex(AssertionError, "accessed private"):
            fixture.expect_denied(lambda: None)

    def test_artifact_selection_rejects_missing_duplicate_and_wrong_targets(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            executable = root / "tests"
            executable.touch()
            valid = {
                "reason": "compiler-artifact",
                "target": {"name": "noter", "kind": ["lib"]},
                "profile": {"test": True},
                "executable": str(executable),
            }
            artifacts = root / "artifacts.json"
            artifacts.write_text(json.dumps(valid))
            self.assertEqual(fixture.test_executable(artifacts), executable)
            wrong = dict(valid, target={"name": "noter", "kind": ["bin"]})
            for messages in (
                [],
                [valid, valid],
                [wrong],
                [dict(valid, profile={"test": False})],
            ):
                with self.subTest(messages=messages), self.assertRaises(ValueError):
                    artifacts.write_text(
                        "\n".join(json.dumps(message) for message in messages)
                    )
                    fixture.test_executable(artifacts)
            executable.unlink()
            artifacts.write_text(json.dumps(valid))
            with self.assertRaises(ValueError):
                fixture.test_executable(artifacts)

    def test_effective_identity_rejects_root_groups_capabilities_and_wrong_ids(self):
        with mock.patch.multiple(
            fixture.os,
            create=True,
            getuid=mock.Mock(return_value=200001),
            geteuid=mock.Mock(return_value=200001),
            getgid=mock.Mock(return_value=200001),
            getegid=mock.Mock(return_value=200001),
            getgroups=mock.Mock(return_value=[]),
        ):
            status = {
                name: "0" for name in ("CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb")
            }
            status["NoNewPrivs"] = "1"

            def read_status(*_args, **_kwargs):
                return "\n".join(f"{key}: {value}" for key, value in status.items())

            with mock.patch.object(Path, "read_text", side_effect=read_status):
                self.assertEqual(
                    fixture.require_identity(200001, 200001)["euid"], 200001
                )
                for uid, gid in ((0, 200001), (200002, 200001), (200001, 200002)):
                    with (
                        self.subTest(uid=uid, gid=gid),
                        self.assertRaises(AssertionError),
                    ):
                        fixture.require_identity(uid, gid)
                for name in status:
                    original = status[name]
                    status[name] = "0" if name == "NoNewPrivs" else "1"
                    with self.subTest(name=name), self.assertRaises(AssertionError):
                        fixture.require_identity(200001, 200001)
                    status[name] = original
                fixture.os.getgroups.return_value = [0]
                with self.assertRaises(AssertionError):
                    fixture.require_identity(200001, 200001)

    def test_owner_exit_never_counts_as_ready(self):
        with tempfile.TemporaryDirectory() as temporary:
            marker = Path(temporary) / "ready"
            marker.write_bytes(fixture.READY)
            with self.assertRaisesRegex(AssertionError, "exited"):
                fixture.wait_ready(mock.Mock(poll=mock.Mock(return_value=0)), marker)
            with mock.patch.object(fixture.time, "monotonic", side_effect=[0, 31]):
                with self.assertRaises(TimeoutError):
                    fixture.wait_ready(
                        mock.Mock(poll=mock.Mock(return_value=None)), marker
                    )

    def test_missing_or_wrong_control_fails_before_denial_attempts(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "control").mkdir()
            with (
                mock.patch.object(fixture, "require_identity", return_value={}),
                mock.patch.object(fixture, "expect_denied") as denied,
            ):
                with self.assertRaises(FileNotFoundError):
                    fixture.probe(root, 200001, 200001)
                (root / "control/original").write_bytes(b"wrong control")
                with self.assertRaises(AssertionError):
                    fixture.probe(root, 200001, 200001)
                denied.assert_not_called()

    def test_capability_command_covers_the_actual_kernel_range(self):
        with (
            mock.patch.object(fixture.shutil, "which", return_value="/usr/bin/setpriv"),
            mock.patch.object(Path, "read_text", return_value="40"),
        ):
            command = fixture.identity_command(200001, 200002, ["probe.py"])
            self.assertIn("--clear-groups", command)
            self.assertIn("--no-new-privs", command)
            for prefix in ("--inh-caps=", "--ambient-caps=", "--bounding-set="):
                self.assertIn(
                    prefix + ",".join(f"-cap_{index}" for index in range(41)), command
                )
            with mock.patch.object(Path, "read_text", return_value="64"):
                with self.assertRaises(RuntimeError):
                    fixture.identity_command(200001, 200002, ["probe.py"])

    def test_disposable_opt_in_is_required_before_any_privileged_operation(self):
        with mock.patch.object(fixture, "test_executable") as executable:
            with self.assertRaisesRegex(RuntimeError, "explicit"):
                fixture.run(Path("unused"), False)
            executable.assert_not_called()


if __name__ == "__main__":
    unittest.main()
