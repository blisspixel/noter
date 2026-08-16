# Playtest Brief

**Updated:** 2026-08-16

**Build under test:** `main` at commit `23e8c23` or later. `noter --version`
still prints `noter 0.1.0-alpha.1`, unchanged from the previous round, because
the roadmap only allows the `0.1.0-alpha.2` label once its gates land on one
green commit. **Identify the build by commit, not by version string.**

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

The full command-line contract, including every exit status, is in
[INSTALLATION.md](INSTALLATION.md).

## Known open, please do not re-report

- **Bold does not survive Enter in Markdown Mode.** With the caret at the end of
  an emphasized run, a line break strands the closing delimiter. CommonMark
  refuses a closer that directly follows a break, so the formatting collapses and
  the raw markers appear. Confirmed: `**a\nb**` stays bold, `**a\n**` does not.
  The fix requires the reopened markers to be written only when the next
  character arrives, so no empty marker pair is ever left in the file. Scheduled,
  not shipped.
- **Idle cost is measured on Windows only.** The previous round ran on Linux
  X11. A repeat measurement there is genuinely useful and is recorded as open M2
  evidence in the [roadmap](ROADMAP.md).
- **No signed binary release exists.** Installation builds the locked source
  checkout. This is stated in the root README and remains M7 work.
- **Files above 8 MiB are refused** before the editor mirrors them. This is
  deliberate containment, not the final limit; the measured large-file path is
  M5 work.

## Where the untested surface is

The previous round could not drive the GUI, so the command-line face is now well
covered and the application itself is barely covered. The unknowns are here:

- the Mode control, and whether switching views leaves bytes byte-identical;
- typing, Undo and Redo, and whether long editing sessions stay bounded;
- Save, Save As, and the external-change prompts (Reload, Keep Editing, Save As,
  and overwrite second confirmation);
- Find, Replace, Replace All scope, and Go To Line;
- crash recovery: kill the process with unsaved changes and check the startup
  Restore and Discard offer, including its timing;
- the five themes as painted pixels, at display scales other than 100 percent;
- keyboard-only operation, and screen-reader semantics.

## Ground rules that make a report useful

These match the previous round's format, which worked well.

- Record the exact commit, the platform, and the exact commands.
- Prefer observations over inferences, and say which is which.
- Report successful flows as well as failures; the previous round's list of what
  worked was directly useful.
- Keep real documents out of the report. Use fixtures.
- Reading application source is not required. If a claim depends on source, say
  so.
