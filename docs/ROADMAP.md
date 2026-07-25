# Noter Roadmap

**Last reviewed:** 2026-07-25

**Current milestone:** M1, Document and Durable I/O Trust Kernel

**Release target:** a trustworthy single-document v0.1 before Markdown v0.2

This roadmap is an execution contract. A milestone is complete only when its
artifacts and measurements exist in the repository. Feature presence alone is
not completion.

The evidence behind the decisions below is in [RESEARCH.md](RESEARCH.md).

## Active objective

Build Noter into an exceptional, focused, cross-platform plain-text editor whose
reliability claims are proven, whose latency is measured, whose interface is
accessible and international-text safe, and whose scope remains deliberately
small.

## What exceptional means

1. **Trust:** No silent conversion, truncation, overwrite, discard, or recovery
   failure. Save and recovery behavior are explainable in one paragraph.
2. **Responsiveness:** Typing, navigation, search, and scrolling stay within
   measured latency budgets on a published corpus.
3. **Craft:** Every visible command works, shortcuts are consistent, states are
   quiet but legible, and error paths are designed rather than improvised.
4. **Reach:** Keyboard-only use, screen readers, CJK IME, dead keys, Unicode,
   high DPI, Windows, macOS, X11, and Wayland are release concerns.
5. **Restraint:** One document per window, plain text on disk, no account, no
   sync, no telemetry, no network, no plugins, and no AI features.
6. **Evidence:** Green CI, enforced coverage, property tests, fault injection,
   reproducible benchmarks, signed manual matrices, and dogfooding records.

## Honest current state

Noter is an early GUI prototype with a partial trust core. It can type text and
perform basic Open, Save, and Save As operations. It is not a completed Phase 1
editor.

Missing or unproven capabilities include dirty-state guards, real undo policy,
find and replace, configuration, recent files, recovery, external-change
detection, window persistence, complete themes, cross-platform shortcuts,
automated UI tests, custom-editor accessibility, performance evidence,
packaging, and release sign-off.

The initial 2026-07-25 baseline found 28.24 percent whole-program line coverage,
85.96 percent line coverage in `core/document.rs`, only three tests, and no
evidence for most named safety properties. M0 closed with seven tests, 31.11
percent whole-workspace line coverage, 96.43 percent testable-core line
coverage, and exact-commit CI on Windows, macOS, and Linux. Those numbers are a
foundation, not evidence that the editor is ready for daily use.

## Product contract for v0.1

Noter v0.1 is one plain-text document per window. It supports strict UTF-8 with
an optional UTF-8 BOM. Invalid UTF-8 never becomes replacement characters
without an explicit conversion flow. Uniform LF, CRLF, and CR files round-trip.
Mixed-EOL behavior must be explicit and tested before release.

Save updates the chosen file. Recovery stores a private local recovery record
and never silently writes the original file. New, Open, Close, Quit, and external
reload all use one tested dirty-document decision state machine.

The UI is rendered consistently by egui and integrates with each system where it
matters: native file dialogs, expected modifier keys, theme preference, recent
files, window behavior, IME, clipboard, accessibility APIs, and packaging. It
does not claim native widget appearance.

Markdown is not in v0.1. It follows as an opt-in v0.2 capability after the editor
has earned trust in daily use.

## Gates that apply to every milestone

- `cargo fmt --all -- --check` passes.
- `cargo clippy --locked --all-targets --all-features -- -D warnings` passes on the pinned
  verified toolchain.
- `cargo test --locked --all-targets --all-features` passes on Windows, macOS,
  and Linux.
- Testable core code maintains at least 80 percent line coverage during
  development. The v0.1 release requires at least 80 percent whole-workspace line
  coverage and at least 90 percent trust-kernel line coverage.
- No coverage exclusion is accepted without a nearby rationale and a replacement
  verification method.
- New trust behavior has failure-path tests. Critical invariants have property or
  model-based tests, not only examples.
- No ignored, flaky, or platform-skipped critical test is counted as passing.
- The dependency health record and FMEA are updated when the change affects them.
- Documentation describes shipped behavior, not intended behavior.
- CI is green on the exact commit being advanced.

## Execution sequence

| Milestone | Outcome | Depends on | Target |
| --- | --- | --- | --- |
| M0 | Truthful, reproducible, green foundation | None | Verified |
| M1 | Proven document and durable I/O trust kernel | M0 | In progress |
| M2 | Tested edit, selection, and undo model | M1 | Planned |
| M3 | Recovery, dirty lifecycle, and conflict safety | M1, M2 | Planned |
| M4 | Complete classic-notepad alpha on `TextEdit` | M1, M2, M3 | Planned |
| M5 | Custom editor feasibility gate and production engine | M4 | Planned |
| M6 | Cross-platform v0.1 release | M5 | Planned |
| M7 | Opt-in Markdown assist v0.2 | M6 | Planned |

Effort ranges below are engineering estimates, not ship dates. Dogfooding and
external sign-off are elapsed-time gates and cannot be compressed by coding
faster.

## M0: Truthful and Green Foundation

**Status:** Verified on 2026-07-25 at commit `7512534`.

**Outcome:** Anyone can check out one coherent branch, run one documented command
set, and see exactly what is implemented and what remains.

**Work:**

- Reconcile the local five-ahead, one-behind branch with `origin/master` without
  losing the current UI or document work.
- Correct README and roadmap status. Link the research record.
- Resolve the root-to-`docs/` migration in CI and all internal links.
- Pin the verified Rust toolchain. Add an explicit MSRV policy and test it if an
  MSRV is advertised.
- Make format, strict Clippy, tests, and an enforced core coverage threshold pass.
- Separate library-testable product code from the binary bootstrap.
- Remove or defer unused dependencies. Record duplicate transitive dependencies
  instead of treating all platform-driven duplicates as Clippy errors.
- Add a fast local `just`, `xtask`, or documented Cargo command sequence only if
  it reduces real friction without adding a large dependency.
- Add documentation link checking and reject placeholder repository metadata.
- Establish `Planned`, `In progress`, `Verified`, and `Deferred` status language.

**Checkpoint on 2026-07-25:**

- **Verified locally:** README and roadmap status, research record, license and
  repository metadata, pinned Rust 1.97.1 toolchain, pinned CI actions, strict
  UTF-8 rejection, formatting, strict Clippy, seven unit tests, enforced 96.43
  percent core line coverage, documentation link checking, and the measured
  [M0 baseline](BASELINE.md).
- **Verified locally:** unused planned dependencies are deferred until their
  milestones. The current graph is four runtime and one development direct
  dependency, 325 resolved cross-target packages, and a 4.53 MiB Windows release
  binary.
- **Verified locally:** the binary-to-library boundary keeps testable product
  logic independent of the GUI shell.
- **Verified locally:** requirements, technical design, UX, privacy, manual
  verification, and ADR-001 through ADR-003 are reconciled with the research and
  roadmap. ADR-003 remains Proposed pending its named platform evidence.
- **Verified locally:** the divergent remote honesty commit is integrated on the
  `m0-foundation` branch. The original local `master` pointer remains as a
  recovery point.
- **Verified:** commit `7512534` passed the complete warning-free Windows,
  macOS, and Linux matrix in
  [GitHub Actions run 30176526028](https://github.com/blisspixel/noter/actions/runs/30176526028).
- **Verified:** the exact evidence commit is pushed to `m0-foundation`; the
  original local `master` pointer remains available as a recovery point.

**Exit evidence:**

- Clean working tree after an intentional commit series.
- Local quality suite green on the pinned toolchain.
- GitHub CI green on Windows, macOS, and Linux for the reconciled branch.
- Enforced core line coverage at or above 80 percent.
- README makes no unsupported Phase 1 claim.
- Baseline report records test count, coverage, binary size, dependency count,
  and current platform verification.

**Estimated effort:** 2 to 4 focused days.

## M1: Document and Durable I/O Trust Kernel

**Outcome:** Loading and saving are boring, strict, fault-tested, and independent
of the GUI.

**Checkpoint on 2026-07-25:**

- **Verified locally:** newline-free, uniform, and mixed profiles count LF,
  CRLF, and CR without changing authoritative text. Dominant ties use first
  occurrence, and mixed insertion prefers preceding, following, then fallback.
- **Verified locally:** the insertion API rejects out-of-range positions and
  positions that split an existing CRLF sequence.
- **Verified locally:** 19 golden byte cases cover BOM, all EOL forms, Unicode,
  embedded BOM, NUL, and invalid UTF-8. Three properties each run 512 generated
  cases for strict byte round-trip, exact classification, and insertion policy.
- **Verified in CI:** line-ending evidence commit `62dc49f` passed Windows,
  macOS, Linux, strict lint, documentation, and the 90 percent coverage gate in
  [GitHub Actions run 30177403255](https://github.com/blisspixel/noter/actions/runs/30177403255).
- **Verified locally:** explicit `Encoding`, `Bom`, and checked `Revision` values
  replace ambiguous primitive metadata.
- **Verified locally:** the injected save protocol distinguishes Committed,
  Conflict, Not Committed, and Commit State Unknown. Its fault matrix proves
  original-byte preservation through every modeled pre-commit failure, two
  conflict windows, partial temporary writes, and cleanup failure. A failed
  post-commit parent barrier is reported as committed with a warning.
- **Verified in CI:** save-protocol evidence commit `0edc342` passed Windows,
  macOS, Linux, strict lint, documentation, and the 90 percent coverage gate in
  [GitHub Actions run 30177953025](https://github.com/blisspixel/noter/actions/runs/30177953025).
- **Verified locally:** `ContentFingerprint` computes BLAKE3-256 from slices or
  streams, matches the official zero-byte and one-byte vectors, and propagates
  incomplete reads instead of accepting a partial digest.
- **Verified in CI:** digest evidence commit `613cbcd` passed Windows, macOS,
  Linux, strict lint, documentation, and the 90 percent coverage gate in
  [GitHub Actions run 30178217482](https://github.com/blisspixel/noter/actions/runs/30178217482).
- **Verified locally:** stable file observations hash an open handle, compare
  identity, length, hard-link count, and modification time around the read, then
  reopen the pathname to close the ordinary replacement race. Missing files,
  directories, final links or reparse points, hard links, same-content distinct
  files, and path-redacted failures have explicit tests.
- **Verified locally:** the main crate still forbids unsafe code. Two Windows
  by-handle queries are isolated behind a safe internal crate and documented
  safety contracts. Preferred 128-bit IDs and labeled reduced fallbacks have
  deterministic tests.
- **Verified locally:** all 47 Windows-local tests and strict workspace Clippy
  pass. Workspace trust-kernel line coverage is 96.20 percent; CI enforces the
  M1 floor of 90 percent.
- **Measured:** the property harness adds eight test-only lock entries and the
  digest adds four runtime lock entries, bringing the cross-target graph to
  337 packages. The internal platform workspace member brings the graph to
  338 without adding an external package. The currently dead-stripped path leaves the release
  binary at 4.53 MiB and 4,749,312 bytes; the adapter integration will be
  measured again.
- **Next:** implement unpredictable exclusive sibling creation, metadata
  transfer, synchronization, cleanup, and commit operations behind the accepted
  ADR-003 `Storage` boundary.

**Work:**

- Define `DocumentId`, `Revision`, `Encoding`, `Bom`, `LineEndingPolicy`,
  `FileIdentity`, and saved-content fingerprint types.
- Decide and document mixed-EOL editing behavior. Untouched bytes must always
  round-trip. Edited mixed-EOL files must never normalize silently.
- Stream strict UTF-8 loading into the authoritative buffer. Offer conversion only
  through a separate explicit command and Save As path.
- Replace the interim save code with an audited I/O adapter that uses a unique
  sibling, flush, file sync, atomic replacement, and parent-directory sync where
  supported.
- Define and test destination metadata, symlink, read-only, ACL, cloud-folder,
  network-filesystem, and external-writer behavior.
- Make config and recent-file state use the same durable-write discipline.
- Introduce injectable I/O operations for failures at create, write, flush, sync,
  replace, metadata, and directory-sync stages.
- Add golden fixtures for empty, BOM, LF, CRLF, CR, mixed EOL, trailing newline,
  newline-only, emoji, CJK, combining marks, RTL, long line, and invalid UTF-8.
- Add property tests for byte round-trip and line-ending detection.

**Exit evidence:**

- Safety properties S1 and S2 have executable tests and traceability entries.
- On every injected pre-commit failure, the original is complete and unchanged.
- On success, the destination equals the exact intended bytes.
- At least 90 percent line coverage for document and I/O modules.
- Mutation testing of the serialization and replace-decision paths finds no
  surviving high-impact mutant, or each survivor is explained.
- Windows metadata and replacement behavior is manually verified on NTFS.

**Estimated effort:** 1 to 2 weeks.

## M2: Edit, Selection, and Undo Model

**Outcome:** One authoritative buffer and a UI-independent command model define
all text changes.

**Work:**

- Remove the full-document `String` and `Rope` dual authority.
- Define byte, Unicode scalar, grapheme, line, logical column, and visual column
  boundaries explicitly.
- Implement cursor, anchor/head selection, preferred column, viewport anchor, and
  edit transactions.
- Implement insert, delete, replace, paste, newline, indent, and EOL-conversion
  commands.
- Implement bounded undo and redo with documented coalescing rules and exact
  cursor and selection restoration.
- Ensure a formatter, Replace All, or EOL conversion becomes one undoable
  transaction.
- Add a deterministic reference model and property-test arbitrary edit sequences.
- Test emoji sequences, combining marks, surrogate-producing clipboard input,
  CRLF boundaries, empty last lines, and very long lines.

**Exit evidence:**

- Safety property S3 and undo invariant U1 pass model-based tests.
- Undo memory remains within its configured byte budget under stress.
- No edit command can create invalid UTF-8 or split CRLF as two logical breaks.
- Trust-kernel coverage remains at least 90 percent.

**Estimated effort:** 1 to 2 weeks.

## M3: Recovery, Dirty Lifecycle, and Conflict Safety

**Outcome:** No destructive lifecycle action can silently discard work, and a
process crash leaves a validated recovery offer.

**Work:**

- Implement one state machine for New, Open, Reload, Close, Quit, Save, Save As,
  Discard, Cancel, and failed recovery persistence.
- Store versioned recovery records in per-user application state, not the general
  temp directory.
- Add random document and instance IDs, checksums, schema versioning, timestamps,
  original file identity, revision, cursor, and selection.
- Coalesce edits in memory and flush recovery on a background worker without
  blocking the UI. Bound and measure the recovery point objective.
- Sync recovery before any no-prompt close. If that fails, keep the window open
  and show the classic dirty prompt.
- Detect external file changes with file identity plus a content fingerprint.
  Define reload, keep mine, and Save As behavior without a fake diff feature.
- Handle stale records, version mismatch, corruption, two Noter instances, PID
  reuse, clock changes, missing source files, and cleanup failure.
- Build a child-process crash harness that kills at controlled edit and save
  points, restarts scanning, and validates recovered content.

**Exit evidence:**

- Safety property S4 and recovery liveness property L2 have executable tests.
- At least 100 automated process-kill cycles have no missing recovery offer and
  meet the documented recovery point objective.
- Dirty actions cannot reach a drop state without Save, explicit Discard, or
  validated recovery persistence under the selected close policy.
- Recovery records are private to the user where platform permissions allow.

**Estimated effort:** 1 to 2 weeks.

## M4: Complete Classic-Notepad Alpha

**Outcome:** The existing egui `TextEdit` path becomes a complete, coherent alpha
for ordinary files and validates the product workflow before renderer risk.

**Work:**

- Route menus and shortcuts through one `Command` dispatcher.
- Implement New, Open, Save, Save As, recent files, and the M3 dirty state machine.
- Connect built-in editing commands correctly. Hide or disable every command that
  is not implemented.
- Implement Find, next, previous, Replace, Replace All, and Go To Line.
- Implement word wrap, zoom, line/column/selection/character status, encoding,
  EOL, BOM, dirty state, and external-change state.
- Implement System, Light, and Dark preferences with tested contrast.
- Persist window bounds safely and clamp restored windows to available monitors.
- Use platform-primary modifiers and accurate displayed shortcut labels.
- Add recovery and error surfaces that preserve keyboard focus.
- Add `egui_kittest` semantic tests and limited visual snapshots.
- Test keyboard-only navigation, focus order, accessible names, high contrast,
  scale factors, and screen-reader announcements.

**Exit evidence:**

- No visible placeholder command remains.
- Whole-workspace line coverage reaches at least 80 percent through core, command,
  state, and semantic UI tests.
- Windows manual matrix passes, including NVDA and at least one real CJK IME.
- Ten consecutive daily-driver sessions complete without data loss or a workflow
  blocker.
- Alpha limitations clearly state the file-size range supported by `TextEdit`.

**Estimated effort:** 2 to 3 weeks plus dogfooding.

## M5: Custom Editor Gate and Production Engine

**Outcome:** Noter either proves a custom egui editor can meet the full contract or
changes architecture before sinking months into the wrong widget.

### M5A: one-week feasibility gate

Prototype only the risky vertical slice:

- authoritative rope-backed edits with no full copy;
- visible-line layout and bounded galley cache;
- caret, selection, hit testing, vertical movement, horizontal scrolling, and a
  pathological long line;
- IME pre-edit rendering and candidate-window placement;
- AccessKit text runs, selection, editable actions, and screen-reader navigation;
- find highlights and a styled source span;
- deterministic frame-time instrumentation.

**Go criteria:** The slice meets correctness tests and the 1 MiB interaction
budgets, shows a credible route to 50 MiB, and passes a real IME plus at least one
screen reader. Otherwise retain `TextEdit`, reduce the large-file promise, or
evaluate another GUI/text-stack architecture explicitly.

### M5B: production engine, only after Go

- Complete mouse, keyboard, selection, clipboard, drag, word, paragraph, page,
  Home/End, platform shortcut, and focus behavior.
- Virtualize only visible logical and wrapped rows with overscan.
- Bound all caches and undo memory. Degrade expensive highlights explicitly.
- Keep disk I/O and indexing off the render thread with cancellation and stale
  revision rejection.
- Add benchmark corpora for normal prose, source text, logs, mixed Unicode,
  newline-only files, one huge line, and adversarial search patterns.
- Publish p50, p95, and p99 results on named reference hardware.

**Exit evidence:**

- Feature parity with M4, including IME and accessibility.
- Required budgets in [RESEARCH.md](RESEARCH.md) pass in release builds.
- No unbounded per-frame work based on total document length.
- Windows and Linux manual matrices pass. macOS core shortcut and IME matrix also
  passes before M6.

**Estimated effort:** 1 week for M5A, then 3 to 6 weeks for M5B.

## M6: Cross-Platform v0.1 Release

**Outcome:** A boring, trustworthy plain-text editor is professionally packaged,
auditable, and ready for real users.

**Work:**

- Generate portable archives and appropriate installers for Windows, macOS, and
  Linux with current cargo-dist tooling.
- Reverify and update pinned CI action SHAs and minimum workflow permissions.
- Test the verified Rust toolchain and the advertised MSRV policy.
- Run dependency license, advisory, duplicate, and source audits.
- Generate SBOM, build provenance, SHA-256 checksums, and signatures where
  credentials are available.
- Publish binary size, RSS, startup, file-open, search, scroll, recovery, and
  crash-harness results.
- Complete Usage, Recovery, Privacy, Security, Troubleshooting, Contributing,
  Stewardship, and release-checklist documents.
- Run the complete manual matrix on Windows, macOS, X11, and Wayland.

**Exit evidence:**

- Exact release commit has green CI and at least 80 percent whole-workspace line
  coverage.
- Two people use the release candidate for at least 14 days each with no data
  loss incident.
- One tester is not the primary developer and one test period is on a non-Windows
  platform.
- All critical and high FMEA risks have tests or a clearly accepted residual risk.
- Install, upgrade, portable use, and uninstall are verified from clean machines.

**Estimated effort:** 2 to 3 weeks plus a minimum 14-day release-candidate soak.

## M7: Markdown Assist v0.2

**Outcome:** Noter offers an opt-in "Ruff for Markdown" without hiding source or
silently rewriting content.

**Work:**

- Ratify one dialect and extension policy, initially CommonMark with a small,
  explicit GFM subset if justified.
- Apply inline styles while keeping Markdown punctuation visible.
- Add non-mutating diagnostics with rule IDs, ranges, explanations, and explicit
  fixes.
- Add smart list continuation as a separate, toggleable edit command.
- Implement Format as an explicit previewed diff. Verify parsed-document
  equivalence, apply one undo transaction, and preserve EOL/BOM.
- Treat heading normalization, ordered-list renumbering, table alignment, and
  trailing whitespace as independent rules with fixtures.
- Parse by revision off the UI thread, cancel stale work, and bound large-file
  processing to visible or changed regions where correctness permits.
- Never load remote images, fetch links, execute HTML, or make a network request.

**Exit evidence:**

- CommonMark conformance corpus for supported behavior is green.
- Formatter fixtures are idempotent and AST-equivalent.
- Every automatic-looking edit is explicit, configurable where appropriate, and
  one-step undoable.
- Markdown off means zero document mutation and negligible idle cost.
- No regression in any v0.1 trust, accessibility, performance, or size gate.

**Estimated effort:** 3 to 5 weeks.

## Deferred until evidence demands it

- 500 MB editable files or files larger than RAM.
- Tabs, workspaces, folder trees, or projects.
- Syntax highlighting for programming languages.
- Plugins, LSP, Git, terminal, command palette, or collaborative editing.
- Non-UTF-8 save encodings. Explicit import conversion may be considered first.
- Embedded web content, remote images, accounts, cloud sync, update checks, or
  telemetry.
- Themes beyond System, Light, and Dark.

## Next executable backlog

These are the next tasks in dependency order:

1. Implement unique sibling creation, metadata transfer, synchronization, and
   cleanup behind the accepted `Storage` boundary.
2. Implement and reconcile Windows existing-file and new-file commit paths.
3. Implement Linux and macOS existing-file and no-overwrite commit paths.
4. Add platform fixtures for metadata, symlinks, hard links, read-only files,
   external writers, and weaker filesystems.
5. Add mutation testing for serialization, conflict, commit-state, and cleanup
   decisions.
6. Integrate revision-tagged snapshots and outcomes into the document model.
7. Add the benchmark corpus generator and automate the trust-kernel baseline.
8. Implement the edit transaction and selection model.
9. Add reference-model undo and redo property tests.
10. Implement the dirty-document lifecycle state machine.
11. Implement versioned state-directory recovery records and crash scanning.
12. Build the controlled child-process crash harness.
13. Create the pure command and application-state reducer, then connect the
    proven core to the complete M4 UI.

The answer to "what is next" is therefore unambiguous: finish M1 by making the
ratified replacement protocol real on Windows, Linux, and macOS and prove it
against metadata, race, durability, and crash fixtures.
Do not begin the custom editor or Markdown engine while save, undo, close, and
recovery semantics are still aspirational.
