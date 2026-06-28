# Noter Requirements Specification

**Version:** 0.1 (Planning Baseline)
**Date:** June 2026
**Status:** Frozen for Phase 0-1 implementation. Changes require explicit update to this document + ROADMAP.

This document defines what "extremely well" means for Noter. Everything in DESIGN.md and the implementation must trace back to these requirements.

## 1. Scope

Noter is a single-document-focused, plain-text editor whose primary purpose is to let a user open any UTF-8 text file, edit it comfortably, and save it with extremely high confidence that their data and intent (line endings, encoding, exact bytes where possible) are preserved.

It is deliberately **not** a code editor, note app, or rich text tool.

### 1.1 Mental Model Alignment (Critical for "Not Slopware")

In response to the critical review, every major user-visible operation must preserve or explicitly communicate the user's expected model of a classic Notepad:

**User Mental Model (to be protected):**
- "The file on disk contains exactly the characters I see in the window (modulo the line-ending convention I chose or that was present when I opened the file)."
- "Save means the bytes on disk become what I have in memory right now. No background rewriting, no cloud sync side effects, no 'smart' Unicode normalization I did not request."
- "Undo undoes my last intentional change. Consecutive typing is one change."
- "If I force-quit or the power fails, I will be offered my recent work back (within ~30 seconds) and nothing will have been silently lost or altered."

**Mental Model Impact Statement requirement:** Before any feature is declared complete in a phase, its implementing engineer must write a one-paragraph statement answering: "How does this feature either reinforce the above model or clearly signal where it deviates, and what UI/affordance makes the deviation visible to the user?"

This is non-negotiable for maintaining the restorative design goal stated in the original pain points.

### 1.2 Verification Criteria & Traceability

Each FR and NFR below is accompanied (or will be in later revisions) by:
- A direct reference to one or more Safety/Liveness properties from DESIGN.md section 3.5.
- The primary verification method (property test name, golden file, FMEA row, manual matrix item, fault-injection case).
- The mental model impact statement (or pointer to it).

This creates a lightweight traceability matrix so that when a test fails or a user reports a surprise, we can immediately point to the originating requirement and the verification artifact.

## 2. Functional Requirements

### 2.1 Core File Operations (MUST)

- **FR-010** Open an existing file via native file dialog (rfd).
- **FR-011** Create a new untitled document (Ctrl+N or menu).
- **FR-012** Save current document (Ctrl+S). Must be atomic (see NFR-Reliability).
- **FR-013** Save As... (Ctrl+Shift+S) - always shows dialog.
- **FR-014** Recent files list (max 10-12 entries, persisted, clickable, missing files are gracefully removed).
- **FR-015** On open, detect and preserve the file's line ending convention (CRLF, LF, or CR). New files on Windows default to CRLF; elsewhere LF. User can change via status bar or Format menu (rarely needed).
- **FR-016** Graceful handling of UTF-8 (with or without BOM). On save, reproduce the BOM state that was present on load. Non-UTF-8 files: offer "Open with lossy UTF-8 conversion" or "Cancel". Never silently corrupt.

### 2.2 Editing (MUST)

- **FR-020** Full keyboard text input, cursor movement (arrows, Home/End, Page Up/Down), selection (Shift+arrows, Ctrl+Shift+arrows, mouse).
- **FR-021** Cut / Copy / Paste / Delete using both menu and standard platform shortcuts (Ctrl/Cmd + X/C/V, Delete, Backspace).
- **FR-022** Undo and Redo with high-quality coalescing:
  - Consecutive character insertions are one undo step.
  - Consecutive deletions (backspace or delete) are one step.
  - Paste or large operations are their own step.
  - Undo stack must be bounded (e.g. 500-1000 entries or ~50 MiB of retained rope history).
- **FR-023** Find (Ctrl+F): opens a find bar (not modal dialog). Supports case-sensitive toggle. "Find Next" (F3) and "Find Previous".
- **FR-024** Replace (Ctrl+H): basic string replace with "Replace", "Replace All". Must respect current selection scope when possible.
- **FR-025** Word wrap toggle (Format > Word Wrap or Alt+Z). Persisted per session or globally (TBD in design, but default on).
- **FR-026** Go To Line (Ctrl+G) - must be fast and accurate even on very large files.

### 2.3 View & Status (MUST)

- **FR-030** Always-visible status bar showing at minimum:
  - Current line and column (1-based, logical)
  - Selection length (chars or "1 char selected")
  - Total characters in document
  - Encoding (always UTF-8 for v1)
  - Line ending mode (CRLF / LF / CR)
  - Modified indicator (`*`)
- **FR-031** Optional line numbers (View > Line Numbers). Off by default to stay true to classic Notepad spirit.
- **FR-032** Zoom font size via Ctrl + mouse wheel and View menu (no font family picker in v1 to reduce scope; we use a good default monospace + user size).

### 2.4 Theme (MUST)

- **FR-040** On launch, detect the operating system's current light/dark preference.
- **FR-041** Provide three explicit choices: "System", "Light", "Dark". Choice is persisted.
- **FR-042** On Windows, the app should react to live system theme changes (WM_SETTINGCHANGE) when "System" is selected.
- **FR-043** All UI elements (text, chrome, scrollbars, selection highlight) must have excellent contrast in both themes.

### 2.5 Markdown Preview (SHOULD - Phase 3 QOL)

- **FR-050** View > Markdown Preview (toggle) opens a read-only pane (right side preferred, or bottom on narrow windows).
- **FR-051** The preview must render common Markdown (headings, lists, code blocks, emphasis, links, blockquotes, horizontal rules) using only pure Rust crates + egui drawing primitives. It must never interpret or execute scripts, images from network, or HTML.
- **FR-052** Toggling preview or editing in the left pane must **never** mutate the underlying document bytes or line ending state. Preview is strictly a derived view.
- **FR-053** Preview updates live as you type (debounced ~150-250 ms).
- **FR-054** Large documents: preview may fall back to "Rendering limited to first N lines for performance" with a clear affordance to render more.

### 2.6 Reliability & Data Safety (MUST - see also NFR)

- **FR-060** On every significant edit burst or timer (configurable, default 25s), write an autosave file to the OS temp directory.
- **FR-061** On clean launch with no command-line file, scan for stale autosave files belonging to this user. Offer "Recover unsaved changes from [timestamp]" in a non-modal but prominent way.
- **FR-062** On normal exit (after successful save or explicit discard), best-effort cleanup of autosave files.
- **FR-063** If the file on disk has changed since Noter loaded it (mtime + size or content hash), show a clear "File changed on disk" prompt with options: Reload (discard my changes), Keep mine, or Diff (future).
- **FR-064** Save operations must be atomic: write to a sibling `.tmp` file in the same directory, fsync, then rename. This must survive process kill, power loss simulation, etc. in testing.

### 2.7 Window & Session (MUST)

- **FR-070** Remember last window position, size, and maximized state. Restore on next launch (with sanity checks so windows don't appear off-screen).
- **FR-071** Multiple instances are allowed and encouraged (classic Notepad behavior). No forced single-instance.

### 2.8 Keyboard & Shortcuts (MUST)

All primary actions must be reachable without a mouse. Standard platform shortcuts must work. A Help > Keyboard Shortcuts view (simple list or table) is required.

## 3. Non-Functional Requirements

### 3.1 Reliability (Highest Priority)

- **NFR-REL-01** Under normal operation (including force-kill of the process at any moment after an edit), the user must never lose more than the last ~30 seconds of typing when recovery is offered.
  **Verification:** Fault-injection harness (corrupt/kill during autosave + restart) run at least 30 times per phase gate. Cross-references Safety Property S4 and FMEA F3.
- **NFR-REL-02** Save must never leave a zero-byte or truncated file on disk when the original existed. Atomic rename is mandatory.
  **Verification:** Property test exercising S1 under normal, full-disk, and simulated-rename-failure conditions. Golden files + external `cmp` / `xxd` checks. FMEA F1.
- **NFR-REL-03** Line ending and BOM fidelity tests must pass for CRLF, LF, and mixed files. Round-trip byte equality for the text content (modulo the intentional normalization we document).
  **Verification:** Exhaustive golden-file matrix (all three endings x BOM x empty / single-line / multi-line / trailing-newline cases). Property test for S2. Mental model impact statement required.
- **NFR-REL-04** Undo/Redo must be information-theoretically lossless for the operations it claims to support. Property tests must prove that applying a sequence of edits + undos + redos returns to an identical rope state.
  **Verification:** `proptest` generator for arbitrary (but bounded) sequences of insert/delete/replace commands; assert U1 / S3 after each undo/redo cycle. Coalescing rules are part of the test specification.

### 3.2 Performance

- **NFR-PERF-01** Open a 50 MiB text file and display the first page in < 2.5 seconds on mid-range 2024-2026 hardware (8-16 GB RAM, modern SSD).
- **NFR-PERF-02** Smooth 60 fps scrolling and cursor movement on a 200,000 line file once the document is loaded (measured via manual + automated frame time logging).
- **NFR-PERF-03** Find operation on 50 MiB file completes in < 800 ms (first match).
- **NFR-PERF-04** Memory baseline (empty document + idle): < 120 MiB RSS on Windows, < 150 MiB on macOS/Linux. 50 MiB document loaded should stay under ~350-400 MiB.
- **NFR-PERF-05** Binary size (release, stripped, LTO): target < 10 MiB on Windows, < 12 MiB on macOS/Linux. Hard ceiling for v0.1: 18 MiB.

### 3.3 Cross-Platform Behavior

- **NFR-XPLAT-01** Identical feature set and (as much as possible) identical keyboard shortcuts across the three platforms, with only the expected Cmd vs Ctrl and menu bar location differences (macOS menu bar at top of screen).
- **NFR-XPLAT-02** File dialogs must be the real native ones on each platform.
- **NFR-XPLAT-03** Theme detection and "System" following must work on all three.
- **NFR-XPLAT-04** Line ending defaults must feel native (CRLF on Windows new files, LF elsewhere).

### 3.4 Code Quality & Maintainability (Non-negotiable)

- **NFR-QUAL-01** Rust edition 2024. `rust-version` in Cargo.toml set to the minimum we actually test (initially 1.85+ or 1.90+).
- **NFR-QUAL-02** `cargo fmt -- --check` and `cargo clippy --all-targets -- -D warnings` must be clean on every commit that lands in `main`. CI enforces this.
- **NFR-QUAL-03** Core modules (`document`, `editor`, `io`, `config`, `theme`) must achieve >= 85% line coverage and >= 70% branch coverage (measured by `cargo llvm-cov` or equivalent) before Phase 2 completion.
  **Additional rigor:** Coverage must include the executable forms of S1-S4 and U1 (see DESIGN section 3.5). Mutation testing (via `cargo-mutants` or equivalent) on the core rope/document path is a stretch goal for Phase 2+.
- **NFR-QUAL-04** All critical invariants have property-based tests (`proptest` or `quickcheck`): undo/redo roundtrips, line-ending detection + save fidelity, atomic save success under simulated failure (via tempfs tricks or post-write corruption tests), config serialization roundtrips.
  **Traceability:** Every property test must be annotated with the Safety/Liveness property ID it is attempting to falsify.
- **NFR-QUAL-05** No `unwrap()`, `expect("...")` is allowed in hot user paths except where the comment explains why the invariant is truly impossible to violate. Prefer `?` + typed errors + user-facing messages.
  **Verification:** `grep` + manual review at each phase gate; `cargo clippy` deny of `unwrap_used` in `src/core` (gradually enforced).
- **NFR-QUAL-06** Every non-trivial public or `pub(crate)` function must have a doc comment explaining intent, edge cases, and performance characteristics.
  **Additional:** Doc comments for core operations must reference the relevant safety property or FMEA row they participate in.
- **NFR-QUAL-07** We will maintain a small but growing set of golden-file tests for save/load behavior with tricky inputs (BOM + CRLF, very long lines, unicode, empty files, files with only newlines).
  **Rigor:** Golden files are the primary executable evidence for S2 and are re-run as part of the "reproducibility envelope" in STEWARDSHIP.md.

### 3.5 Security & Privacy

- **NFR-SEC-01** The application must make **zero** network requests at any time, including update checks, telemetry, font loading, image loading in preview, etc. This is auditable via `cargo tree` + runtime traffic inspection in CI/tests.
- **NFR-SEC-02** Dependency tree must remain small. Any new dependency > 5 transitive crates or with "crypto", "network", "async" in its tree requires explicit justification in DESIGN.md and ROADMAP sign-off.
- **NFR-SEC-03** We only ever read files the user explicitly chose via Open or drag-and-drop. We never scan directories, never follow symlinks outside user intent, never write anywhere except the save location the user chose + the OS temp dir for autosave.
- **NFR-SEC-04** Reproducible builds are a long-term goal. We will track the `cargo auditable` / `cargo-cyclonedx` story and document SBOM generation for releases.

### 3.6 Accessibility & Usability

- **NFR-A11Y-01** All menu items, find bar, and status information must be keyboard reachable.
- **NFR-A11Y-02** High contrast in both themes. Text selection must be clearly visible.
- **NFR-A11Y-03** Status bar text must be readable at 125% and 150% scaling on Windows.
- **NFR-A11Y-04** The editor must remain usable with only a keyboard (no mouse required for 95% of workflows).

### 3.7 Distribution & Packaging (v0.1 target)

- Portable single-file executable must work when copied to any machine (no installer required).
- Optional proper installers via `cargo-dist` (`.msi` on Windows, `.dmg`/app bundle on macOS, `.deb`/`.tar.gz` on Linux).
- Clear "unstable / early" messaging until v0.5 or v1.0.

## 4. Constraints & Assumptions

- We will use **egui 0.30+ / eframe 0.30+** (or the current stable at implementation time) as the GUI layer. This decision is recorded in DESIGN.md.
- All rendering and logic must be possible without a webview for the core experience.
- The project is developed primarily on Windows but must compile and run cleanly on the other two platforms via CI from day one of Phase 1.
- We will not take on `async` runtimes or tokio unless a very specific need appears (file watcher is a possible exception; we prefer simple polling or the `notify` crate's synchronous mode first).

## 5. Success Metrics for v0.1

1. A person who has used classic Notepad for 10+ years can perform their daily workflow without reading docs.
2. In a simulated crash test (kill -9 during heavy typing + repeated 20 times), recovery always offers the document and the recovered version contains all but the last <30s of work.
3. Binary + RAM numbers meet the NFR-PERF targets on the reference machines.
4. `cargo clippy -- -D warnings` and fmt clean; core coverage >= 85%.
5. The author (and at least one other tester on macOS or Linux) uses it as their daily driver for plain text notes and config files for two consecutive weeks without data loss or major annoyance.

## 6. Out of Scope for v0.1 (and probably v0.2)

- Multiple tabs or a "project" concept
- Plugin system or extension
- Git integration or "changes" gutter
- Image embedding or rich paste
- Collaborative editing
- Any form of account, sync, or "sign in"
- Heavy customization of the editing surface (vim mode, etc.)
- Built-in terminal or command palette beyond find/goto

---

This requirements document is the source of truth. If implementation or DESIGN.md diverges, this document must be updated first and the divergence justified.
