# Noter Manual Test Matrix

Copy this template into a dated release-evidence file. Record Pass, Fail, Blocked,
or Not Applicable for every item. Not Applicable requires a reason.

Automated tests remain mandatory. This matrix covers behavior that compilation,
unit tests, and coverage cannot prove.

## Evidence header

- Noter commit:
- Artifact checksum:
- Build profile and packaging:
- Tester:
- Date:
- Operating system, version, and build:
- Desktop session, if Linux:
- CPU and memory:
- Display resolution, refresh rate, and scaling:
- Keyboard layout and input method:
- Screen reader and version:
- Filesystem and storage:
- Relevant automated evidence links:

## 1. Launch and basic lifecycle

- [ ] LIF-01 Launch without a path shows one clean Untitled document.
- [ ] LIF-02 Typing marks the title and status as modified.
- [ ] LIF-03 New on a dirty document exercises Save, Discard, and Cancel.
- [ ] LIF-04 Open on a dirty document exercises Save, Discard, and Cancel.
- [ ] LIF-05 Window Close on a dirty document exercises Save, Discard, and Cancel.
- [ ] LIF-06 Quit on a dirty document exercises Save, Discard, and Cancel.
- [ ] LIF-07 A failed Save keeps the document open, dirty, and editable.
- [ ] LIF-08 Repeated Close while a decision is open creates no duplicate dialog
  and loses no state.
- [ ] LIF-09 Multiple instances edit independent documents without sharing dirty
  or recovery state.

## 2. Open, Save, and byte fidelity

- [ ] IO-01 Open uses the system file dialog and cancellation changes no state.
- [ ] IO-02 Save As cancellation preserves the prior path and dirty state.
- [ ] IO-03 Save As failure preserves the prior path, original file, and content.
- [ ] IO-04 Empty, whitespace-only, and newline-only files round-trip.
- [ ] IO-05 UTF-8 with and without BOM round-trips byte for byte when untouched.
- [ ] IO-06 Invalid UTF-8 is rejected without replacement characters or source
  modification.
- [ ] IO-07 Uniform LF, CRLF, and CR files round-trip byte for byte.
- [ ] IO-08 A mixed-EOL file round-trips untouched.
- [ ] IO-09 Editing one mixed-EOL line does not normalize unrelated lines.
- [ ] IO-10 Explicit EOL conversion changes every ending, updates status, and is
  one undo step.
- [ ] IO-11 Read-only, permission-denied, disk-full, and locked-target failures
  leave the original complete.
- [ ] IO-12 Existing destination metadata follows the ratified platform policy.
- [ ] IO-13 Open and Save As refuse final symlinks and Windows reparse points
  without changing the link or its target.
- [ ] IO-14 Cloud, network, removable, and weaker-filesystem limitations are
  explicit, and the reported durability never exceeds observed capability.
- [ ] IO-15 A newly created Unix document is mode 0600; a new Windows document
  receives the parent directory's expected inherited DACL.
- [ ] IO-16 A multiply hard-linked destination is refused until explicit
  confirmation; after confirmation only the selected name receives new bytes.
- [ ] IO-17 Windows existing-file replacement preserves DACLs, named streams,
  compression, encryption, and the documented creation and identifier policy.
- [ ] IO-18 Windows `ReplaceFileW` success and errors 1175, 1176, and 1177 leave
  only states the reconciliation model classifies correctly; uncertain states
  retain private artifacts and never invite blind retry.
- [ ] IO-19 Linux replacement preserves exact mode, attainable ownership, ACLs,
  visible extended attributes, SELinux context, and capabilities, or refuses
  before commit with the original complete.
- [ ] IO-20 macOS replacement preserves mode, attainable ownership, ACLs,
  extended attributes, resource forks, and quarantine data; modification time
  advances according to policy, and BSD file-flag behavior is recorded.
- [ ] IO-21 A cleanup failure after commit is distinguishable from a durability
  warning, and neither is reported as a failed or uncommitted save.

Record byte-comparison commands, fixture checksums, and observed save outcomes:

## 3. Recovery

- [ ] REC-01 Force-kill after typing and before the idle debounce.
- [ ] REC-02 Force-kill after the idle debounce.
- [ ] REC-03 Force-kill during recovery replacement.
- [ ] REC-04 Restart offers the newest valid acknowledged revision.
- [ ] REC-05 Recovered content opens dirty and does not write the original.
- [ ] REC-06 Save and explicit Discard remove only the owned recovery record.
- [ ] REC-07 Cancel preserves the recovery record.
- [ ] REC-08 Corrupt, truncated, wrong-version, and checksum-invalid records are
  quarantined and explained.
- [ ] REC-09 Two live instances do not claim each other's records.
- [ ] REC-10 Recovery persistence failure is visible and dirty Close still
  prompts.

Record kill method, recovery revisions, timing, and recovery directory:

## 4. External changes

- [ ] CON-01 Modify the file externally while Noter is clean.
- [ ] CON-02 Modify the file externally while Noter is dirty.
- [ ] CON-03 Replace, delete, and recreate the path externally.
- [ ] CON-04 Reload is guarded by the dirty decision flow.
- [ ] CON-05 Keep Editing does not silently authorize a later overwrite.
- [ ] CON-06 Save As preserves both versions.
- [ ] CON-07 Explicit overwrite requires confirmation and writes the intended
  revision.
- [ ] CON-08 A conflict arising during Save is detected before commit.

## 5. Editing, undo, find, and replace

- [ ] EDT-01 Character, word, line, page, and document movement follows platform
  conventions.
- [ ] EDT-02 Shift variants extend and shrink selection predictably.
- [ ] EDT-03 Mouse selection, drag, double-click word, and triple-click line work.
- [ ] EDT-04 Cut, Copy, Paste, Delete, Select All, and clipboard cancellation work.
- [ ] EDT-05 Consecutive typing forms one intuitive undo transaction.
- [ ] EDT-06 Backspace and forward delete coalesce independently.
- [ ] EDT-07 Paste, Replace All, EOL conversion, and formatting are distinct
  one-step undo transactions.
- [ ] EDT-08 Undo and Redo restore content, caret, and selection.
- [ ] EDT-09 Find next, previous, wrap, case toggle, count, and no-match state work.
- [ ] EDT-10 Replace respects selection or whole-document scope.
- [ ] EDT-11 Go To Line validates bounds and reaches the expected logical line.
- [ ] EDT-12 Word wrap changes layout without changing bytes.

## 6. Unicode and IME

- [ ] TXT-01 Type and edit CJK with a real IME.
- [ ] TXT-02 Pre-edit text is distinguishable and is not committed, recovered, or
  undoable before IME commit.
- [ ] TXT-03 The IME candidate window follows the caret while scrolling.
- [ ] TXT-04 Dead keys and composed Latin characters work.
- [ ] TXT-05 Emoji, variation selectors, and zero-width-joiner sequences survive
  navigation, selection, clipboard, save, and undo.
- [ ] TXT-06 Combining marks survive navigation, selection, clipboard, save, and
  undo.
- [ ] TXT-07 Bidirectional samples render and edit without corruption or panic.
- [ ] TXT-08 A 100,000-character single line remains bounded and recoverable.

## 7. Accessibility and keyboard

- [ ] A11Y-01 Complete New, Open, Edit, Find, Save, conflict, recovery, and Close
  workflows without a mouse.
- [ ] A11Y-02 Menus expose names, enabled state, checked state, and shortcuts.
- [ ] A11Y-03 The editor exposes text, caret, selection, editable state, and
  actions to the platform screen reader.
- [ ] A11Y-04 Modified, error, recovery, find, and conflict states are announced.
- [ ] A11Y-05 Focus order is stable and focus never becomes trapped.
- [ ] A11Y-06 Information is not conveyed by color alone.
- [ ] A11Y-07 High-contrast mode remains legible.

Required real readers:

- [ ] Windows: NVDA
- [ ] macOS: VoiceOver
- [ ] Linux: Orca

## 8. Theme, display, and window behavior

- [ ] UI-01 System, Light, and Dark work and persist.
- [ ] UI-02 System theme changes are followed according to the platform contract.
- [ ] UI-03 Selection, caret, focus, disabled controls, links, errors, and
  conflicts meet contrast expectations.
- [ ] UI-04 100, 125, 150, 175, and 200 percent scaling remain crisp and usable.
- [ ] UI-05 Zoom works from keyboard, menu, and supported pointer gesture.
- [ ] UI-06 Window state restores on the same display.
- [ ] UI-07 Removed or rearranged displays cannot restore the window off screen.
- [ ] UI-08 System dialogs, Command versus Control shortcuts, and window close
  behavior match the platform.

## 9. Performance and resources

Attach the reproducible benchmark report rather than estimating subjectively.

- [ ] PERF-01 Warm launch p95 meets the requirement.
- [ ] PERF-02 1 MiB open and edit p95 meets the requirement.
- [ ] PERF-03 50 MiB first editable frame p95 meets the requirement.
- [ ] PERF-04 Input-to-frame and scroll percentiles meet the requirement.
- [ ] PERF-05 50 MiB search p95 meets the requirement.
- [ ] PERF-06 Idle and 50 MiB RSS meet the requirement.
- [ ] PERF-07 The pathological long-line corpus remains responsive and bounded.
- [ ] PERF-08 The stripped artifact meets its target and ceiling.
- [ ] PERF-09 No background task commits a stale result after rapid editing or
  document replacement.

## 10. Privacy and security

- [ ] SEC-01 Runtime traffic capture shows no outgoing application connection.
- [ ] SEC-02 Opening one file causes no unrelated directory or document reads.
- [ ] SEC-03 Recovery and configuration permissions match the platform policy.
- [ ] SEC-04 Default logs contain no content, clipboard text, recovery bytes, or
  full paths.
- [ ] SEC-05 Markdown samples with remote images, HTML, and links trigger no
  fetch or execution.
- [ ] SEC-06 Dependency, license, advisory, SBOM, and provenance evidence is
  attached.

## 11. Packaging and soak

- [ ] REL-01 Portable artifact works on a clean supported system.
- [ ] REL-02 Installer install, launch, upgrade, and uninstall work cleanly.
- [ ] REL-03 File associations, if shipped, open only the selected path.
- [ ] REL-04 Checksums and signatures verify.
- [ ] REL-05 Two named testers completed at least 14 days each.
- [ ] REL-06 At least one tester is not the primary developer.
- [ ] REL-07 At least one full soak occurred on a non-Windows platform.
- [ ] REL-08 No data-loss incident or unresolved critical or high defect occurred.

## 12. Markdown v0.2, when applicable

- [ ] MD-01 Markdown off schedules no parser work and changes no behavior.
- [ ] MD-02 Source punctuation remains visible under inline styling.
- [ ] MD-03 Diagnostics are non-mutating and accessible.
- [ ] MD-04 Each explicit fix is one undo transaction.
- [ ] MD-05 Format shows an accurate diff and requires confirmation.
- [ ] MD-06 Format is idempotent, parsed-document equivalent, and preserves BOM
  and EOL policy.
- [ ] MD-07 Remote images, HTML, and links cause no network or execution.
- [ ] MD-08 Stale parser results never appear on a newer revision.

## Sign-off

- Overall result:
- Failed or blocked item IDs:
- Issue links:
- Residual risks accepted and approver:
- Tester signature and date:
- Release approver and date:
