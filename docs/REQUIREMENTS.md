# Noter Product Requirements

**Version:** 0.3

**Reviewed:** 2026-07-25

**Status:** Ratified product contract for v0.1 and v0.2

This document defines what Noter must do. [DESIGN.md](DESIGN.md) defines how,
[ROADMAP.md](ROADMAP.md) defines when, and dated evidence proves whether a
requirement is complete. When those documents disagree, this contract and the
latest ratified architecture decision record govern.

## 1. Product promise

Noter is a focused, single-document, cross-platform plain-text editor. It opens,
edits, and saves user-selected UTF-8 files without hidden transformation,
network activity, telemetry, accounts, or proprietary document formats.

The v0.1 release earns trust in ordinary text editing. Markdown assistance is a
separate opt-in v0.2 capability and cannot weaken any v0.1 guarantee.

### 1.1 User mental model

The product must preserve these expectations:

1. The document is plain text and the visible source is the saved source.
2. Save writes the current intended revision to the selected file.
3. Save never silently changes encoding, BOM state, existing line endings, or
   Unicode normalization.
4. Undo reverses the last intentional edit transaction.
5. New, Open, Reload, Close, and Quit cannot discard dirty work without an
   explicit Save, Discard, or Cancel decision.
6. Crash recovery is a safety net, not a hidden replacement for Save.
7. The application never connects to a network or reads unrelated documents.

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

## 2. v0.1 functional requirements

### 2.1 Document and file operations

- **FR-010 New:** Create one clean, untitled document per window.
- **FR-011 Open:** Open a user-selected regular file through a system file
  dialog. Refuse a final symlink or Windows reparse point in v0.1; following a
  link requires a future resolved-target identity contract.
- **FR-012 Strict UTF-8:** Accept UTF-8 with or without a UTF-8 BOM. Reject
  invalid UTF-8 without replacement characters. A future explicit import flow
  may create a new untitled converted document, but it must never overwrite the
  source implicitly.
- **FR-013 Save:** Save to the current regular-file path through the durable
  replacement protocol in NFR-REL-02. A multiply hard-linked destination
  requires explicit confirmation that only the selected directory entry will
  advance.
- **FR-014 Save As:** Ask for a destination every time. The document path and
  clean revision change only after the new destination commits successfully.
  Refuse an existing final symlink or unsupported reparse point rather than
  replacing or following it implicitly.
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
- **FR-031 Theme:** Provide System, Light, and Dark. Persist the choice and follow
  system changes when System is selected.
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

## 3. v0.2 Markdown requirements

- **FR-100 Opt in:** Markdown assistance is off by default and adds negligible
  idle work when disabled.
- **FR-101 Visible source:** Inline styling keeps all Markdown punctuation and
  source text visible.
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

## 4. Non-functional requirements

### 4.1 Reliability

- **NFR-REL-01 Byte fidelity:** Loading and saving an unedited supported file
  produces identical bytes.
- **NFR-REL-02 Durable replacement:** Saving writes a unique sibling, writes all
  bytes, flushes, syncs the file, performs an atomic platform replacement, and
  syncs the parent directory where supported. A pre-commit failure leaves the
  original complete and unchanged. Outcomes distinguish Committed, Conflict,
  Not Committed, and Commit State Unknown. A post-commit barrier failure is
  Committed with a durability warning; an uncertain commit retains dirty state
  and recovery until reconciliation.
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
| Idle RSS on reference Windows machine | at most 120 MiB |
| 50 MiB document RSS | at most 350 MiB |
| Stripped Windows release binary | target under 10 MiB, ceiling 18 MiB |

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

- **NFR-SEC-01 Zero network:** The application makes no outgoing connection,
  update check, telemetry submission, remote asset request, or automatic crash
  report.
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

## 6. Explicit non-goals for v0.1 and v0.2

- Tabs, projects, folder trees, and workspaces
- Rich text or a proprietary document format
- Programming-language syntax highlighting
- LSP, Git integration, terminal, plugins, or command palette
- Accounts, synchronization, collaboration, or cloud storage
- AI features
- Built-in network access of any kind
- Non-UTF-8 save encodings
- Arbitrary themes or a font marketplace
- 500 MB editable-file guarantees

## 7. Traceability

The detailed matrix lives beside implementation tests and is expanded per
milestone. The minimum mapping is:

| Contract area | Milestone | Primary evidence |
| --- | --- | --- |
| FR-010 to FR-019, NFR-REL-01 to 03 | M1 | golden bytes, property tests, injected I/O failures |
| FR-020 to FR-028, NFR-REL-04 | M2 | reference-model edit and undo tests |
| FR-060 to FR-069, NFR-REL-05 to 07 | M3 | state-machine, recovery, conflict, and crash tests |
| FR-030 to FR-036, FR-080 to FR-086 | M4 and M5 | semantic UI tests and signed platform matrices |
| Performance requirements | M5 and M6 | reproducible benchmark reports |
| FR-100 to FR-107 | M7 | conformance, equivalence, idempotence, and UI tests |
| Security and release requirements | Every gate, final in M6 | audits, runtime inspection, SBOM, provenance, release checklist |

No requirement becomes Verified without a stable evidence link on the same
commit.
