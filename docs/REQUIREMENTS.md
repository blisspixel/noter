# Noter Product Requirements

**Version:** 0.4

**Reviewed:** 2026-07-26

**Status:** Ratified contract for the first public-quality release

This document defines what Noter must do. [DESIGN.md](DESIGN.md) defines how,
[ROADMAP.md](ROADMAP.md) defines when, and dated evidence proves whether a
requirement is complete. When those documents disagree, this contract and the
latest ratified architecture decision record govern.

## 1. Product promise

Noter is a focused, single-document, cross-platform editor for ordinary `.txt`
and `.md` files. It combines a classic Text Mode with an editable native Markdown
Mode, diagnostics, and explicit formatting. The file on disk
remains UTF-8 text or standard Markdown source. Noter has no telemetry, accounts,
cloud service, or proprietary document format.

The first public-quality release must earn trust in both text and Markdown
workflows. Markdown depends on the same save, undo, lifecycle, accessibility,
privacy, and performance guarantees as plain text. It is not a separate rich-text
document model and cannot weaken those guarantees.

### 1.1 User mental model

The product must preserve these expectations:

1. The document is text or Markdown source. Markdown Mode is a reversible,
   directly editable projection of that source, and Text Mode always exposes the
   exact saved representation.
2. Save writes the current intended revision to the selected file.
3. Save never silently changes encoding, BOM state, existing line endings, or
   Unicode normalization.
4. Undo reverses the last intentional edit transaction.
5. New, Open, Reload, Close, and Quit cannot discard dirty work without an
   explicit Save, Discard, or Cancel decision.
6. Crash recovery is a safety net, not a hidden replacement for Save.
7. The application reads no unrelated document and performs no background
   network activity. A user-initiated update action may contact the documented
   release endpoint without document data or a persistent identifier.

Any intentional exception requires visible UI, an explicit command, one-step
undo where content changes, and a testable specification.

### 1.2 Release vocabulary

- **Planned:** accepted but not implemented.
- **In progress:** implementation exists but one or more required gates are
  missing.
- **Verified:** implementation and all named evidence exist on the same green
  commit.
- **Deferred:** intentionally outside the named release.

Feature presence alone is not verification.

## 2. Core functional requirements

### 2.1 Document and file operations

- **FR-010 New:** Create one clean, untitled document per window.
- **FR-011 Open:** Open a user-selected regular file through a system file
  dialog. Refuse a final symlink or Windows reparse point in v0.1; following a
  link requires a future resolved-target identity contract.
- **FR-012 Strict UTF-8:** Accept UTF-8 with or without a UTF-8 BOM. Reject
  invalid UTF-8 without replacement characters. A future explicit import flow
  may create a new untitled converted document, but it must never overwrite the
  source implicitly. v0.1 refuses documents above 64 MiB before allocating or
  hashing beyond that bound; the 50 MiB performance corpus remains inside the
  supported range.
- **FR-013 Save:** Save to the current regular-file path through the durable
  replacement protocol in NFR-REL-02. A multiply hard-linked destination
  requires an explicit GUI confirmation that only the selected directory entry
  will advance and that other names retain the previous revision. Confirmation
  must retain the exact pre-dialog target expectation; it must not rebaseline a
  path that changes while the dialog is visible.
- **FR-014 Save As:** Ask for a destination every time. The document path and
  clean revision change only after the new destination commits successfully.
  Refuse an existing final symlink or unsupported reparse point rather than
  replacing or following it implicitly. A failed or indeterminate save that may
  leave a private sibling must identify the safe random basename and give
  explicit inspection, recovery, retry, and removal guidance. If identity
  inspection or platform-specific privacy finalization fails immediately after
  exclusive creation, report the creation failure and any retained-sibling
  cleanup failure separately.
- **FR-015 Recent files:** Maintain at most ten deduplicated user-opened paths.
  Missing or inaccessible entries fail safely and can be removed without
  reading their parent directories.
- **FR-016 BOM fidelity:** Preserve the loaded UTF-8 BOM state until the user
  invokes an explicit conversion command.
- **FR-017 Line-ending fidelity:** Preserve all existing LF, CRLF, and CR byte
  sequences. Untouched content must round-trip byte for byte. Editing a
  mixed-EOL document must not normalize unrelated lines.
- **FR-018 Explicit EOL conversion:** A user may convert all line endings through
  one explicit command. The command is one undo transaction and updates the
  status bar immediately.
- **FR-019 File identity:** Save and external-change decisions use recorded file
  identity and content fingerprints, not only a display path or timestamp.

### 2.2 Editing

- **FR-020 Text input:** Support Unicode keyboard input, dead keys, CJK IME
  composition, emoji, combining marks, and bidirectional text without
  corruption or panic.
- **FR-021 Navigation:** Support expected character, word, line, document, and
  page movement with and without selection on each platform.
- **FR-022 Clipboard:** Cut, Copy, Paste, Delete, and Select All share the same
  command path as their menu items and platform shortcuts.
- **FR-023 Undo and Redo:** Maintain a bounded history of edit transactions.
  Typing and deletion coalesce predictably; paste, replace, EOL conversion, and
  formatting are distinct one-step transactions.
- **FR-024 Find:** Provide a non-modal find bar, literal search, case matching,
  next, previous, wrap indication, and visible match count.
- **FR-025 Replace:** Provide Replace and Replace All with an explicit current
  selection or whole-document scope.
- **FR-026 Go To Line:** Navigate to a validated one-based logical line.
- **FR-027 Word wrap:** Toggle wrapping without changing document bytes.
- **FR-028 Long operations:** Search, indexing, and formatting cannot commit a
  stale result to a newer document revision.

### 2.3 Lifecycle and recovery

- **FR-060 Dirty decision:** New, Open, Reload, Close, and Quit use one
  Save / Discard / Cancel state machine.
- **FR-061 Window close:** Closing a dirty window cannot complete until Save
  succeeds, Discard is explicitly confirmed, or the action is cancelled.
- **FR-062 Recovery location:** Store private recovery records in the
  per-user application state or local-data directory, never the general
  temporary directory.
- **FR-063 Recovery point objective:** After the first edit, persist a valid
  recovery record after at most 15 seconds of continued editing and normally
  within 2 seconds of idle time.
- **FR-064 Recovery integrity:** Each record has a schema version, random
  document and instance IDs, revision, checksum, original-path metadata, and
  atomic manifest update.
- **FR-065 Recovery launch:** On startup, validate records and offer recovery
  before replacing them with a normal untitled document.
- **FR-066 Recovery isolation:** Recovered content opens as dirty and never
  writes the original file until the user invokes Save.
- **FR-067 Recovery cleanup:** Remove a record only after a successful save or
  explicit discard. Corrupt records are quarantined and explained, not silently
  deleted.
- **FR-068 Recovery failure:** A persistence failure is visible and does not
  suppress the classic dirty prompt.
- **FR-069 External change:** Detect changed, replaced, deleted, or recreated
  files on focus and periodic checks. Never overwrite a detected conflicting
  revision without a user decision.

### 2.4 Interface and status

- **FR-030 Status:** Show path or Untitled, modified state, one-based logical line
  and column, selection size, encoding, BOM, and EOL classification.
- **FR-031 Theme:** Provide System, Light, Dark, Green Screen, and Amber Screen
  through a visible top-level selector. Persist the choice and follow system
  changes when System is selected. Specialty themes must retain the same text
  shaping and accessibility behavior as the core themes. Custom palettes are
  declarative, size-bounded, and fail closed through the same contrast
  validator; they cannot execute code or reference external assets.
- **FR-032 Zoom:** Provide keyboard and menu zoom with a readable bounded range.
- **FR-033 Window state:** Restore valid size, position, and maximized state
  without placing a window entirely off screen.
- **FR-034 Commands:** Every visible command either works, is visibly disabled
  with a reason, or is absent. Placeholder commands are forbidden.
- **FR-035 Errors:** Errors state what failed, what was preserved, and the next
  safe action. Document content and paths are not copied into logs by default.
- **FR-036 Multiple instances:** Independent windows are supported. No global
  single-instance lock is required.

### 2.5 Platform behavior and accessibility

- **FR-080 Shortcuts:** Use Command on macOS and Control on Windows and Linux,
  with platform-standard alternatives where conventions differ.
- **FR-081 Keyboard reachability:** All primary workflows, dialogs, bars, menus,
  and recovery actions work without a mouse.
- **FR-082 Semantics:** Expose names, roles, values, selection, caret, editable
  text actions, and status changes through the platform accessibility bridge.
- **FR-083 Screen readers:** Release testing covers NVDA, VoiceOver, and Orca.
- **FR-084 IME:** Pre-edit text remains distinguishable from committed text and
  the candidate window follows the caret.
- **FR-085 Display:** Support high DPI, 125 to 200 percent scaling, high contrast,
  and visible focus and selection states.
- **FR-086 Dialogs:** Use system file dialogs. The rendered application chrome is
  consistent and system-integrated; native widget appearance is not promised.

### 2.6 Installation and updates

- **FR-090 Install:** Provide supported per-user installer commands for Windows,
  macOS, and Linux that select the correct published artifact, verify it, and do
  not require administrator access by default.
- **FR-091 Update command:** `noter update` checks the documented release channel,
  shows the offered version and trust information, and performs a verified
  upgrade without losing the working installation on failure.
- **FR-092 Update UI:** Help > Check for Updates invokes the same version and
  verification policy as the command-line updater.
- **FR-093 Explicit network:** Update checks occur only after an explicit action
  unless the user later enables a clearly labeled periodic check. Requests contain
  no document data, path, account, or persistent installation identifier.
- **FR-094 Package ownership:** A package-manager installation is updated by that
  package manager. Noter must not replace files owned by another installer.
- **FR-095 Lifecycle safety:** An update cannot begin while unsaved work exists and
  must preserve settings, recovery records, and the previous executable until the
  new artifact is verified and committed.
- **FR-096 Uninstall:** Every supported install path documents complete uninstall
  behavior and distinguishes binaries, preferences, and recovery data.

The complete release and verification contract is in
[INSTALLATION.md](INSTALLATION.md).

## 3. Native Markdown requirements

- **FR-100 Modes:** Text Mode opens any supported document as exact source.
  Markdown files can switch to Markdown Mode without changing bytes. Text Mode
  schedules no Markdown work. The primary mode control remains visible in the
  top application row; Markdown formatting controls consume a separate row only
  while Markdown Mode is active.
- **FR-101 Source authority:** Source remains the authoritative document.
  Markdown Mode maps every direct edit to the smallest practical Markdown source
  transaction, preserves untouched source, and exposes ambiguous constructs as
  source instead of guessing.
- **FR-102 Diagnostics:** Lint findings are non-mutating and include a rule ID,
  range, explanation, and explicit fix where available.
- **FR-103 Format:** Formatting is an explicit command with a diff preview,
  parsed-document equivalence check, EOL and BOM preservation, and one-step
  undo.
- **FR-104 Rule isolation:** Heading spacing, ordered-list numbering, table
  alignment, trailing whitespace, and list continuation are independent rules.
- **FR-105 Revision safety:** Parsing and diagnostics are revision-tagged,
  cancellable, and cannot update a newer document with stale results.
- **FR-106 Content safety:** Never execute HTML, fetch links, load remote images,
  or make a network request.
- **FR-107 Conformance:** Supported syntax is ratified against CommonMark plus an
  explicitly listed subset of GitHub Flavored Markdown, if any.
- **FR-108 Views:** Provide Text Mode and Markdown Mode, with optional
  reading-focused and synchronized split layouts. Both editable modes operate on
  one source revision; switching modes never changes document bytes.
- **FR-109 Formatting controls:** Provide selection-aware Bold, Italic,
  Strikethrough, Inline Code, Link, Heading, Quote, List, Task List, and Code
  Fence commands through accessible menus and documented keyboard paths. Each
  command is one explicit `EditTransaction` and one undo step.
- **FR-110 Layout synchronization:** Rendered output is revision-tagged, rejects
  stale parser results, and supports deterministic Text-to-Markdown scroll
  mapping in a split layout.
- **FR-111 Native restricted rendering:** Markdown Mode renders a restricted
  native document model rather than arbitrary HTML or a webview. Unsupported or
  unsafe constructs remain inert and accessible in Text Mode.
- **FR-112 Direct formatted editing:** Headings, emphasis, links, lists, tasks,
  quotes, code, and supported tables are directly editable in Markdown Mode while
  retaining keyboard, selection, IME, clipboard, and accessibility parity.
- **FR-113 Minimal transactions:** A Markdown Mode operation changes only the source
  range required by the user action. It cannot normalize unrelated blocks.
- **FR-114 Malformed source:** Invalid, incomplete, or unsupported Markdown stays
  visible and editable. Parser failure cannot block Text Mode or saving.
- **FR-115 Quality profiles:** Diagnostics distinguish portable syntax problems
  from optional style policy. Every rule has a stable ID, severity, explanation,
  revision-tagged range, and safe fix only when unambiguous.
- **FR-116 Formatter determinism:** Whole-document Format is deterministic and
  idempotent. It preserves front matter and documented opaque regions, rejects a
  supported semantic-tree change, previews the diff, and commits as one undo step.

[MARKDOWN.md](MARKDOWN.md) is the normative interaction and safety specification.

## 4. Non-functional requirements

### 4.1 Reliability

- **NFR-REL-01 Byte fidelity:** Loading and saving an unedited supported file
  produces identical bytes.
- **NFR-REL-02 Durable replacement:** Saving writes a unique sibling, writes all
  bytes, flushes, syncs the file, performs an atomic platform replacement, and
  syncs the parent directory where supported. A pre-commit failure leaves the
  original complete and unchanged. Outcomes distinguish Committed, Conflict,
  Not Committed, and Commit State Unknown. A post-commit barrier failure is
  Committed with every durability warning preserved; a failed file barrier
  reports Best Effort even if parent synchronization succeeds. An uncertain
  commit retains dirty state and recovery until reconciliation.
- **NFR-REL-03 Revision safety:** A successful save clears dirty state only when
  the committed revision is still the current revision.
- **NFR-REL-04 Undo fidelity:** Applying edits and their inverse transactions
  restores identical content, selection, and caret state.
- **NFR-REL-05 Lifecycle safety:** No destructive action can bypass FR-060.
- **NFR-REL-06 Recovery safety:** Any valid acknowledged edit is either in the
  current process, a successful save, or a recovery record within the stated
  recovery point objective.
- **NFR-REL-07 Conflict safety:** A changed file is never silently overwritten by
  a stale in-memory revision.

### 4.2 Performance

Measurements use release builds, named hardware, a published corpus, at least 30
samples for latency percentiles, and explicit cold or warm state.

| Measure | v0.1 requirement |
| --- | ---: |
| Warm launch to first interactive frame | p95 at most 250 ms |
| Open and edit 1 MiB UTF-8 file | p95 at most 150 ms |
| First editable frame for 50 MiB file | p95 at most 2.0 s |
| Input to painted frame | p95 at most 16.7 ms, p99 at most 33 ms |
| Warm scroll frame time | p99 at most 16.7 ms |
| First literal-search match in 50 MiB | p95 at most 800 ms |
| Native Markdown edit to painted frame for 1 MiB | p95 at most 33 ms |
| Markdown diagnostic refresh after ordinary edit | p95 at most 150 ms |
| Text Mode to Markdown Mode switch for 1 MiB | p95 at most 250 ms |
| Idle RSS on reference Windows machine | at most 120 MiB |
| 50 MiB document RSS | at most 350 MiB |
| Stripped Windows release binary | target under 10 MiB, ceiling 12 MiB |

Files around 500 MB, files larger than memory, and instant open within one frame
are deferred until evidence supports a separate viewer or editing mode.

### 4.3 Quality and verification

- **NFR-QUAL-01 Toolchain:** Rust 1.97.1 is pinned in the repository and CI.
- **NFR-QUAL-02 Local gates:** Formatting, locked strict Clippy, locked tests,
  documentation links, and the coverage threshold pass before a commit advances.
- **NFR-QUAL-03 Coverage:** Testable product code remains at least 80 percent line
  coverage during development. v0.1 requires at least 80 percent whole-workspace
  line coverage and at least 90 percent line coverage for document, I/O,
  revision, lifecycle, and recovery modules.
- **NFR-QUAL-04 Test strength:** Critical invariants use property or model-based
  tests, I/O failure injection, golden fixtures, and mutation testing in addition
  to examples.
- **NFR-QUAL-05 UI evidence:** Semantic UI automation covers commands and state.
  Real keyboard, IME, screen-reader, display, and platform behavior remains part
  of the signed manual matrix.
- **NFR-QUAL-06 No hidden skips:** Ignored, flaky, or platform-skipped critical
  tests do not count as passing.
- **NFR-QUAL-07 Exact commit:** Required Windows, macOS, and Linux CI is green on
  the exact commit being advanced.
- **NFR-QUAL-08 Documentation:** Public behavior, FMEA, architecture decisions,
  dependency health, benchmarks, and manual evidence match the shipped commit.
- **NFR-QUAL-09 Errors:** Production paths avoid panics and unexplained
  unwraps. Typed errors preserve enough context for a safe user-facing message.

### 4.4 Security and privacy

- **NFR-SEC-01 Explicit network only:** The editor makes no background outgoing
  connection, telemetry submission, remote asset request, or automatic crash
  report. A user-initiated update action may access only the documented release
  service under FR-093.
- **NFR-SEC-02 Local scope:** Read document content only from paths the user
  explicitly opened and versioned recovery records the application created.
- **NFR-SEC-03 Private state:** Configuration and recovery use least-permission
  per-user directories. Recovery content is never written to diagnostic logs.
- **NFR-SEC-04 Dependencies:** Every direct dependency has a requirement,
  feature, size, license, maintenance, duplicate, and network-capability review.
- **NFR-SEC-05 Supply chain:** Release workflows use immutable action revisions,
  locked Rust dependencies, minimum token permissions, checksums, SBOM, and
  provenance.

## 5. Release success criteria

Noter v0.1 is releasable only when:

1. All v0.1 requirements have traceable automated or manual evidence.
2. No critical or high data-safety defect is open.
3. The performance table passes on the named reference systems.
4. Exact release-commit CI and coverage gates pass.
5. Windows, macOS, X11, and Wayland manual matrices pass.
6. Two people, including one non-primary developer and one non-Windows user, use
   the release candidate for at least 14 days each without data loss.
7. Install, portable use, upgrade, recovery, and uninstall are verified from
   clean environments.
8. Text Mode, Markdown Mode, diagnostics, and Format pass the conformance,
   source-preservation, semantic-equivalence, accessibility, and security gates.

## 6. Explicit non-goals

- Tabs, projects, folder trees, and workspaces
- Rich text or a proprietary document format
- Programming-language syntax highlighting
- LSP, Git integration, terminal, plugins, or command palette
- Accounts, synchronization, collaboration, or cloud storage
- AI features
- Background networking, remote Markdown assets, or automatic telemetry
- Non-UTF-8 save encodings
- Executable theme plugins, remote theme galleries, arbitrary theme fonts, or a
  font marketplace
- 500 MB editable-file guarantees

## 7. Traceability

The detailed matrix lives beside implementation tests and is expanded per
milestone. The minimum mapping is:

| Contract area | Milestone | Primary evidence |
| --- | --- | --- |
| FR-010 to FR-019, NFR-REL-01 to 03 | M1 | golden bytes, property tests, injected I/O failures |
| FR-020 to FR-028, NFR-REL-04 | M3 | reference-model edit and undo tests |
| FR-060 to FR-069, NFR-REL-05 to 07 | M4 | state-machine, recovery, conflict, and crash tests |
| FR-030 to FR-036 | M2, M3, and M5 | shell, command, status, and production-editor UI tests |
| FR-080 to FR-086 | M5 | semantic accessibility, IME, display, and platform matrices |
| FR-090 to FR-096 | M7 | clean-machine install, update, rollback, and uninstall evidence |
| Performance requirements | M5 to M7 | reproducible benchmark reports |
| FR-100 to FR-116 | M6 | conformance, source mapping, equivalence, idempotence, synchronization, safety, accessibility, and UI tests |
| Security and release requirements | Every gate, final in M7 | audits, runtime inspection, SBOM, provenance, release checklist |

No requirement becomes Verified without a stable evidence link on the same
commit.
