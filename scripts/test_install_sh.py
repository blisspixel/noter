"""Native release-installer staging and directory-authority regressions."""

from __future__ import annotations

import hashlib
import io
import json
import os
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
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
        self.data_home = self.root / "xdg-data"
        self.data_home.mkdir(mode=0o700)
        self.data_home.chmod(0o700)
        self.environment = os.environ.copy()
        self.environment.update(
            PATH=f"{mock_bin}{os.pathsep}{self.environment['PATH']}",
            TEST_ARCHIVE=str(archive),
            TEST_SIDECAR=str(sidecar),
            TEST_BIN_DIR=str(self.bin_dir),
            XDG_DATA_HOME=str(self.data_home),
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
        self.data_home = self.root / "xdg-data"
        self.data_home.mkdir(mode=0o700)
        self.data_home.chmod(0o700)
        self.environment = os.environ.copy()
        self.environment.update(
            PATH=f"{mock_bin}{os.pathsep}{self.environment['PATH']}",
            TEST_SOURCE_BINARY=str(self.built),
            TEST_CARGO_ROOT_LOG=str(self.cargo_root_log),
            XDG_DATA_HOME=str(self.data_home),
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

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux desktop integration")
    def test_source_install_includes_the_selected_file_argument(self) -> None:
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        desktop = self.data_home / "applications" / "noter.desktop"
        self.assertIn(
            f'Exec=/usr/bin/env -- "{self.installed}" %f\n', desktop.read_text()
        )


def embedded_heredoc(script: str, function_name: str) -> str:
    signature = f"{function_name}() {{"
    start = script.index(signature)
    marker = script.index("<<'EOF'\n", start)
    body = marker + len("<<'EOF'\n")
    end = script.index("\nEOF\n", body)
    return script[body : end + 1]


class DesktopAssetTests(unittest.TestCase):
    def test_installer_embeds_the_packaging_desktop_files(self) -> None:
        script = Path(__file__).with_name("install.sh").read_text(encoding="utf-8")
        root = Path(__file__).resolve().parents[1]
        self.assertEqual(
            embedded_heredoc(script, "desktop_template"),
            (root / "packaging" / "linux" / "noter.desktop.in").read_text(
                encoding="utf-8"
            ),
        )
        self.assertEqual(
            embedded_heredoc(script, "icon_svg"),
            (root / "packaging" / "linux" / "noter.svg").read_text(encoding="utf-8"),
        )


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux desktop integration")
class LinuxDesktopInstallerTests(ReleaseInstallerTests):
    def desktop_path(self) -> Path:
        return self.data_home / "applications" / "noter.desktop"

    def icon_path(self) -> Path:
        return self.data_home / "icons" / "hicolor" / "scalable" / "apps" / "noter.svg"

    def uninstall(self, root: Path) -> subprocess.CompletedProcess[str]:
        installer = Path(__file__).with_name("install.sh")
        return subprocess.run(
            ["sh", str(installer), "--binary", "--uninstall", "--root", str(root)],
            cwd=self.root,
            env=self.environment,
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )

    def test_binary_install_quotes_exec_and_uninstall_removes_owned_files(self) -> None:
        spaced = self.root / "install dir"
        result = self.install(root=spaced)
        self.assertEqual(result.returncode, 0, result.stderr)
        binary = spaced / "bin" / "noter"
        self.assertTrue(binary.is_file())
        desktop = self.desktop_path().read_text(encoding="utf-8")
        self.assertIn(f'Exec=/usr/bin/env -- "{binary}" %f\n', desktop)
        self.assertIn("X-Noter-Installer=true\n", desktop)
        self.assertNotIn("@EXEC@", desktop)
        self.assertIn("noter-installer", self.icon_path().read_text(encoding="utf-8"))

        removed = self.uninstall(spaced)
        self.assertEqual(removed.returncode, 0, removed.stderr)
        self.assertFalse(binary.exists())
        self.assertFalse(self.desktop_path().exists())
        self.assertFalse(self.icon_path().exists())

    def test_uninstall_preserves_another_install_roots_launcher_and_icon(self) -> None:
        first = self.root / "first"
        second = self.root / "second"
        self.assertEqual(self.install(root=first).returncode, 0)
        self.assertEqual(self.install(root=second).returncode, 0)
        desktop = self.desktop_path().read_bytes()
        icon = self.icon_path().read_bytes()

        removed = self.uninstall(first)

        self.assertEqual(removed.returncode, 0, removed.stderr)
        self.assertFalse((first / "bin" / "noter").exists())
        self.assertTrue((second / "bin" / "noter").is_file())
        self.assertEqual(self.desktop_path().read_bytes(), desktop)
        self.assertEqual(self.icon_path().read_bytes(), icon)
        self.assertEqual(self.uninstall(second).returncode, 0)
        self.assertFalse(self.desktop_path().exists())
        self.assertFalse(self.icon_path().exists())

    def test_exec_uses_both_escape_layers_and_literal_percent(self) -> None:
        special = self.root / 'notes 雪 % \\ $ " `'
        result = self.install(root=special)
        self.assertEqual(result.returncode, 0, result.stderr)
        escaped = str(self.root) + r'/notes 雪 %% \\\\ \\$ \\" \\`/bin/noter'
        self.assertIn(
            f'Exec=/usr/bin/env -- "{escaped}" %f\n', self.desktop_path().read_text()
        )
        self.assertEqual(self.uninstall(special).returncode, 0)
        self.assertFalse(self.desktop_path().exists())
        self.assertFalse(self.icon_path().exists())

    @unittest.skipUnless(shutil.which("gio"), "GIO desktop launch integration")
    def test_open_with_delivers_the_exact_selected_path(self) -> None:
        special = self.root / 'notes 雪 % \\ $ " `'
        result = self.install(root=special)
        self.assertEqual(result.returncode, 0, result.stderr)
        binary = special / "bin" / "noter"
        binary.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, pathlib, sys\n"
            "pathlib.Path(os.environ['TEST_LAUNCH_ARGUMENTS']).write_text(json.dumps(sys.argv[1:]))\n"
        )
        selected = self.root / 'selected 雪 = % \\ $ " `.md'
        selected.write_text("private fixture")
        arguments = self.root / "launch-arguments.json"
        environment = dict(self.environment, TEST_LAUNCH_ARGUMENTS=str(arguments))
        command = ["gio", "launch", str(self.desktop_path()), str(selected)]
        help_result = subprocess.run(
            ["gio", "help", "launch"], capture_output=True, check=False
        )
        if help_result.returncode != 0:
            # Older GLib provides DesktopAppInfo without the gio launch command.
            command = [
                sys.executable,
                "-c",
                "from gi.repository import Gio; import sys; "
                "app = Gio.DesktopAppInfo.new_from_filename(sys.argv[1]); "
                "assert app is not None; "
                "assert app.launch([Gio.File.new_for_path(sys.argv[2])], None)",
                str(self.desktop_path()),
                str(selected),
            ]
        launched = subprocess.run(
            command,
            env=environment,
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
        self.assertEqual(launched.returncode, 0, launched.stderr)
        # GIO returns after spawning, before the child has necessarily written.
        deadline = time.monotonic() + 10
        while not arguments.exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertEqual(json.loads(arguments.read_text()), [str(selected)])

    def test_unsupported_exec_paths_are_refused_before_launcher_writes(
        self,
    ) -> None:
        for control in ("\n", "\r", "\t", "="):
            with self.subTest(control=repr(control)):
                result = self.install(root=self.root / f"invalid{control}root")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("cannot contain control characters or '='", result.stderr)
                self.assertFalse(self.desktop_path().exists())
                self.assertFalse(self.icon_path().exists())

    def test_uninstall_removes_a_legacy_owned_launcher_and_icon(self) -> None:
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        binary = self.bin_dir / "noter"
        self.desktop_path().write_text(
            f'Exec="{binary}"\nX-Noter-Installer=true\n', encoding="utf-8"
        )
        removed = self.uninstall(self.bin_dir.parent)
        self.assertEqual(removed.returncode, 0, removed.stderr)
        self.assertFalse(self.desktop_path().exists())
        self.assertFalse(self.icon_path().exists())

    def test_uninstall_keeps_a_foreign_exec_and_a_symlink(self) -> None:
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        desktop = self.desktop_path()
        icon = self.icon_path()
        desktop.write_text(
            "Exec=/usr/bin/other\nX-Noter-Installer=true\n", encoding="utf-8"
        )
        foreign_icon = self.root / "foreign.svg"
        foreign_icon.write_text("<!-- noter-installer -->\n", encoding="utf-8")
        icon.unlink()
        icon.symlink_to(foreign_icon)

        removed = self.uninstall(self.bin_dir.parent)

        self.assertEqual(removed.returncode, 0, removed.stderr)
        self.assertEqual(
            desktop.read_text(encoding="utf-8"),
            "Exec=/usr/bin/other\nX-Noter-Installer=true\n",
        )
        self.assertTrue(icon.is_symlink())
        self.assertTrue(foreign_icon.is_file())

    def test_group_writable_data_home_is_refused_after_the_binary_is_installed(
        self,
    ) -> None:
        self.data_home.chmod(0o777)
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("writable by another user", result.stderr)
        self.assertTrue((self.bin_dir / "noter").is_file())
        self.assertFalse(self.desktop_path().exists())

    def test_relative_data_home_is_refused(self) -> None:
        self.environment["XDG_DATA_HOME"] = "relative/share"
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("XDG_DATA_HOME must be an absolute path.", result.stderr)
        self.assertTrue((self.bin_dir / "noter").is_file())

    def test_check_prints_desktop_paths_without_creating_them(self) -> None:
        installer = Path(__file__).with_name("install.sh")
        result = subprocess.run(
            [
                "sh",
                str(installer),
                "--binary",
                "--version",
                "0.1.0-beta.1",
                "--root",
                str(self.bin_dir.parent),
                "--check",
            ],
            cwd=self.root,
            env=self.environment,
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(str(self.desktop_path()), result.stdout)
        self.assertIn(str(self.icon_path()), result.stdout)
        self.assertFalse((self.data_home / "applications").exists())
        self.assertFalse((self.bin_dir / "noter").exists())


if __name__ == "__main__":
    unittest.main()
