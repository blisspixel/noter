# Changelog

All notable project changes are recorded here. Noter has not published a stable
release, so current work remains under Unreleased.

## Unreleased

### Added

- Add a public vulnerability-reporting policy with a private-reporting path and
  guidance that keeps sensitive details and real documents out of public issues.
- Add persisted System, Light, Dark, Green Screen, and Amber Screen themes with
  deliberate document typography. The specialty palettes retain native text
  shaping and enforce enhanced text, selection, and control contrast in tests.
- Add a fail-closed specialty-palette validator that reconstructs the complete
  standard Dark state after invalid input, and document the safe declarative
  custom-theme boundary.
- Add an early native Markdown Mode with source-backed formatted editing, H1,
  H2, Bold, Italic, Link, Code, List, and Quote actions, plus five conservative
  source diagnostics. Supported heading and inline delimiters remain hidden in
  the active editor while the file stays ordinary Markdown on disk.
- Bundle the variable Inter typeface under the SIL Open Font License so headings
  and strong emphasis use real font weights consistently across platforms.
- Add working About Noter and truthful update-status dialogs, including the
  `noter update` entry point.
- Add locked source-install helpers for PowerShell and POSIX systems with check
  and custom-root modes.
- Add deterministic native Light and Dark README screenshot generation and
  validation using a polished non-sensitive Markdown demo document. The capture
  keeps formatted content active so the direct editor and formatting controls
  are visible together while suppressing transient focus pixels.
- Add revision-checked edit transactions with exact UTF-8 source expectations,
  exact inverses, directional selections, operation origin, and adapter-supplied
  monotonic timestamps.
- Add Edit-menu Undo and Redo with Ctrl+Z, Ctrl+Y, Cmd+Z, and Shift+Cmd+Z paths.
  History is shared by Text and Markdown modes and bounded by both transaction
  count and retained source bytes.

### Changed

- Place the responsive Text and Markdown mode control and the current Theme
  menu on the upper-right of the application menu row. The second toolbar now
  appears only in Markdown Mode and contains formatting actions only. Visual,
  keyboard, and accessibility-tree order follow the same left-to-right sequence.
- Preserve the current directional source selection across Undo, Redo, editing
  mode switches, and safe fallback from an over-budget Markdown projection.
- Map click-and-drag selection in inactive formatted Markdown back to complete
  source spans, including hidden delimiters, escapes, and parser-decoded
  character references, so formatting commands operate on the text the user
  selected. Synthesized text that cannot be mapped without invention remains
  visibly editable as source.
- Commit focused Markdown input before activating another block, finishing the
  active edit, or applying a requested mode change, including when multiple
  input events arrive in one native frame.
- Verify version output and invalid-argument status, standard output, and
  standard error against the installed release executable in both source
  installers.

### Security

- Restrict any race-safe Unix displaced-document recovery artifact to
  owner-only mode through its verified open handle. On macOS, remove and verify
  the absence of extended access-control entries before retaining it.
- Bound PNG validation by regular-file type, repository symlink policy, encoded
  size, decoded size, and exact RGBA dimensions. Markdown link checking now
  verifies one opened file identity, refuses path swaps and symlinks, reads
  through that descriptor with an exact size ceiling, and rejects invalid UTF-8
  so untrusted pull requests cannot turn CI validation into an unbounded read or
  decompression operation.
- Escape control characters in rejected command-line values before writing
  diagnostics to a terminal.
- Bind every document observation and reopen to a native no-follow handle. Unix
  uses `O_NOFOLLOW`; Windows opens the final entry with
  `FILE_FLAG_OPEN_REPARSE_POINT`, preserves ordinary sharing, and rejects link
  or reparse metadata before reading content.
- Bound document loading and save-target hashing to the explicit 64 MiB v0.1
  limit, including protection against concurrent file growth.
- Bound Unix extended-attribute snapshots to 4,096 entries and 64 MiB of
  aggregate names and values before value allocation, including macOS resource
  forks.
- Create Windows staging and new files with a protected owner-and-system DACL so
  permissive parent ACLs cannot expose staged document bytes.
- Create macOS staging and new files by requesting mode 0600 and a zero-entry,
  no-inherit ACL in the same `openx_np` operation. Native evidence proves the
  resulting file has true ACL absence while an ordinary control file inherits
  the parent ACE. Defensively remove and verify ACL absence through the live
  descriptor before writing any document bytes, and report the random zero-byte
  sibling if security finalization fails.
- Add a pinned RustSec audit gate to CI.
- Add an explicit cargo-deny policy and pinned CI gate for dependency licenses,
  registry and Git sources, wildcard versions, advisories, and duplicate-version
  visibility.
- Pin the coverage tool used by CI.

### Fixed

- Render conservative line-wide emphasis-spacing mistakes such as `*text *` as
  intended in Markdown Mode while reporting MD037 and preserving exact source
  until an explicit correction. Flush inactive render runs whenever visible
  style changes so trailing punctuation or whitespace cannot erase emphasis.
- Check the more specific Save As shortcut before Save, use Command on macOS
  and Control on Windows and Linux, and derive displayed shortcut text from the
  same command metadata.
- Return command-line status 2 with actionable usage for invalid arguments, and
  accept `--` before a document path that begins with a dash. A closed standard
  output or error pipe no longer panics the Windows release process.
- Restore the last authoritative document text and reset editor-local state if
  an in-memory edit cannot advance the document revision.
- Track dirty state against the last saved serialized-content fingerprint so
  Undo and Redo can return to clean saved bytes while revisions remain monotonic.
- Bound the synchronous pre-alpha Markdown projection by source bytes, logical
  lines, line length, block count, block span, and parser events. Over-budget
  files remain unchanged in Text Mode, and Markdown diagnostic counts are
  cached by document generation and revision without retaining a diagnostic
  vector.
- Replace the dirty-document close trap with a Save, Discard Changes, and Cancel
  decision for New, Open, Quit, and native window close.
- Retain indeterminate-save recovery guidance through every dirty-document
  decision and Cancel path, and block an ordinary Save retry until the user has
  reconciled that state or chosen Save As.
- Keep a link destination visible and selected while it is being edited in
  Markdown Mode, then hide the source target again when the caret leaves it.
- Preserve replacement artifacts whose identity or bytes changed during cleanup.
- Delete Windows cleanup candidates through the exact verified open handle so a
  rebound pathname cannot redirect deletion.
- Keep Unix staging owner-only from creation through atomic exchange, finalize
  metadata after commit, and retain artifacts when safe handle-bound cleanup is
  unavailable.
- Preserve Unix destination metadata from an immutable pre-commit snapshot,
  verify the displaced original after atomic exchange, require its stable
  metadata payload to still match the snapshot, and never apply unratified or
  stale metadata to the committed file.
- Serialize the macOS ACL into the immutable metadata snapshot and replay it
  through the destination descriptor, eliminating temporary ACL paths while
  keeping resource forks and other xattrs inside the bounded snapshot. Treat
  macOS `ENOENT` from `acl_get_fd` as a distinct absent state, replay it with the
  native remove-ACL sentinel, and verify true absence. Record the native kernel
  behavior that replaying explicit zero-entry ACL text canonicalizes to absence.
- Surface exact save cleanup and durability warnings instead of a generic
  success warning.
- Process same-frame editor input before file commands and native close checks.
- Carry the exact pre-dialog Save As target expectation through hard-link
  confirmation so a rebound destination conflicts instead of being overwritten.
- Preserve creation-time identity failures and retained-sibling cleanup guidance
  as distinct typed errors.
- Detect a same-authority Windows staging mutation before replacement or during
  the final handoff, and classify postcommit mismatch as indeterminate.
- Make About Noter open a truthful project dialog and state the exact limits of
  the current Markdown implementation.
- Preserve editor focus and the exact source selection when switching from Text
  Mode to Markdown Mode or invoking a formatting control. A first click or drag
  on formatted content now activates that range in the same frame.

### Engineering

- Route plain-text input, direct formatted Markdown input, and Markdown
  formatting through the same atomic document mutation boundary. Add fixed-seed
  512-case reference-model properties for single replacements, ordered
  multi-edit transactions, and arbitrary Undo and Redo sequences.
- Close a focused 118-candidate mutation campaign for the transaction and
  history modules with 95 caught and 23 validated compiler rejections. The
  infrastructure validator reports no miss, timeout, or hidden tool failure.
- Raise measured Windows-local line coverage to 94.59 percent for the trust
  kernel and 92.56 percent for the complete workspace. The new transaction and
  history modules measure 97.30 and 97.61 percent respectively.

- Upgrade eframe and egui to 0.35 while retaining restricted features and
  enabling current shaping, hinting, theme-aware font transfer, and subpixel
  placement behavior. Replace the secondary Markdown renderer with one
  pre-layout native projection built directly from `pulldown-cmark`, reducing
  the dependency graph while keeping active and inactive content on the same
  real-weight style mapping.
- Lock the renderer's distinct light-background and dark-background
  coverage-transfer behavior with a regression test.
- Define and enforce repository-wide code-quality and evidence standards.
- Enforce Ruff linting and formatting for repository validation scripts in CI,
  and normalize text files to LF across supported development platforms.
- Keep local automation state and runtime logs in ignored dedicated directories.
- Remove obsolete tracked automation metadata and commented-out build or CI
  plans.
- Expand mutation enforcement through the native platform adapter with a macOS
  job and a current 741-candidate supported-platform union with no gap. Hosted
  run 30213398323 completed the 49-candidate macOS scope without a survivor and
  exposed only Linux decision-coverage gaps plus two shared line-scanner
  timeouts. The settled local correction removes mutable progress arithmetic,
  gives repeated native decisions exact named predicates, and retains all
  supported-platform candidates across Linux, Windows, and macOS. Exact-commit
  run 30221793209 passes the complete matrix: Linux 617 total with 438 caught,
  Windows 557 total with 381 caught, and macOS 49 total with 43 caught. Every
  remaining candidate is a validated compiler rejection; no scope has a miss or
  timeout.
- Give Windows mutation tests a 60-second minimum test-process timeout after a
  prior hosted run timed out one mutant even though its truth-table test had
  already failed. A focused rerun catches all four mutations of that predicate;
  the 90-minute outer job limit remains unchanged.
- Reject linker, compiler, storage, process, and tool-lock infrastructure
  failures that cargo-mutants would otherwise classify as unviable. Normalize
  ANSI-decorated logs and reject clang linker signal crashes after post-run
  review found one hidden in the otherwise green run 30219731527; corrected run
  30221793209 catches that mutant and passes the strengthened validator.
- Close a focused 58-candidate Markdown diagnostics campaign with a composite
  result of 55 caught and three genuine compiler rejections after isolating and
  rerunning one Windows linker-lock failure.
- Close focused mutation campaigns for lifecycle and save-result decisions with
  26 of 26 candidates caught, and for final native Markdown editing and
  rendering with 80 of 88 candidates caught plus eight genuine compiler
  rejections. Both final reports pass infrastructure validation with no miss or
  timeout.
- Close the focused 16-candidate final-entry observation campaign with 12
  candidates caught and four genuine compiler rejections. A first pass exposed
  one handle-kind truth-table gap; the settled rerun has no miss, timeout, or
  infrastructure failure.
- Maintain measured fixed-seed line coverage at 93.73 percent for the trust
  kernel and 89.49 percent for the complete workspace. The Windows-local suite
  contains 208 tests, and CI enforces respective 90 and 80 percent floors.
