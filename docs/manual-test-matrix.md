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

The pure lifecycle reducer has exhaustive decision and save-continuation unit
cases plus a fixed-seed 512-case command-sequence model property. Native dialogs,
filesystem effects, and window-manager behavior still require these rows.

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
- [ ] LIF-10 After an indeterminate save, New, Open, Quit, and native Close keep
  the reconciliation guidance visible. Cancel restores that guidance. Every
  Save and Save As remains blocked before destination inspection or mutation,
  including after New or Open, until each record is explicitly reconciled.
  Dismiss notice hides only the notice, and attempting a save resurfaces every
  active record. Each record shows a bounded destination label and an explicit
  Copy Destination Path action. A non-Unicode path instead offers a clearly
  labeled reversible hexadecimal operating-system representation with no
  replacement characters. Reconcile repeats the diagnostic and path action and
  requires confirmation that the user inspected the destination and retained
  sibling and preserved the needed version. Confirming removes only that record
  and performs no write, retry, or document mutation. Removing the last record
  re-enables Save and Save As without leaving stale block guidance. With a
  fault-injected full 16-record ledger, verify another save is refused before
  destination work and the bounded, scrollable guidance remains usable.

## 2. Open, Save, and byte fidelity

The partial 2026-07-31 local record in
[M1_FILESYSTEM_EVIDENCE.md](M1_FILESYSTEM_EVIDENCE.md) covers native NTFS and
WSL2 ext4 fixtures plus a fail-closed Windows-to-WSL boundary. It does not mark
the rows below complete because Windows encryption, creation and identifier
policy, macOS, network, cloud, removable, weak-filesystem, cross-identity, and
crash-persistence cases remain unproved.

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
- [ ] IO-15 A newly created Unix document is mode 0600. Under a parent with a
  broader inheritable ACL, an ordinary macOS control file inherits the ACE while
  the protected file immediately reports true ACL absence; absence is verified
  again before the first byte is written. A new Windows document has a protected
  DACL granting full control only to its owner and SYSTEM.
- [ ] IO-16 A multiply hard-linked destination is refused until explicit
  GUI confirmation; after confirmation only the selected name receives new
  bytes and the dialog states that other names keep the previous revision.
- [ ] IO-17 Windows existing-file replacement preserves DACLs, named streams,
  compression, encryption, and the documented creation and identifier policy.
- [ ] IO-18 Windows `ReplaceFileW` success and errors 1175, 1176, and 1177 leave
  only states the reconciliation model classifies correctly; uncertain states
  retain private artifacts and never invite blind retry.
- [ ] IO-19 Linux replacement preserves exact mode, attainable ownership, ACLs,
  visible extended attributes, SELinux context, and capabilities. A post-commit
  metadata failure retains the displaced revision and reports the safest state
  reached without claiming the save did not commit. An exactly validated
  retained artifact is mode 0600; a restriction failure is explicit.
- [ ] IO-20 macOS replacement preserves mode, attainable ownership, ACLs,
  extended attributes, resource forks, and quarantine data; modification time
  advances according to policy, and BSD file-flag behavior is recorded. An
  exactly validated retained artifact is mode 0600 with no ACL; a restriction
  failure is explicit.
- [ ] IO-21 Cleanup and every file or parent durability warning remain distinct,
  and none is reported as a failed or uncommitted save. A file-barrier failure
  reports Best Effort even if the parent barrier succeeds.
- [ ] IO-22 Every retained Unix sibling warning names the random basename,
  describes only the artifact states established by reconciliation, uses
  neutral wording where concurrent bytes remain possible, and gives safe
  inspection and removal guidance.

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

The transaction, history, and literal-search models have automated inverse,
UTF-8 boundary, directional-selection, saved-content identity, stale-revision,
count-limit, byte-limit, coalescing-boundary, wrap-navigation, case-policy, and
replacement-scope coverage. Fixed-seed reference properties cover Unicode
typing, literal search, and replacement. These checks do not replace the manual
platform rows.

Automated native UI coverage proves Select All in both views, exact directional
selection carry across Markdown blocks and mixed line endings, UTF-8 boundary
rejection, and replacement of only selected source bytes. EDT-04 remains open
until Cut, Copy, Paste, Delete, cancellation, and platform behavior pass the
full manual matrix.

- [ ] EDT-01 Character, word, line, page, and document movement follows platform
  conventions.
- [ ] EDT-02 Shift variants extend and shrink selection predictably.
- [ ] EDT-03 Mouse selection, drag, double-click word, and triple-click line work.
- [ ] EDT-04 Cut, Copy, Paste, Delete, Select All, and clipboard cancellation work.
- [ ] EDT-05 Consecutive typing forms one intuitive undo transaction.
- [ ] EDT-06 Backspace and forward delete coalesce independently.
- [ ] EDT-07 Paste, Replace All, EOL conversion, and formatting are distinct
  one-step undo transactions.
- [ ] EDT-08 Edit-menu Undo and Redo plus Ctrl+Z, Ctrl+Y, Cmd+Z, and
  Shift+Cmd+Z restore content, caret, directional selection, and saved-content
  state.
- [ ] EDT-09 Find next, previous, wrap, case toggle, count, and no-match state work.
- [ ] EDT-10 Replace respects selection or whole-document scope.
- [ ] EDT-11 Go To Line validates bounds and reaches the expected logical line.
  Switching to Markdown Mode, creating a new document, or opening another
  document closes the dialog and discards its prior input, error, and focus.
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

- [ ] UI-01 System, Light, Dark, Green Screen, and Amber Screen work and
  persist. Returning from a specialty theme restores the standard palette.
- [ ] UI-02 System theme changes are followed according to the platform contract.
- [ ] UI-03 Selection, caret, focus, disabled controls, links, errors, and
  conflicts meet contrast expectations.
- [ ] UI-04 100, 125, 150, 175, and 200 percent scaling remain crisp and usable.
- [ ] UI-05 Zoom works from keyboard, menu, the Markdown document-bar controls,
  and supported pointer gesture. Every route uses the same 50 to 300 percent
  state and scales document typography without scaling application chrome. The
  document-bar reset control announces the current percentage, and repeated
  keyboard or screen-reader activation retains control focus without activating
  formatted content.
- [ ] UI-06 Window state restores on the same display.
- [ ] UI-07 Removed or rearranged displays cannot restore the window off screen.
- [ ] UI-08 System dialogs, Command versus Control shortcuts, and window close
  behavior match the platform.
- [ ] UI-09 The Markdown document uses the same borderless editor surface and
  outer gutter as Text Mode, without a card, document box, or centered measure,
  in System, Light, Dark, Green Screen, and Amber Screen. Narrow windows remain
  unclipped and never create a nested surface.
- [ ] UI-10 Command or Control plus Shift plus S invokes Save As rather than
  Save, and every displayed shortcut uses the platform convention.
- [ ] UI-11 Markdown files at the exact source-byte ceiling, including UTF-8
  multibyte text and a BOM, enter Markdown Mode without byte changes. A file
  that exceeds the source, logical-line, line-length, block-count, block-span,
  or parser-event ceiling stays unchanged and saveable in Text Mode and names
  the temporary budget it exceeded.
- [ ] UI-12 At ordinary widths, File, View, and Help align left while Mode and
  Theme align right on the same row. At the 420-pixel minimum width, labeled
  mode and theme menus remain visible without overlap. The formatting row is
  absent in Text Mode. Keyboard focus and screen-reader traversal follow the
  same left-to-right sequence as the visible controls, including Zoom Out,
  the current-percentage Reset Zoom control, and Zoom In.
- [ ] UI-13 Invalid declarative custom themes fail closed to Dark with an
  actionable explanation and no partially applied colors or external access.
- [ ] UI-14 Reload is disabled and announced as unavailable for Untitled, then
  becomes enabled after the document owns a filesystem path.

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

- [ ] SEC-01 Runtime traffic capture shows no unexpected background connection;
  an explicit release-link action is separately identified.
- [ ] SEC-02 Opening one file causes no unrelated directory or document reads.
- [ ] SEC-03 Recovery and configuration permissions match the platform policy.
- [ ] SEC-04 Default logs contain no content, clipboard text, recovery bytes, or
  full paths.
- [ ] SEC-05 Markdown samples with remote images and HTML trigger no fetch or
  execution; links open externally only after an explicit click.
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
- [ ] REL-09 The five README screenshots are regenerated from the cited native
  release build, contain the same polished non-sensitive demo text, and pass
  visual review at 100 and 150 percent scaling. The Light Text and Markdown
  pair proves view and gutter parity; Dark, Green Screen, and Amber Screen prove
  the remaining public themes.

## 12. Native Markdown Mode

- [ ] MD-01 Text Mode schedules no Markdown work and exposes exact source.
- [ ] MD-02 Switching between Text Mode and Markdown Mode changes no bytes.
- [ ] MD-03 Clicking or dragging formatted content activates the matching
  complete source range, keeps supported syntax visually formatted, and
  updates only that range. Hidden delimiters, escapes, and character references
  are never split by a visual caret or selection; unsafe synthesis falls back
  to visible source.
- [ ] MD-04 Paragraph and all six ATX heading levels are exact idempotent style
  choices; each changed style, formatting action, and safe fix is one minimal
  undo transaction. Choosing the current style creates no edit or selection
  reversal. Indented markers, tab separators, and optional closing ATX
  sequences preserve rendered content; code, setext headings, nested blocks,
  unsupported structures, and paragraphs with ambiguous leading whitespace or
  trailing closing-style hash runs show Unavailable and remain byte-exact. At
  the 420 by 300 minimum viewport, all compact Format actions and the nested
  style choices remain visibly reachable.
- [ ] MD-05 Diagnostics are revision-tagged, non-mutating, and accessible.
- [ ] MD-06 Format shows an accurate diff and requires confirmation.
- [ ] MD-07 Format is idempotent, parsed-document equivalent, and preserves BOM
  and EOL policy.
- [ ] MD-08 Remote images and HTML cause no network or execution; links require
  an explicit external-open action.
- [ ] MD-09 Stale parser results never appear on a newer revision.
- [ ] MD-10 Keyboard selection, IME, screen readers, high DPI, and all themes
  pass the same document-editing expectations as Text Mode.
- [ ] MD-11 Active and inactive Markdown use real heading and strong-emphasis
  font weights. Supported punctuation stays hidden, while a selected link
  target becomes visible only for editing.
- [ ] MD-12 Prose spacing, wrapping, list markers, multiline quotes, and mixed
  inline weights render without collisions, missing spaces, or source leakage.
- [ ] MD-13 The five native README captures use the same polished demo document,
  align menus and controls, and match the exact candidate renderer. The Light
  Text and Markdown pair preserves the same outer gutter, approximate scroll
  position, selection, and source bytes across the view switch.
- [ ] MD-14 Same-frame input followed by Escape commits exact source before the
  active range closes, and Undo and Redo restore the final caret. An adversarial
  same-frame paste that exceeds a structural ceiling receives only bounded plain
  layout work before exact-source fallback to Text Mode.

## Sign-off

- Overall result:
- Failed or blocked item IDs:
- Issue links:
- Residual risks accepted and approver:
- Tester signature and date:
- Release approver and date:
