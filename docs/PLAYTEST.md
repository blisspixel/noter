# Playtest Brief

**Updated:** 2026-10-11

**Build under test:** beta.2, printing `noter 0.1.0-beta.2`, from an identified
source checkout or an available matching release artifact. Record the full commit
and binary checksum for every report. Beta.1's terminal preview has known safety
defects; do not use it for this round. The [readiness record](RELEASE_READINESS.md)
records source observations and remaining gates; check
[GitHub Releases](https://github.com/blisspixel/noter/releases) for available artifacts.

## Current round: Linux integration and beta hardening

Use a disposable Markdown fixture and keep real documents out of the report.
Build the exact reviewed checkout using the source installer, or verify a matching
published artifact. Record desktop,
compositor, toolkit session, display scale, and installed executable path.

- Open the fixture through the launcher and Open With. Confirm the selected
  file opens, including a path containing spaces and multibyte characters.
- On Hyprland, leave the window on an inactive workspace for at least two
  minutes, return, and type and save. Repeat with a dirty document and confirm
  crash recovery still persists while the workspace is hidden.
- Inspect File, Edit, View, and Markdown formatting menus at 100 and 150 percent
  scale. Long labels must remain separated from shortcuts.
- Inspect `plain **bold** and *italic*` in Green Screen and Amber Screen.
  Strong emphasis must be visibly distinct.
- Exercise failed Save, Undo, restart recovery, and terminal restoration using
  disposable files. Record observed results separately from unit-test coverage.

The earlier rounds below are historical evidence, not verification of the
current candidate. Interactive Linux, macOS, accessibility, and packaged
installation results remain pending unless recorded for the candidate.

This brief exists so a round is spent on unknowns instead of re-deriving what is
already known. It records what changed since the last round, what is deliberately
still open, and where the untested surface actually is.

## What changed since the 2026-08-05 round

Every finding from the previous installed-product report is addressed below.
Measurements are local Windows release-build numbers unless stated otherwise.

| Previous finding | Disposition |
| --- | --- |
| Idle window held about one CPU core | **Fixed.** The window title was resent as a viewport command every frame, and every viewport command requests a repaint, so the event loop never slept. The title is now sent only when it changes. Idle cost fell from 100.2 percent of one core to under 0.5 percent over 15 idle seconds; the working set fell from 87.2 MiB to 67.2 MiB. |
| A missing path, a directory, or a binary silently became `Untitled` | **Fixed for argument-shaped failures.** A missing path, a directory, and an unreadable file now print one line to standard error and exit 2, matching the existing invalid-option contract. Content-shaped failures, such as invalid UTF-8 or a document above the interactive size limit, still open the window and report in it, because those also arrive from the Open dialog and from desktop file associations. |
| `noter update` was indistinguishable from a blank editor | **Fixed.** The window is titled `Update status - Noter` while that status is showing, and returns to document titles once it is closed. `noter update --help` prints the usage block instead of exiting 2. |
| `--theme LIGHT` and `--view TEXT` were rejected | **Fixed.** Option values are accepted in any letter case. |
| The window title does not name the active view | **Declined.** The Mode control in the upper right names and switches the view. Adding it to the title trades a permanent cost for a first-run question. |

Two defects found while fixing the above are also closed: a dirty document now
books its own repaint from the recovery schedule, so a sleeping window cannot
silently lengthen the recovery-point objective; and `webbrowser` moved to 1.2.4
for RUSTSEC-2026-0257.

A later first-contact pass closed these additional defects:

| Finding | Disposition |
| --- | --- |
| Bold (and italic, strike, inline code) collapsed on Enter at the end of a run | **Fixed.** Enter closes the run on the current line. Markers reopen only when the next character arrives. Empty `**\n**` pairs are never written. Mid-run Enter that stays a legal CommonMark span is unchanged. |
| First untitled frame did not focus the editor | **Fixed.** Launch without a file puts the caret in the document so the first keystroke is the first character. |
| Untitled status said `Saved` | **Fixed.** Never-saved documents report `Unsaved`. `Saved` is only after a successful commit to a path. |
| Opening a file discarded private recovery records | **Fixed.** Explicit path launches skip the restore prompt without deleting records. A later untitled launch can still offer restore. Discard remains an explicit choice. |
| Keep Editing re-prompted every inspect after local typing | **Fixed.** The prompt returns only when the disk classification changes. |
| Recovery offer had no Cancel; Escape did nothing | **Fixed.** Later or Escape hides the offer and keeps the private copy. |
| Enter in a list or quote dropped the marker | **Fixed.** `- ` and `> ` continue; an empty item exits. |
| Mode and Wrap were pointer-only | **Fixed.** Ctrl/Cmd+Shift+M switches Mode; Alt+Z toggles wrap in Text Mode. |
| Replace-field Enter did nothing | **Fixed.** Enter replaces a selected match, otherwise Find Next. |
| A second window could Discard another window's live recovery | **Fixed.** A living window holds an exclusive lease on its recovery instance. |

The full command-line contract, including every exit status, is in
[INSTALLATION.md](INSTALLATION.md).

## Known open, please do not re-report

- **Idle cost is measured on Windows only.** The previous round ran on Linux
  X11. A repeat measurement there is genuinely useful and is recorded as open M2
  evidence in the [roadmap](ROADMAP.md).
- **Prerelease artifacts are unsigned.** Platform archives, checksums, SBOMs,
  provenance, and a Windows MSI are planned release outputs. Check the actual
  release inventory; Windows and macOS platform signing remains M7 work.
  Keep backups and verify downloaded files before use.
- **Files above 8 MiB are refused** before the editor mirrors them. This is
  deliberate containment, not the final limit; the measured large-file path is
  M5 work.

## Remaining interactive coverage

The 2026-08-22 Windows recovery exercise drove the live editor, force-killed the
process after a private record appeared, restored the exact observed editor
value after restart, and exited through explicit discard with no record or lease
left. The later [2026-10-11 Windows observations](evidence/beta2-windows-2026-10-11.md)
cover edit/save, Undo/Redo, failed-save and conflict retention, crash recovery,
menus, and CRT themes at the observed host scale on `ed67c26`. Neither record
establishes the complete current platform matrix. Remaining coverage includes:

- complete mode-switch, selection, and clipboard byte fidelity across platforms;
- long editing sessions and bounded history across platforms;
- the full Save, Save As, and external-change prompt matrix, including overwrite
  second confirmation;
- Find, Replace, Replace All scope, and Go To Line;
- crash recovery on interactive macOS, Linux X11, and Linux Wayland sessions;
- the five themes as painted pixels, at display scales other than 100 percent;
- keyboard-only operation, and screen-reader semantics.

## Ground rules that make a report useful

These match the previous round's format, which worked well.

- Record the exact commit, the platform, and the exact commands.
- Prefer observations over inferences, and say which is which.
- Report successful flows as well as failures; the previous round's list of what
  worked was directly useful.
- Keep real documents out of the report. Use ordinary English visible fixtures.
  Use multibyte, wide, combining, and control characters in the targeted cases
  that verify those boundaries; retain their automated safety coverage.
- Reading application source is not required. If a claim depends on source, say
  so.
