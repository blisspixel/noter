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


@unittest.skipUnless(os.name == "posix", "the POSIX installer needs a Unix host")
class SourceInstallerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(
            prefix="noter-source-install-test-", dir=Path.home()
        )
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.install_root = self.root / "install"
        self.bin_dir = self.install_root / "bin"
        self.bin_dir.mkdir(parents=True)
        self.installed = self.bin_dir / "noter"
        self.installed.write_text("last working binary")
        self.source = self.root / "source"
        self.source.mkdir()
        (self.source / "Cargo.toml").write_text('name = "noter"\n')
        self.built = self.root / "built-noter"
        self.built.write_text(
            "#!/bin/sh\n"
            'case "$1" in\n'
            "  --version) printf 'noter 0.1.0-beta.1\\n' ;;\n"
            "  --theme) printf 'unknown theme `invalid`; expected system, light, dark, green, or amber\\nUsage: noter\\n' >&2; exit 2 ;;\n"
            "esac\n"
        )
        self.built.chmod(0o755)
        self.cargo_root_log = self.root / "cargo-root"
        mock_bin = self.root / "mock-bin"
        mock_bin.mkdir()
        cargo = mock_bin / "cargo"
        cargo.write_text(
            "#!/bin/sh\n"
            'case "$1" in\n'
            '  metadata) printf \'{"packages":[{"name":"noter","version":"0.1.0-beta.1"}]}\\n\' ;;\n'
            "  install)\n"
            '    while [ "$#" -gt 0 ]; do\n'
            '      if [ "$1" = --root ]; then shift; root=$1; break; fi\n'
            "      shift\n"
            "    done\n"
            '    printf \'%s\' "$root" >"$TEST_CARGO_ROOT_LOG"\n'
            '    mkdir -p "$root/bin"\n'
            '    cp "$TEST_SOURCE_BINARY" "$root/bin/noter"\n'
            '    chmod 755 "$root/bin/noter" ;;\n'
            "  *) exit 2 ;;\n"
            "esac\n"
        )
        cargo.chmod(0o755)
        self.environment = os.environ.copy()
        self.environment.update(
            PATH=f"{mock_bin}{os.pathsep}{self.environment['PATH']}",
            TEST_SOURCE_BINARY=str(self.built),
            TEST_CARGO_ROOT_LOG=str(self.cargo_root_log),
        )

    def install(self) -> subprocess.CompletedProcess[str]:
        installer = Path(__file__).with_name("install.sh")
        return subprocess.run(
            [
                "sh",
                str(installer),
                "--source",
                str(self.source),
                "--root",
                str(self.install_root),
            ],
            cwd=self.root,
            env=self.environment,
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )

    def test_bad_staged_version_preserves_previous_executable(self) -> None:
        self.built.write_text("#!/bin/sh\nprintf 'noter wrong-version\\n'\n")
        self.built.chmod(0o755)

        result = self.install()

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("staged executable did not report", result.stderr)
        self.assertEqual(self.installed.read_text(), "last working binary")
        self.assertFalse(Path(self.cargo_root_log.read_text()).exists())

    def test_verified_source_binary_replaces_previous_executable(self) -> None:
        result = self.install()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.installed.read_bytes(), self.built.read_bytes())
        self.assertFalse(Path(self.cargo_root_log.read_text()).exists())
        self.assertFalse(list(self.bin_dir.glob(".noter.install.????????")))


if __name__ == "__main__":
    unittest.main()
