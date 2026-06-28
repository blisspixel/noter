# Noter Roadmap

**How we will build an exceptionally high-quality, reliable, pure cross-platform notepad in 2026 - with code quality and test coverage designed in, not added later.**

## Vision

By the end of Phase 4 we will have a tool that a long-time classic Notepad user can switch to without friction, that they trust with important text after weeks of daily use, and that feels like a natural, boring, reliable part of their computing environment on Windows, macOS, and Linux.

We will achieve this by refusing to ship until each phase's quality gates are passed.

## Guiding Principles

1. **Reliability > Features.** A missing "nice to have" is acceptable. Losing data or corrupting line endings is not.
2. **Purity has a cost and we pay it.** Markdown preview is a view only. Core editing and saving is always plain text with maximum fidelity.
3. **Tests are part of the product.** Core logic is not "done" until property tests and golden tests demonstrate the invariants.
4. **Small is a feature.** Every new dependency and every new UI surface must justify itself against binary size, complexity, and attack surface.
5. **Cross-platform from day one.** We do not accept "it only works well on Windows."

## Phase Structure & Quality Gates

Each phase has:
- Clear goals
- Deliverables
- **Quality Gate** (these are mandatory before moving on). Gates now incorporate the rigor items from RIGOROUS_REVIEW.md (FMEA updates, mental model statements, dependency health tables, re-reading of the critical review, executable safety properties, etc.).
- Estimated effort (solo developer, calendar time, very rough)

The entire roadmap is a direct response to the original user pain points (hate of bloat, telemetry, and unpredictability in modern Notepad) and the internal critical review aimed at preventing slopware. We are building a restorative, minimalist, high-integrity tool, not just another editor.

---

### Phase 0 - Foundation & Planning (Current - Target: Complete within days of starting)

**Goals**
- All major planning artifacts exist and are high quality.
- Project is correctly initialized (edition 2024, good .gitignore, git, basic CI).
- Skeleton compiles cleanly and basic quality tooling is wired.

**Deliverables**
- [x] README.md (philosophy, build instructions, status)
- [x] REQUIREMENTS.md (detailed functional + non-functional with priorities)
- [x] DESIGN.md (deep architecture, data model, testing strategy, risks)
- [x] ROADMAP.md (this file)
- Professional .gitignore with OneDrive warning
- Cargo.toml with rich metadata, release profile, lints section started
- Basic GitHub Actions CI skeleton (fmt, clippy, test on all three OS)
- `cargo fmt`, `cargo clippy`, and `cargo test` all green on the skeleton
- `cargo llvm-cov --all-targets --all-features --workspace --fail-under-lines 80` green on the skeleton

**Quality Gate**
- All four planning documents reviewed by the author for internal consistency.
- `cargo clippy --all-targets -- -D warnings` produces zero warnings on the initial commit.
- Coverage gate fails below 80% line coverage, even for the skeleton.
- Git history contains the planning docs as the first real commits.

**Status:** In progress. The planning documents, skeleton binary, CI, and skeleton coverage gate exist. No usable editor features exist yet.

---

### Phase 1 - Core Pure Notepad MVP (The "It Just Works" Release)

**Goals**
Deliver a genuinely usable classic notepad replacement with the fundamentals of reliability.

**Deliverables**
- Native file open / save / save-as using `rfd`
- New document (Ctrl+N)
- Recent files (persisted, capped, clickable)
- Multi-line editing with egui `TextEdit` (or early custom widget)
- Cut/Copy/Paste/Delete + basic Undo/Redo (at least character coalescing)
- Find bar (Ctrl+F) + Find Next (F3)
- Word wrap toggle (persisted)
- Status bar with line/col, selection, char count, line ending, modified flag
- Full theme system (detect + System/Light/Dark, persisted, live update on Windows)
- Atomic save implementation (the `save_atomic` path + tests)
- Basic autosave to temp + recovery offer on launch (even if UX is rough)
- Close / quit prompt when document is dirty
- Window position/size/maximized persistence
- All primary keyboard shortcuts working on Windows + at least documented for other platforms
- Graceful handling of line endings + BOM fidelity (with golden tests)

**Quality Gate (Mandatory) - Phase 0/1 Rigor Enhancements**
- `cargo fmt -- --check` && `cargo clippy --all-targets -- -D warnings` clean.
- Core modules (`core/document`, `core/editor`, `io` logic) achieve **>= 80% line coverage**, including the executable forms of Safety Properties S1-S4 and U1 (see DESIGN.md section 3.5, added in response to RIGOROUS_REVIEW.md).
- First-cut FMEA table (DESIGN section 13.1) is committed and at least the Phase-1 rows (F1-F3, F6) have corresponding fault-injection or property tests.
- Property tests exist and pass for:
  - Line ending detection + `to_bytes()` roundtrip for CRLF / LF / CR + BOM combinations (S2).
  - Atomic save never leaves a truncated file (simulated via temp dir tests) - evidence for S1.
  - Undo roundtrips under arbitrary edit sequences (U1 / S3).
- At least 5 golden integration tests in `tests/integration/` covering empty file, CRLF-heavy log, unicode, very long single line, file with only newlines. These are the primary artifacts for S2.
- Mental model impact statements written for every feature delivered in the phase (REQUIREMENTS section 1.1).
- Dependency health table (per DESIGN section 1.4) committed for all crates introduced.
- Manual test pass on Windows (author's machine) using the written checklist in `docs/manual-test-matrix.md`.
- The author can use the binary for real note-taking for 3 consecutive days without data loss or major workflow breakage.
- Binary size (release) on Windows < 18 MiB.
- RIGOROUS_REVIEW.md has been re-read; a short review response subsection added noting which recommendations were actioned.

**Rough Calendar:** 2-4 weeks of focused work (depending on how quickly the custom editor widget is needed).

**Exit Criteria:** We have something that can replace Notepad for plain text work on the primary platform.

---

### Phase 2 - Editing Trust & Polish (The "I Actually Trust This" Release)

**Goals**
Make the editor feel excellent and make the reliability story bulletproof.

**Deliverables**
- Production-grade custom `EditorWidget` (virtualized, only visible lines, proper cursor/selection, horizontal scroll when needed).
- Excellent undo/redo with full coalescing rules + bounded memory (property tests proving roundtrips).
- Replace (Ctrl+H) with "Replace" and "Replace All".
- Go To Line (Ctrl+G) - works instantly even on 500k line files.
- File-changed-on-disk detection (mtime + quick content check) with clear reload prompt.
- Mature autosave + recovery UX (timestamped list, "Discard this recovery", auto-clean on successful save).
- Find improvements: case sensitive, "whole word", live match count + highlight in the document (if editor widget supports it).
- Better error messages and confirmation dialogs (Save / Discard / Cancel is rock solid).
- Font size zoom (Ctrl + wheel + menu items) with persisted size.
- Performance numbers documented and meeting NFR-PERF targets on reference hardware (50 MiB file open time, scroll fps on large docs).
- Crash recovery simulation harness (documented script or test that actually exercises the recovery path under process kill).

**Quality Gate**
- Core coverage **>= 85% line / 70% branch**, with explicit coverage of the safety properties and FMEA rows addressed in this phase.
- Property tests now also cover:
  - Arbitrary sequences of inserts/deletes + undos + redos return to identical document + cursor state (U1/S3).
  - Undo stack memory stays bounded under heavy typing.
- Updated FMEA table with new rows discovered during implementation; corresponding fault-injection cases added to the harness.
- Full manual test matrix executed and signed off on **Windows + Linux** (Wayland preferred for one run). Includes mental model impact validation for delivered features.
- No data loss in 30 simulated "kill during heavy editing + restart + recover" cycles (evidence for NFR-REL-01 and F1/F3).
- `cargo bloat` or equivalent used to produce a report; top 5 largest contributors documented in an issue or DESIGN addendum. Dependency health table refreshed.
- All NFR-REL reliability requirements have passing tests or documented manual verification.
- RIGOROUS_REVIEW.md re-read; response notes updated.

**Rough Calendar:** 3-5 weeks.

**Exit Criteria:** This is the point at which the author (and early testers) can switch their daily plain-text workflow to Noter with high confidence.

---

### Phase 3 - 2026 Quality-of-Life - Markdown View + Power User Features

**Goals**
Add the Markdown preview that was promised without compromising the pure text soul. Add the last set of "I use this every day" features.

**Deliverables**
- View > Markdown Preview toggle (right-side resizable pane preferred).
- Pure-Rust `pulldown-cmark` + custom egui renderer for headings, lists, code, emphasis, blockquotes, links (non-interactive in v1).
- Live (debounced) preview updates.
- Graceful degradation for huge documents (render first N lines, button to render more).
- Keyboard shortcut reference (Help > Keyboard Shortcuts or a simple modal).
- Format menu with explicit "Line Endings" and "Encoding" (even if encoding is always UTF-8 for v1, the UI exists for future).
- Optional line numbers (View menu).
- Drag-and-drop file onto the window to open (nice-to-have).
- Refined status bar (perhaps clickable elements to change line endings or jump to line).
- Refined find bar (previous/next buttons, close with Esc, search term history?).
- All shortcuts verified on macOS (Cmd key) and Linux.

**Quality Gate**
- Preview feature has its own small test suite (parser events -> expected egui draw calls, or at least "does not panic on common markdown" + snapshot of rendered structure if we adopt `insta`).
- Adding the preview crate does not push release binary over the size budget (or we accept a documented increase and re-optimize elsewhere).
- Manual test on all three platforms (or at least Windows + one other) including "edit while preview is open", "toggle preview on a 10k line .md file", "preview with code blocks and lists looks usable".
- No regressions in Phase 1-2 core tests.
- Documentation updated (README features list, user-facing "Markdown Preview" section in a small `docs/USAGE.md`).

**Rough Calendar:** 2-3 weeks.

**Exit Criteria:** The "Markdown for QOL" promise from the original request is delivered without having turned Noter into a webview or Markdown-first app.

---

### Phase 4 - Cross-Platform Hardening, Packaging & v0.1 Release

**Goals**
Make the release process professional and ensure the product is supportable on all target platforms.

**Deliverables**
- `cargo-dist` configuration that produces:
  - Windows: `.msi` installer + portable `.exe` zip
  - macOS: universal (x86_64 + aarch64) `.dmg` + app bundle (notarized if we have the certs)
  - Linux: `.tar.gz` + `.deb` (optional `.rpm`)
- GitHub Actions release workflow that runs the full test + clippy + coverage matrix on all three OSes.
- SBOM generation (basic `cargo auditable` or `cargo-cyclonedx`) included in releases.
- Signed release binaries where possible (Windows code signing cert is a stretch goal; at minimum we publish SHA256SUMS).
- Final performance & reliability numbers published in the release notes.
- `docs/manual-test-matrix.md` completed and used for the release sign-off.
- User-facing documentation: `docs/USAGE.md` (short), keyboard reference, recovery behavior explanation.
- "Known issues" and "How to report bugs" section in README.
- Version bumped to 0.1.0 (or 0.5.0 if we feel it deserves more maturity signaling).

**Quality Gate**
- Full CI matrix (Windows, macOS 13/14, Ubuntu 22.04/24.04, both X11 and Wayland where possible) is green on the release tag.
- Coverage report for the release shows core >= 85%, including the safety properties.
- At least two people (author + one external tester) have used the release candidate as daily driver for >= 7 days with no data loss incidents. Structured dogfooding log (including mental model surprises) is committed.
- Binary sizes on all published artifacts meet or are documented against the NFR-PERF-05 targets.
- `cargo clippy -- -D warnings` and fmt are clean on the exact commit tagged for release.
- A "v0.1 Release Checklist" issue is created, all items checked, and closed with the release.
- `STEWARDSHIP.md` (per DESIGN section 12.1), updated FMEA, final dependency health table, and Markdown scope contract are part of the release tag.
- RIGOROUS_REVIEW.md has a final response note summarizing what was achieved vs. deferred.

**Rough Calendar:** 2-4 weeks (a lot of this is infrastructure + waiting for CI runs and tester feedback).

**Exit Criteria:** We have a real, shippable v0.1 that we are proud to point people at.

---

### Post-v0.1 (Future Phases - Only After Demand & Maintainer Capacity)

- Optional tabs / multi-document interface (many classic Notepad users prefer multiple windows; this is controversial).
- Better horizontal scrolling / very long line handling.
- Simple syntax highlighting (as an opt-in, never on by default, still plain text on disk).
- Embedded font for perfect cross-platform monospace (size cost vs benefit analysis).
- Theming beyond light/dark (very low priority).
- "Portable mode" that stores config next to the binary.
- Any feature that would require a webview or heavy JS.

These will only be considered after v0.1 has real users and the core reliability story remains flawless.

## Metrics We Will Track

- Binary size (per platform, release profile)
- Core line/branch coverage (per phase gate)
- Number of "data loss" or "corrupted save" bugs found in testing (target: 0 after Phase 1)
- Time to open 10 MiB / 50 MiB reference files
- Frame time (p99) while scrolling a 100k line file
- Number of days the primary author uses Noter as daily driver without switching back

## How to Read This Roadmap

- Do not add new user-facing features in the middle of a phase unless the quality gate for the previous work is already green.
- When in doubt, cut scope to protect the reliability and test invariants.
- The planning documents (REQUIREMENTS + DESIGN) are the specification. If something feels wrong during implementation, update the documents first.

---

**This is how you build software that lasts and that people actually trust.**

Next concrete step after Phase 0 gate: begin Phase 1 implementation with the first working window + menu + file open/save using the architecture laid out in DESIGN.md.
