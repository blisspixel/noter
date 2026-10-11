# Beta.2 Windows native observations

Date: 2026-10-11. Runtime source: `ed67c26015fa8bfb60549341fc97a9bce8d5945b`.
Host: Windows 11 Pro, build 26200. Executable: release build with the
`screenshot-qa` feature, version `0.1.0-beta.2`, SHA-256
`a0bbb382f23d0935349a172125422cbed5d476b7ae3a1dd74a5d281680300960`.
Recovery state and disposable documents were isolated under `.agent/` through
`NOTER_SCREENSHOT_QA_STATE_DIRECTORY`. This is a native source-build observation,
not a downloaded package, clean-machine, or MSI observation.

| Action | Observed result |
| --- | --- |
| Open UTF-8 Markdown with CRLF, edit, Undo, Redo, Save | Exact expected Unicode source and CRLF bytes persisted; clean status after Save |
| Change destination metadata externally, then Save | Conflict surfaced; dirty text and original disk bytes retained; Keep Editing worked |
| Force-stop only the isolated dirty fixture process after a recovery snapshot exists; restart and Restore | Full source restored, including Markdown delimiters and multibyte characters; Text Mode confirmed complete source |
| Save restored untitled source to a new fixture path | Exact UTF-8 and CRLF bytes persisted; recovery snapshot retired after Save; clean exit released live guards |
| Open a destination already marked read-only, edit, Save | Explicit Not Committed diagnostic: ApplyMetadata failed; original disk bytes unchanged; dirty text and recovery snapshot retained |
| Undo refused-save fixture to its baseline | Clean status and exact original text restored |
| File menu and Green Screen/Amber Screen Markdown | Shortcut separation and distinct strong emphasis visually confirmed; mode/theme changes retained saved source |
| Ten idle TCP/UDP endpoint samples for the isolated fixture PID | No endpoints observed; this bounded sample does not establish every runtime path |

The five tracked native screenshots were regenerated with the beta.2 manifests
and lockfile. Their bytes match the previously reviewed captures. The screenshot
validator binds their input digest to this source state.

Limits: host display scale was not independently established; document zoom is
not display scale. The complete 100/150 percent desktop-scale matrix, macOS/X11
interaction, clean-machine packaging, and Omarchy/Hyprland workspace retest remain
pending at their assigned checkpoints. No native Hyprland resolution is claimed.
Visible follow-up fixtures use English text; automated fidelity tests retain
multibyte, wide, combining, and control-character cases as required by the merge
contract.
