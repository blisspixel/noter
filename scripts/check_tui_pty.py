#!/usr/bin/env python3
"""Drive a built Noter binary's terminal interface through a pseudo-terminal.

CI test processes run without a controlling terminal, so raw mode, bracketed
paste, terminal restoration, and hangup handling cannot be observed there.
This check runs them for real on macOS or Linux and fails on the first broken
expectation. Pass the binary to test, for example `target/debug/noter`.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import select
import signal
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path

try:
    import fcntl
    import pty
    import termios
except ImportError:  # pragma: no cover - Windows has none of these modules.
    fcntl = None
    pty = None
    termios = None


def set_window_size(fd: int, columns: int, rows: int) -> None:
    """Set the pseudo-terminal size before the child starts."""

    if fcntl is None or termios is None:
        raise RuntimeError("terminal size is unavailable")
    # winsize is row, column, x pixels, y pixels.
    size = struct.pack("HHHH", rows, columns, 0, 0)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, size)


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


def read_until_any(
    master: int, needles: tuple[bytes, ...], seconds: float = 15.0
) -> bytes:
    """Collect terminal output until one of `needles` appears or `seconds` pass."""

    output = b""
    deadline = time.monotonic() + seconds
    while (
        not any(needle in output for needle in needles) and time.monotonic() < deadline
    ):
        ready, _, _ = select.select([master], [], [], 0.05)
        if ready:
            try:
                output += os.read(master, 65536)
            except OSError:
                break
    return output


def read_until(master: int, needle: bytes, seconds: float = 15.0) -> bytes:
    """Collect terminal output until `needle` appears or `seconds` pass."""

    return read_until_any(master, (needle,), seconds)


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


def check_save_warning_review(master: int, document: Path, prior: bytes) -> bytes:
    """Review Linux's actual retained sibling without editing or removing bytes."""

    siblings = list(document.parent.glob(".noter-save-*.tmp"))
    expect(len(siblings) == 1, "the save did not retain exactly one prior sibling")
    sibling = siblings[0]
    current = document.read_bytes()
    os.write(master, b"\x12")
    screen = read_until(master, b"Closing does not remove files")
    expect(b"Latest save warnings" in screen, "Ctrl+R did not open save warnings")
    expect(
        b"Closing does not remove files" in screen,
        "the complete warning pane was not drawn",
    )
    # Diagnostic wrapping and ANSI styling may split the synthetic basename.
    plain = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", screen)
    compact = b"".join(plain.split())
    expect(sibling.name.encode() in compact, "the actual retained basename is absent")
    os.write(master, b"X\r")
    screen += next_frame(master)
    expect(sibling.read_bytes() == prior, "warning review changed the retained bytes")
    expect(document.read_bytes() == current, "warning review changed saved bytes")
    return screen


def plain_terminal(screen: bytes) -> bytes:
    """Drop ANSI control sequences so a later assertion sees the drawn text."""

    return re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", screen)


def read_ready(master: int, seconds: float) -> bytes:
    """Read output that arrives within `seconds`.

    A stopped reader fills the terminal buffer. macOS delivers only a few
    kilobytes before the editor blocks in its next write, so every wait has
    to keep draining or a following key is never read. `seconds` of zero
    drains what is already buffered and does not wait.
    """

    output = b""
    deadline = time.monotonic() + seconds
    while True:
        remaining = deadline - time.monotonic()
        if seconds != 0 and remaining <= 0:
            break
        ready, _, _ = select.select(
            [master], [], [], 0 if seconds == 0 else min(0.05, remaining)
        )
        if not ready:
            break
        try:
            chunk = os.read(master, 65536)
        except OSError:
            break
        if not chunk:
            break
        output += chunk
    return output


def wait_for_drawn_text(master: int, marker: bytes, seconds: float = 5.0) -> bytes:
    """Drain until `marker` is on screen and the frame that drew it finished."""

    screen = b""
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if marker in plain_terminal(screen):
            marker_at = screen.rfind(marker)
            if marker_at != -1 and FRAME_END in screen[marker_at:]:
                screen += read_ready(master, 0)
                return screen
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        screen += read_ready(master, min(0.05, remaining))
    plain = plain_terminal(screen)
    expect(False, f"{marker!r} was not drawn; screen ended {plain[-400:]!r}")


def wait_for_document(master: int, document: Path, expected: str) -> None:
    """Drain terminal output until `document` equals `expected`."""

    screen = b""
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if document.read_text(encoding="utf-8") == expected:
            return
        screen += read_ready(master, 0.05)
    plain = plain_terminal(screen)
    expect(
        False,
        f"document stayed {document.read_text(encoding='utf-8')!r}; "
        f"screen ended {plain[-400:]!r}",
    )


def check_split_escape(binary: str, state_directory: str) -> None:
    """A sequence split after ESC moves the caret, and a bare ESC waits."""

    workspace = tempfile.mkdtemp()
    document = Path(workspace, "note.txt")
    document.write_text("alpha\nbeta\n", encoding="utf-8")
    master, slave = pty.openpty()
    set_window_size(slave, 80, 24)
    process = subprocess.Popen(
        [binary, "--tui", str(document)],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=terminal_environment(state_directory),
        close_fds=True,
    )
    try:
        next_frame(master)
        # One write: this Down is not the split under test.
        os.write(master, b"\x1b[B")
        time.sleep(0.05)
        # Under the 100 ms deadline, so the two writes must join into Up.
        os.write(master, b"\x1b")
        time.sleep(0.04)
        os.write(master, b"[A")
        time.sleep(0.05)
        os.write(master, b"Q")
        wait_for_drawn_text(master, b"Qalpha")
        os.write(master, b"\x13")
        wait_for_document(master, document, "Qalpha\nbeta\n")

        os.write(master, b"\x07")
        opened = read_until(master, b"closes this help")
        expect(b"closes this help" in opened, "help did not open")
        # A following byte before the deadline would be Alt+that key, not a
        # separate press. Hold the ESC alone and require no redraw yet.
        os.write(master, b"\x1b")
        early = read_until(master, FRAME_END, 0.05)
        expect(FRAME_END not in early, "bare Escape closed help before the deadline")
        time.sleep(0.12)
        closed = read_until(master, FRAME_END, 1.0)
        expect(FRAME_END in closed, "bare Escape did not close help after the deadline")
        os.write(master, b"Z")
        wait_for_drawn_text(master, b"QZalpha")
        os.write(master, b"\x13")
        wait_for_document(master, document, "QZalpha\nbeta\n")
        os.write(master, b"\x18")
        read_until(master, b"\x1b[?1049l")
        expect(process.wait(timeout=10) == 0, "the split-escape session did not exit")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=10)
        os.close(master)
        os.close(slave)


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
    # A retained sibling says Wrote. A clean directory sync says synced to disk.
    os.write(master, b"\x13")
    screen += read_until_any(master, (b"Wrote", b"synced to disk"))
    expect(
        b"Wrote" in screen or b"synced to disk" in screen,
        "the save did not report a committed write",
    )
    if sys.platform == "linux":
        screen += check_save_warning_review(master, document, "héllo\n".encode())
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

    unsaved = "never saved: ordinary English draft"
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
    read_until(master, unsaved.encode())
    os.close(master)
    process.wait(timeout=10)

    master, slave = pty.openpty()
    # The offer keeps restore, discard, and later on an 80-column bar. Pin
    # that width so a narrow inherited size cannot hide the label.
    set_window_size(slave, 80, 24)
    process = subprocess.Popen(
        [binary, "--tui"],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=terminal_environment(state_directory),
        close_fds=True,
    )
    offer = b"(r)estore, (d)iscard, (l)ater. Unsaved text from Untitled"
    screen = read_until(master, offer)
    expect(offer in screen, "no recovery offer after the hangup")
    os.write(master, b"r")
    screen = read_until(master, unsaved.encode())
    expect(unsaved.encode() in screen, "the restored text is not on screen")
    os.write(master, b"\x18n")
    read_until(master, b"\x1b[?1049l")
    expect(process.wait(timeout=10) == 0, "the restored session did not exit cleanly")
    os.close(master)
    os.close(slave)


def check_sigterm_restores_the_terminal(binary: str, state_directory: str) -> None:
    """SIGTERM leaves the alternate screen and restores cooked settings."""

    master, slave = pty.openpty()
    settings_before = termios.tcgetattr(slave)
    process = subprocess.Popen(
        [binary, "--tui"],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=terminal_environment(state_directory),
        close_fds=True,
    )
    screen = b""
    status = None
    settings_after = None
    try:
        next_frame(master)
        process.send_signal(signal.SIGTERM)
        screen = read_until(master, b"\x1b[?1049l", 5.0)
        status = process.wait(timeout=10)
        settings_after = termios.tcgetattr(slave)
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=10)
        os.close(master)
        os.close(slave)
    expect(
        status == -signal.SIGTERM,
        f"SIGTERM did not end the process by signal: {status}",
    )
    expect(b"\x1b[?1049l" in screen, "SIGTERM did not leave the alternate screen")
    expect(b"\x1b[?1000l" in screen, "SIGTERM did not disable mouse reporting")
    expect(b"\x1b[?1002l" in screen, "SIGTERM did not disable button-event reporting")
    expect(b"\x1b[?1006l" in screen, "SIGTERM did not disable SGR mouse reporting")
    expect(b"\x1b[?2004l" in screen, "SIGTERM did not disable bracketed paste")
    changed = settings_difference(settings_before, settings_after)
    expect(not changed, f"SIGTERM left terminal settings changed: {changed}")


def check_volatile_recovery_refusal(binary: str) -> None:
    """Show real tmpfs refusal while retaining records and permitting a save."""

    with (
        tempfile.TemporaryDirectory(prefix="noter-volatile-", dir="/dev/shm") as state,
        tempfile.TemporaryDirectory(prefix="noter-persistent-") as workspace,
    ):
        mount = json.loads(
            subprocess.check_output(
                ["findmnt", "--json", "--target", state, "--output", "FSTYPE"],
                text=True,
            )
        )
        expect(
            mount["filesystems"][0]["fstype"] == "tmpfs",
            "the volatile-state fixture requires a real tmpfs mount",
        )
        control = Path(state, "control.txt")
        control.write_bytes(b"accessible volatile control")
        expect(
            control.read_bytes() == b"accessible volatile control", "control I/O failed"
        )
        recovery = Path(state, "noter", "recovery")
        previous = recovery / "records" / "previous.rec"
        previous.parent.mkdir(parents=True)
        previous.write_bytes(b"preserved recovery sentinel")
        facts = previous.stat()
        identity = (facts.st_dev, facts.st_ino, facts.st_size)
        document = Path(workspace, "note.txt")
        document.write_bytes(b"ordinary English note\n")
        master, slave = pty.openpty()
        # Pin 80 columns, the width where a long path can crowd the reason.
        set_window_size(slave, 80, 24)
        settings_before = termios.tcgetattr(slave)
        process = None
        try:
            process = subprocess.Popen(
                [binary, "--tui", str(document)],
                stdin=slave,
                stdout=slave,
                stderr=slave,
                env=terminal_environment(state),
                close_fds=True,
            )
            screen = next_frame(master)
            expect(
                b"Recovery is off for this session:" in screen,
                "the volatile-state refusal is not visible in the terminal",
            )
            os.write(master, b"\x12")
            screen = read_until(master, b"Choose a private")
            plain = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", screen)
            compact = b" ".join(plain.split())
            reason_at = compact.find(b"unsupported file system")
            step_at = compact.find(b"Choose a private directory")
            expect(
                reason_at != -1 and step_at != -1 and reason_at < step_at,
                "the review does not show the file-system reason before the next step: "
                + compact[:500].decode("utf-8", "replace"),
            )
            expect(
                b"Saves still work" in compact,
                "the review drops the save promise",
            )
            os.write(master, b"\x12")
            next_frame(master)
            os.write(master, b"\x1b[F updated")
            screen = read_until(master, b"updated")
            expect(b"updated" in screen, "editing failed with recovery unavailable")
            os.write(master, b"\x13")
            screen = read_until_any(master, (b"Wrote", b"synced to disk"))
            expect(
                b"Wrote" in screen or b"synced to disk" in screen,
                "saving failed with recovery unavailable",
            )
            os.write(master, b"\x18")
            screen = read_until(master, b"\x1b[?1049l")
            expect(process.wait(timeout=10) == 0, "the refused-root session failed")
            expect(b"\x1b[?1049l" in screen, "the alternate screen was not left")
            expect(
                not settings_difference(settings_before, termios.tcgetattr(slave)),
                "the refused-root session did not restore terminal settings",
            )
        finally:
            if process is not None and process.poll() is None:
                process.kill()
                process.wait(timeout=10)
            os.close(master)
            os.close(slave)
        expect(
            document.read_bytes() == b"ordinary English note updated\n",
            "the saved document bytes differ with recovery unavailable",
        )
        facts = previous.stat()
        expect(
            (facts.st_dev, facts.st_ino, facts.st_size) == identity,
            "record identity changed",
        )
        expect(
            previous.read_bytes() == b"preserved recovery sentinel",
            "record bytes changed",
        )
        expect(
            control.read_bytes() == b"accessible volatile control",
            "control bytes changed",
        )
        expect(
            sorted(
                path.relative_to(recovery).as_posix() for path in recovery.rglob("*")
            )
            == ["records", "records/previous.rec"],
            "refused recovery created, removed or renamed an entry",
        )


def main(arguments: list[str] | None = None) -> int:
    """Run the terminal checks against the named binary."""

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", help="path to a built noter executable")
    options = parser.parse_args(arguments)
    if pty is None:
        print("pseudo-terminals are not available on this platform", file=sys.stderr)
        return 2
    with tempfile.TemporaryDirectory() as state_directory:
        check_editing_session(options.binary, state_directory)
        check_split_escape(options.binary, state_directory)
        check_hangup_recovery(options.binary, state_directory)
    with tempfile.TemporaryDirectory() as state_directory:
        check_sigterm_restores_the_terminal(options.binary, state_directory)
    if sys.platform == "linux":
        check_volatile_recovery_refusal(options.binary)
    print("terminal interface checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
