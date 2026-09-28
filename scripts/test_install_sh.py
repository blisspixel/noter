"""Native release-installer staging and directory-authority regressions."""

from __future__ import annotations

import hashlib
import io
import os
import stat
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path


@unittest.skipUnless(os.name == "posix", "the POSIX installer needs a Unix host")
class ReleaseInstallerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(
            prefix="noter-install-test-", dir=Path.home()
        )
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.bin_dir = self.root / "install" / "bin"
        self.bin_dir.mkdir(parents=True)

        archive = self.root / "release.tar.xz"
        executable = b"#!/bin/sh\nprintf 'noter 0.1.0-beta.1\\n'\n"
        entry = tarfile.TarInfo("noter")
        entry.mode = 0o755
        entry.size = len(executable)
        with tarfile.open(archive, "w:xz") as bundle:
            bundle.addfile(entry, io.BytesIO(executable))
        sidecar = self.root / "release.tar.xz.sha256"
        sidecar.write_text(
            f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  release.tar.xz\n"
        )

        mock_bin = self.root / "mock-bin"
        mock_bin.mkdir()
        mock_curl = mock_bin / "curl"
        mock_curl.write_text(
            "#!/usr/bin/env python3\n"
            "import os, pathlib, sys\n"
            "if os.environ.get('TEST_POISON_STAGE') == '1' and not sys.argv[-1].endswith('.sha256'):\n"
            "    stage = pathlib.Path(os.environ['TEST_BIN_DIR']) / f'.noter.install.{os.getppid()}'\n"
            "    stage.symlink_to(os.environ['TEST_VICTIM'])\n"
            "source = os.environ['TEST_SIDECAR'] if sys.argv[-1].endswith('.sha256') else os.environ['TEST_ARCHIVE']\n"
            "sys.stdout.buffer.write(pathlib.Path(source).read_bytes())\n"
        )
        mock_curl.chmod(0o755)
        self.environment = os.environ.copy()
        self.environment.update(
            PATH=f"{mock_bin}{os.pathsep}{self.environment['PATH']}",
            TEST_ARCHIVE=str(archive),
            TEST_SIDECAR=str(sidecar),
            TEST_BIN_DIR=str(self.bin_dir),
        )

    def install(self, root: Path | None = None) -> subprocess.CompletedProcess[str]:
        installer = Path(__file__).with_name("install.sh")
        process = subprocess.Popen(
            [
                "sh",
                str(installer),
                "--binary",
                "--version",
                "0.1.0-beta.1",
                "--root",
                str(self.bin_dir.parent if root is None else root),
            ],
            cwd=self.root,
            env=self.environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        self.install_pid = process.pid
        stdout, stderr = process.communicate(timeout=30)
        return subprocess.CompletedProcess(
            process.args, process.returncode, stdout, stderr
        )

    def test_predictable_stage_symlink_cannot_redirect_copy(self) -> None:
        victim = self.root / "victim"
        victim.write_text("keep me")
        original_mode = stat.S_IMODE(victim.stat().st_mode)
        self.environment.update(TEST_POISON_STAGE="1", TEST_VICTIM=str(victim))

        result = self.install()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(victim.read_text(), "keep me")
        self.assertEqual(stat.S_IMODE(victim.stat().st_mode), original_mode)
        self.assertTrue(
            (self.bin_dir / f".noter.install.{self.install_pid}").is_symlink()
        )
        self.assertTrue((self.bin_dir / "noter").is_file())
        self.assertFalse(list(self.bin_dir.glob(".noter.install.????????")))

    def test_group_or_other_writable_install_directory_is_refused(self) -> None:
        for mode in (0o777, 0o1777):
            with self.subTest(mode=oct(mode)):
                self.bin_dir.chmod(mode)
                result = self.install()

                self.assertNotEqual(result.returncode, 0)
                self.assertIn("writable by another user", result.stderr)
                self.assertFalse((self.bin_dir / "noter").exists())

    def test_requested_symlink_in_writable_ancestor_is_refused(self) -> None:
        shared = self.root / "shared"
        shared.mkdir()
        shared.chmod(0o1777)
        redirected = self.root / "redirected"
        (redirected / "bin").mkdir(parents=True)
        link = shared / "chosen-root"
        link.symlink_to(redirected, target_is_directory=True)

        result = self.install(root=link)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("writable by another user", result.stderr)
        self.assertFalse((redirected / "bin" / "noter").exists())

    def test_plain_child_in_writable_ancestor_is_refused(self) -> None:
        shared = self.root / "shared"
        child = shared / "chosen-root"
        (child / "bin").mkdir(parents=True)
        shared.chmod(0o777)

        result = self.install(root=child)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("writable by another user", result.stderr)
        self.assertFalse((child / "bin" / "noter").exists())

    def test_symlink_in_private_ancestor_can_install(self) -> None:
        link = self.root / "chosen-root"
        link.symlink_to(self.bin_dir.parent, target_is_directory=True)

        result = self.install(root=link)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.bin_dir / "noter").is_file())

    @unittest.skipUnless(sys.platform == "darwin", "macOS ACL semantics")
    def test_macos_acl_hidden_by_extended_attributes_is_refused(self) -> None:
        subprocess.run(
            ["chmod", "+a", "everyone allow add_file,delete_child", str(self.bin_dir)],
            check=True,
        )
        subprocess.run(
            ["xattr", "-w", "com.noter.install-fixture", "yes", str(self.bin_dir)],
            check=True,
        )

        result = self.install()

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unsupported access control list", result.stderr)
        self.assertFalse((self.bin_dir / "noter").exists())


if __name__ == "__main__":
    unittest.main()
