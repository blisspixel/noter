"""Tests for the pseudo-terminal check's pure helpers."""

from __future__ import annotations

import os
import unittest
from unittest import mock

import check_tui_pty


class TerminalEnvironmentTests(unittest.TestCase):
    def test_removes_every_display_variable_and_isolates_state(self) -> None:
        inherited = {
            "DISPLAY": ":0",
            "WAYLAND_DISPLAY": "wayland-0",
            "WAYLAND_SOCKET": "3",
            "HOME": "/home/tester",
        }
        with mock.patch.dict(os.environ, inherited, clear=True):
            environment = check_tui_pty.terminal_environment("/tmp/state")

        self.assertNotIn("DISPLAY", environment)
        self.assertNotIn("WAYLAND_DISPLAY", environment)
        self.assertNotIn("WAYLAND_SOCKET", environment)
        self.assertEqual(environment["XDG_DATA_HOME"], "/tmp/state")
        self.assertEqual(environment["TERM"], "xterm-256color")
        self.assertEqual(environment["HOME"], "/tmp/state")


@unittest.skipIf(check_tui_pty.pty is None, "pseudo-terminals are unavailable")
class ReadUntilTests(unittest.TestCase):
    def test_stops_at_the_needle_and_gives_up_at_the_deadline(self) -> None:
        reader, writer = os.pipe()
        try:
            os.write(writer, b"before needle after")
            self.assertIn(b"needle", check_tui_pty.read_until(reader, b"needle", 5.0))
            self.assertEqual(check_tui_pty.read_until(reader, b"absent", 0.1), b"")
        finally:
            os.close(reader)
            os.close(writer)

    def test_expect_raises_with_the_message(self) -> None:
        with self.assertRaisesRegex(AssertionError, "broken"):
            check_tui_pty.expect(False, "broken")
        check_tui_pty.expect(True, "unused")


if __name__ == "__main__":
    unittest.main()
