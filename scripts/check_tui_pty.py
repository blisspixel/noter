#!/usr/bin/env python3
"""Drive a built Noter binary's terminal interface through a pseudo-terminal.

CI test processes run without a controlling terminal, so raw mode, bracketed
paste, terminal restoration, and hangup handling cannot be observed there.
This check runs them for real on macOS or Linux and fails on the first broken
expectation. Pass the binary to test, for example `target/debug/noter`.
"""

from __future__ import annotations

import argparse
import os
import select
import subprocess
import sys
import tempfile
import time
from pathlib import Path

try:
    import pty
    import termios
except ImportError:  # pragma: no cover - Windows has neither module.
    pty = None
    termios = None


def terminal_environment(state_directory: str) -> dict[str, str]:
    """Return an environment with no display and an isolated state root."""

    # Linux keeps state under XDG_DATA_HOME and macOS under HOME, so both
    # point at the temporary root and the user's real recovery store is never
    # touched.
    environment = dict(
        os.environ,
        TERM="xterm-256color",
        XDG_DATA_HOME=state_directory,
        HOME=state_directory,
    )
    for name in ("DISPLAY", "WAYLAND_DISPLAY", "WAYLAND_SOCKET"):
        environment.pop(name, None)
    return environment


def read_until(master: int, needle: bytes, seconds: float = 15.0) -> bytes:
    """Collect terminal output until `needle` appears or `seconds` pass."""

    output = b""
    deadline = time.monotonic() + seconds
    while needle not in output and time.monotonic() < deadline:
        ready, _, _ = select.select([master], [], [], 0.05)
        if ready:
            try:
                output += os.read(master, 65536)
            except OSError:
                break
    return output


# Every frame ends by placing the cursor and showing it; waiting for that
# marker after input means the input has been handled and drawn.
FRAME_END = b"\x1b[?25h"


def next_frame(master: int) -> bytes:
    """Wait for the next complete frame."""

    return read_until(master, FRAME_END)


def expect(condition: bool, message: str) -> None:
    """Fail the check with `message` unless `condition` holds."""

    if not condition:
        raise AssertionError(message)


SETTING_NAMES = ("iflag", "oflag", "cflag", "lflag", "ispeed", "ospeed", "cc")


def settings_difference(before: list, after: list) -> list[str]:
    """Name each terminal setting that differs, ignoring kernel state bits.

    PENDIN and FLUSHO in the local flags report the line discipline's own
    progress rather than a mode a program sets, so a restored terminal may
    differ from its saved copy in them alone.
    """

    state_bits = getattr(termios, "PENDIN", 0) | getattr(termios, "FLUSHO", 0)
    changed = []
    for name, old, new in zip(SETTING_NAMES, before, after):
        if name == "lflag":
            old, new = old & ~state_bits, new & ~state_bits
        if old != new:
            changed.append(f"{name} {old!r} -> {new!r}")
    return changed


def check_editing_session(binary: str, state_directory: str) -> None:
    """Type split UTF-8 and a paste, save, exit, and check the terminal."""

    workspace = tempfile.mkdtemp()
    document = Path(workspace, "note.txt")
    document.write_text("héllo\n", encoding="utf-8")
    master, slave = pty.openpty()
    settings_before = termios.tcgetattr(slave)
    process = subprocess.Popen(
        [binary, "--tui", str(document)],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=terminal_environment(state_directory),
        close_fds=True,
    )
    screen = next_frame(master)
    os.write(master, b"\x1b[F")
    word = " wörld".encode()
    # Two writes with a pause, so the character reaches Noter split across
    # two reads.
    os.write(master, word[:3])
    time.sleep(0.05)
    os.write(master, word[3:])
    os.write(master, b"\x1b[200~\npasted\x0fline\x1b[201~")
    screen += read_until(master, b"line")
    os.write(master, b"\x13")
    screen += read_until(master, b"Wrote")
    os.write(master, b"\x18")
    screen += read_until(master, b"\x1b[?1049l")
    status = process.wait(timeout=10)

    expect(status == 0, f"the editing session exited with {status}")
    expect(
        document.read_text(encoding="utf-8") == "héllo wörld\npasted\x0fline\n",
        "the saved text differs from what was typed and pasted",
    )
    changed = settings_difference(settings_before, termios.tcgetattr(slave))
    expect(not changed, f"terminal settings were not restored: {changed}")
    expect(b"\x1b[?1049l" in screen, "the alternate screen was not left")
    expect(b"\x1b]" not in screen, "an operating-system command reached the terminal")
    os.close(master)
    os.close(slave)


def check_hangup_recovery(binary: str, state_directory: str) -> None:
    """Close the terminal on unsaved text, then restore it on the next launch."""

    unsaved = "never saved: 世界"
    master, slave = pty.openpty()
    process = subprocess.Popen(
        [binary, "--tui"],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=terminal_environment(state_directory),
        close_fds=True,
        start_new_session=True,
    )
    os.close(slave)
    next_frame(master)
    os.write(master, unsaved.encode())
    read_until(master, unsaved[-2:].encode())
    os.close(master)
    process.wait(timeout=10)

    master, slave = pty.openpty()
    process = subprocess.Popen(
        [binary, "--tui"],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=terminal_environment(state_directory),
        close_fds=True,
    )
    screen = read_until(master, b"was kept after Noter closed")
    expect(
        b"was kept after Noter closed" in screen, "no recovery offer after the hangup"
    )
    os.write(master, b"r")
    screen = read_until(master, unsaved.encode())
    expect(unsaved.encode() in screen, "the restored text is not on screen")
    os.write(master, b"\x18n")
    read_until(master, b"\x1b[?1049l")
    expect(process.wait(timeout=10) == 0, "the restored session did not exit cleanly")
    os.close(master)
    os.close(slave)


def main(arguments: list[str] | None = None) -> int:
    """Run both terminal checks against the named binary."""

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", help="path to a built noter executable")
    options = parser.parse_args(arguments)
    if pty is None:
        print("pseudo-terminals are not available on this platform", file=sys.stderr)
        return 2
    with tempfile.TemporaryDirectory() as state_directory:
        check_editing_session(options.binary, state_directory)
        check_hangup_recovery(options.binary, state_directory)
    print("terminal interface checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
