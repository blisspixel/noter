# Noter Manual Test Matrix

This document is the living checklist used for every phase quality gate.

Copy this file into a dated run (e.g. `tests/manual-2026-06-12-windows.md`) and mark items as you execute.

## Environment

- OS + version + build:
- Display scaling:
- Primary input: keyboard / mouse / both
- Noter binary: debug or release? commit hash:
- Date / tester:

## 1. Basic Workflow (must feel like classic Notepad)

- [ ] Launch with no arguments → clean untitled document
- [ ] Type text, cursor movement with arrows/Home/End/PageUp/PageDown
- [ ] Ctrl+N → new document (previous one prompts if dirty)
- [ ] Ctrl+O → native open dialog, open a real file
- [ ] Edit, Ctrl+S → saves (verify with external editor or `cat`)
- [ ] Ctrl+Shift+S → Save As to a new location
- [ ] Close window with unsaved changes → Save / Discard / Cancel all work
- [ ] Recent files list appears after opening 3+ files; clicking opens them
- [ ] Open a file that no longer exists from recent list → graceful removal or error

## 2. Line Endings & Fidelity (critical)

- [ ] Open a CRLF-heavy Windows log file → status shows CRLF, save produces identical line endings (byte diff or hex editor)
- [ ] Open an LF file (typical Linux/macOS) → preserves LF
- [ ] Create new file on this OS → correct default line ending (CRLF on Windows, LF elsewhere)
- [ ] Open file with UTF-8 BOM → BOM is preserved on save
- [ ] Mixed line endings file → we pick one style on load and are consistent (document the chosen behavior)

## 3. Theme

- [ ] Launch with system in Light → app starts in Light (or "System" correctly follows)
- [ ] Switch system to Dark while app is running (if "System" selected) → app follows live (Windows) or on next launch (other platforms)
- [ ] View menu → explicit Light / Dark / System choices work and persist across restart
- [ ] Both themes have readable text, visible selection, and decent contrast (no light-gray-on-white disasters)

## 4. Reliability Features

- [ ] While typing heavily, force-kill the process (Task Manager / `kill -9`)
- [ ] Restart Noter → recovery offer appears with the document content (most recent autosave)
- [ ] Save the recovered document → autosave file is cleaned up
- [ ] Edit a file in Noter, then edit the same file in another editor and save → on focus or timer, Noter detects change and offers Reload / Keep Mine
- [ ] Perform Save on a file in a directory you have no write permission (or simulate by making the target read-only) → clear error, original file untouched

## 5. Editing & Find

- [ ] Undo (Ctrl+Z) and Redo (Ctrl+Y / Ctrl+Shift+Z) feel natural
- [ ] Type 50 characters quickly → they form **one** undo step (coalescing)
- [ ] Ctrl+F opens find bar, type, F3 / Shift+F3 cycle matches
- [ ] Ctrl+H opens replace, "Replace All" works on a selection or whole doc
- [ ] Go To Line (Ctrl+G) on a 10,000+ line file jumps instantly and accurately
- [ ] Word wrap on/off, status bar updates, text reflows correctly

## 6. Large Files & Performance (spot check)

- [ ] Open a 10–20 MiB real log or data file → first screen appears in < 3s, scrolling is usable
- [ ] Scroll to the end and back while measuring subjective frame time (no multi-second freezes)
- [ ] Cursor movement and typing near the end of the large file feels responsive

## 7. Markdown Preview (Phase 3+)

- [ ] Open a real README.md → View > Markdown Preview
- [ ] Preview renders headings, lists, code blocks, emphasis at usable quality
- [ ] Edit in the main pane → preview updates (debounced, not laggy)
- [ ] Toggle preview off → editor takes full width again, no state corruption
- [ ] Very large .md file → preview either limits itself gracefully or remains usable

## 8. Cross-Platform Specific (run on each OS)

- [ ] Shortcuts: Ctrl on Windows/Linux, Cmd on macOS all work for the primary actions
- [ ] Native file dialog looks and behaves correctly for the OS
- [ ] High-DPI / Retina / 150% scaling: text is crisp, UI elements are sized correctly
- [ ] Window restore after close (position, size, maximized) works and doesn't put window off-screen
- [ ] Multiple instances can be launched and used independently

## 9. Crash & Recovery Simulation (at least once per phase gate)

Document the method used (actual kill, debugger, power loss on test hardware, etc.):

- 10+ simulated crashes during heavy editing of an important document
- Recovery always offered
- Recovered content is within the last 30–40 seconds of work
- After successful save of recovered content, no stale autosaves remain

## 10. Polish & Edge Cases

- [ ] Empty file (0 bytes) → open, edit, save → still valid
- [ ] File containing only newlines / only whitespace
- [ ] File with 10,000 character single line (no word wrap)
- [ ] Unicode (emoji, CJK, RTL if we claim support, combining chars)
- [ ] Very long path + filename on Save As
- [ ] Status bar never shows obviously wrong line/col numbers

## Sign-off

- Tester name + date:
- Platform(s) covered:
- Major issues found (list or "none"):
- Recommendation: Pass / Pass with notes / Block

This matrix, when filled, becomes part of the release record for the phase.
