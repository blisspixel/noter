# Noter Technical Design Document

**Deep technical design for a high-quality, pure, reliable 2026 cross-platform notepad.**

This document is intentionally long and specific. It exists so that implementation decisions are not rediscovered under time pressure and so that code quality + test coverage goals are designed in from the beginning, not bolted on.

**Rigor note (June 2026):** This design has undergone a structured adversarial design review (see [RIGOROUS_REVIEW.md](RIGOROUS_REVIEW.md)). The review identified specification gaps, the need for explicit invariants, a first FMEA, dependency governance, and stewardship planning. The sections below (particularly 3.5, 4.6, 13, and 15) have been expanded or added in direct response. The goal is to make Noter an existence proof that small desktop tools can be engineered with professional-grade discipline rather than hobbyist accretion.

## 1. Technology Stack & Architectural Decisions

### 1.1 GUI Layer: egui + eframe (Chosen)

**Decision Record (DR-001)**

We chose `eframe` (the official egui application framework) + `egui` as the sole GUI technology.

**Why egui wins for Noter's values:**
- Produces genuinely small, self-contained binaries (no webview runtime, no JS engine, no hundreds of MB of Chromium).
- Immediate mode is an excellent fit for a text editor: the entire UI is a pure function of state on every frame. This dramatically reduces "I edited text but the UI didn't notice" classes of bugs.
- Excellent high-DPI and scaling support out of the box.
- The egui text layout and font system (powered by cosmic-text or harfbuzz under the hood in recent versions) is good enough for a plain text + monospace editor.
- Mature in 2026. Many production tools (hex editors, log viewers, configuration UIs, small IDEs) ship successfully with it.
- Zero network, zero "web tech" in the core product — matches the purity requirement perfectly.

**Trade-offs accepted:**
- We must implement (or carefully wrap) our own multi-line virtualized text editor widget. egui's `TextEdit` is convenient for prototypes but will eventually be insufficient for 100k+ line files and fine-grained undo.
- ### 6.2 Inline Markdown Styling (Phase 3)
We will NOT use a split-pane HTML webview. That violates the purity mandate.
Instead, we will parse the plaintext buffer using `pulldown-cmark` (or a lightweight regex highlighter) and dynamically apply `egui` rich-text styles (bold, larger fonts for headings, colors for links) directly to the plaintext characters in the editor view itself. The file remains 100% pure text, but the user gets a beautiful "rich text" experience while editing.
- Native menu bar on macOS is slightly more work (egui can do global menus or we can use platform-specific code via `objc2` / `windows` crate later if needed; start with in-window top panel for simplicity and consistency).

**Considered & Rejected Alternatives (recorded for posterity):**
- **Tauri v2**: Excellent packaging and trivial Markdown (just HTML). Loses on binary size, purity (webview + web technologies in the stack), startup time, and "this is a native tool" feel. Would be the right choice for a more complex app or one where rich preview was the #1 feature. For a notepad whose soul is "plain text that I trust," it is the wrong default.
- **Iced**: More principled Elm-like architecture. More boilerplate for the simple things we need. Better for very large, complex UIs that will live for years. Overkill here.
- **Slint**: Promising declarative UI. Still smaller ecosystem for custom text-heavy widgets in mid-2026.
- **GTK4-rs / Adwaita**: Poor native experience on Windows and macOS. Not worth the pain for a tool that must feel at home everywhere.
- **Winit + raw wgpu + manual everything**: Too much work for the scope. egui gives us the right 80% for free.

We will re-evaluate only if egui fundamentally cannot deliver the editor performance or accessibility we need after Phase 2 prototype.

### 1.2 Core Text Buffer: ropey (June 2026 decision)

As of June 2026 the latest published version is `ropey 2.0.0-beta.1`. We are **intentionally staying on the latest stable 1.x series** (target `ropey = "1.6"`) for Noter v0.x.

Reasons (nerd consensus):
- A "never lose the user's text" tool should be built on the most battle-tested, non-beta foundation possible for its core data structure.
- 1.x has years of production use in editors and tools. The 2.0 beta brings SIMD and metric improvements that are nice-to-have, not must-have for a notepad.
- We will re-evaluate ropey 2.0 (or its GA successor) only after v0.1 and only if it demonstrates superior reliability characteristics in our own test harness.

We will wrap it in our own `Document` type rather than leaking `Rope` everywhere. This also gives us an abstraction seam if we ever need to swap the rope implementation.

**Byte-to-Char Indexing Strategy (Crucial for 1.x):**
Because `ropey 1.x` uses character-based indexing while `egui` and many standard APIs use byte-based indexing, we are exposed to severe Unicode boundary bugs (the exact issue Ropey 2.0 fixes). 
Our `EditorWidget` and `Document` translation layer must explicitly isolate this:
- All external APIs (egui events, OS clipboards) speak strictly in byte offsets.
- Only the very innermost boundary of `Document` is allowed to translate a byte offset to a `ropey` char index using `rope.byte_to_char()`.
- We will write property tests that throw arbitrary combining characters (ZWJ, emoji, CJK) at the editor and verify that byte-to-char-to-byte roundtrips never panic or slice strings midway through a codepoint.

### 1.3 Other Core Crates (June 2026 GA versions, introduced by phase)

**Phase 0–1 baseline (current as of June 2026, must still be justified before addition):**
- `egui` + `eframe` = "0.34.3" — latest GA. MSRV 1.85. This is our GUI layer (see decision record above).
- `rfd` = "0.17" — real native file dialogs. Non-negotiable.
- `directories` = "6.0" — correct config/cache locations.
- `serde` + `toml` = "1.1" — human-readable, git-friendly, diffable config.
- `thiserror` = "2.0" — clean domain error types.
- `dark-light` = "2.0" — recommended for system theme detection. It encapsulates the Windows registry, macOS NSAppearance, and Linux gsettings logic behind a simple API. Much less maintenance than hand-rolled platform code.
- `tracing` = "0.1.44" + `tracing-subscriber` = "0.3.23" (with file appender + env-filter for post-mortem diagnostics; we log nothing sensitive).

**Phase 2+:**
- `pulldown-cmark` = "0.13" — CommonMark compliant, fast, event-based parser. Perfect for a controlled, pure-Rust markdown renderer.
- `proptest` = "1.11" — property-based tests for the core invariants (undo roundtrips, line-ending fidelity, etc.).
- `tempfile` = "3.27" — for golden I/O tests and recovery simulation.
- `notify` — deliberately deferred. As of June 2026 the 9.0 series is still at rc. We start with simple periodic mtime + size polling (2-3s when focused). Only adopt a filesystem watcher after it is GA *and* we have proven the polling version insufficient in real use.

**Explicitly avoided (unless extraordinary justification + size audit appears later):**
- Any async runtime (tokio etc.) on the main thread.
- Large egui widget crates or egui_extras unless a tiny specific module is extracted.
- Any image loading or network-capable crates (the markdown preview will never load remote images).
- Heavy regex engines for find (start with literal search + memchr; add the `regex` crate only with written justification).

### 1.4 Dependency Governance Policy (June 2026)

In direct response to the critical review, we adopt an explicit (if lightweight) governance process rather than ad-hoc "latest GA" decisions.

For every crate introduced or upgraded at a phase gate, the following must be recorded in an appendix or commit message and summarized in a table committed alongside the gate:

- Exact version pinned (or range with upper bound).
- Date of last upstream release and number of active maintainers (bus-factor estimate from GitHub/org activity).
- Transitive dependency count (`cargo tree -i <crate>` + `cargo tree --duplicates`).
- Our usage surface (which features, which modules import it).
- Upgrade risk assessment (breaking change likelihood, API stability history).
- Security / supply-chain posture (does it do I/O or networking at initialization? Any "phone home" in tests or build scripts?).
- Rationale tied back to a specific requirement (e.g., "required for S1 atomic save cross-platform").

**Current conservative stance examples (as of June 2026):**
- ropey stays on 1.6.x stable series until 2.x demonstrates multi-year production use in at least two other editors; 2.0-beta.1 is monitored but not adopted for v0.x.
- notify 9 remains deferred while at rc; mtime polling is the baseline because it has a vastly smaller failure mode surface.
- dark-light 2.0 is accepted because its entire purpose is narrow, its MSRV is reasonable, and it eliminates hand-maintained platform bindings that are a common source of bit-rot.

At every phase gate the lead must re-run the health table for crates already in the tree and present it as part of the "Definition of Done" evidence.

### 1.5 Rust Edition & Tooling

- Edition: 2024 (already set in the initial `Cargo.toml`).
- `rust-version`: will be set to a recent but supportable version once we have CI (likely 1.90+ or whatever the 2026 stable baseline is).
- Profiles: aggressive release optimizations for size + speed (see Packaging section).
- Lints: We will have a `[lints.clippy]` table with `pedantic` + selected restrictions turned on gradually, and `-D warnings` in CI.

## 2. High-Level Architecture

```
noter (bin)
├── src
│   ├── main.rs                 # eframe entry point, App bootstrap
│   ├── app.rs                  # The main NoterApp struct implementing eframe::App
│   ├── core/
│   │   ├── mod.rs
│   │   ├── document.rs         # Document + LineEnding + Encoding + load/save logic
│   │   ├── editor.rs           # Editor state (cursor, selection, viewport) + edit ops
│   │   ├── undo.rs             # Undo stack + coalescing policy
│   │   └── recovery.rs         # Autosave + recovery scanning
│   ├── ui/
│   │   ├── mod.rs
│   │   ├── menu.rs             # Menu construction (egui::menu)
│   │   ├── editor_widget.rs    # The custom (or wrapped) text editor widget
│   │   ├── find_bar.rs
│   │   ├── status_bar.rs
│   │   └── markdown_preview.rs # Pure-Rust markdown renderer
│   ├── platform/
│   │   ├── mod.rs
│   │   ├── theme.rs            # System theme detection + live updates
│   │   └── shortcuts.rs        # Cmd vs Ctrl abstraction + key binding table
│   ├── config.rs               # AppConfig + persistence (TOML)
│   └── error.rs                # NoterError + user-facing error mapping
├── tests/
│   └── integration/            # Golden file tests, cross-cutting behavior
└── Cargo.toml
```

The `core` modules have **no knowledge of egui**. This is critical for testability and for the (remote) possibility of one day offering a terminal or headless mode. All egui types live in `ui/` and `app.rs`.

## 3. Core Domain Model

### 3.1 LineEnding (strong type)

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum LineEnding {
    Lf,      // \n
    CrLf,    // \r\n
    Cr,      // \r (rare, but must round-trip)
}

impl LineEnding {
    pub fn as_str(&self) -> &'static str { ... }
    pub fn detect_from_bytes(bytes: &[u8]) -> (Self, usize /* bom len or 0 */);
}
```

### 3.2 Document

```rust
pub struct Document {
    pub rope: Rope,
    pub path: Option<PathBuf>,
    pub line_ending: LineEnding,
    pub had_bom: bool,
    pub is_dirty: bool,
    // last_saved_mtime, content_hash for change detection, etc.
}

impl Document {
    pub fn from_path(path: &Path) -> Result<Self, NoterError>;
    pub fn from_bytes(bytes: &[u8], path: Option<PathBuf>) -> Self;
    pub fn to_bytes(&self) -> Vec<u8>;   // applies BOM + chosen line endings
    pub fn save_atomic(&self) -> Result<(), NoterError>; // the critical method
}
```

`Document` owns the authoritative text. `Editor` owns transient UI state (cursor position, scroll, selection, viewport offset).

### 3.3 Editor & Undo

The editor will contain:
- Reference to (or owned) `Document`
- `Cursor { line: usize, column: usize }` (logical, not visual)
- `Selection` (anchor + head, or collapsed)
- `UndoStack` (Vec of `UndoAction` or a more sophisticated structure with checkpoints)
- Viewport state (first visible line, scroll offset in pixels for sub-line precision)

Undo coalescing rules are defined in `undo.rs` and must be property-tested.

### 3.4 AppConfig (persisted)

```toml
[app]
theme = "System"          # or "Light" | "Dark"
font_size = 15.0
word_wrap = true
show_line_numbers = false
recent_files = ["/path/to/a.txt", ...]   # capped + deduped

[window]
x = 120
y = 80
width = 900
height = 700
maximized = false

[editor]
autosave_interval_secs = 25
undo_limit = 800
```

Config is loaded at startup, saved on clean exit and on major preference changes. We use a simple "write whole file" strategy (the file is tiny).

### 3.5 Core Behavioral Specification (Lightweight Formalization)

In response to the critical review (RIGOROUS_REVIEW.md §3.1), we document the intended safety and liveness properties explicitly. These are not TLA+ (yet), but they are precise enough to be turned into property tests and to serve as acceptance criteria for the core modules.

#### Safety Properties (must never be violated)

**S1 — Save Fidelity**  
For any `Document` and target `Path`, if `save_atomic(doc, path)` returns `Ok(())`, then the bytes observed on disk at `path` (after the operation) must be byte-for-byte identical to `to_bytes(doc)`, subject only to the documented line-ending normalization and BOM policy that were determined at load time for that document.

**S2 — Line Ending & BOM Preservation**  
The `line_ending` and `had_bom` fields recorded at load time are the *only* values that `to_bytes()` is permitted to use when emitting content for a subsequent save of the same logical document. No "helpful" normalization to the host platform's native ending is allowed on save unless the user has explicitly chosen a different ending via the UI (an operation that must itself be undoable and clearly indicated in the status bar).

**S3 — Undo Information Preservation (Content Only)**  
For any sequence of mutating `EditorCommand`s C1 … Cn that the undo system classifies as "content-affecting," applying the corresponding undo actions must restore both the `Rope` content *and* the logical (line, column) cursor/selection state to a state that is information-theoretically equivalent to the state before the sequence (viewport and transient UI state are explicitly excluded from this guarantee).

**S4 — No Silent Data Loss on Close**  
If `is_dirty` is true and the user requests close/quit, the only permitted outcomes are: (a) successful save to the current or a new path, (b) explicit user confirmation to discard, or (c) cancellation of the close. There must be no code path that drops the in-memory `Rope` without one of the above.

#### Liveness & Progress Properties (under normal conditions)

**L1 — Save Progress**  
If the filesystem is writable, has sufficient space, and is not experiencing pathological latency, a Save request must terminate (success or documented error) within a bounded multiple of the size of the document (modulo `fsync` costs that the OS controls).

**L2 — Recovery Offer**  
On launch, if any autosave artifacts belonging to this user and newer than N hours exist and their owning process is no longer running, the application must surface a recovery offer before presenting a normal untitled or "Open recent" document.

#### Accepted Residual Risks (documented, not hidden)

- Networked or distributed filesystems (NFS, SMB, OneDrive "Files On-Demand", etc.) may violate atomic rename visibility or `fsync` durability. We detect some cases via mtime races but cannot guarantee S1 in the presence of external writers or caching layers.
- Mandatory file locks or anti-virus scanners that hold the target file open across the rename window on Windows.
- Power loss or kernel panic *after* `fsync` on the `.tmp` but before the rename inode update is durable on certain non-journaling or log-structured filesystems.
- User confusion about "what a character is" when combining characters, zero-width joiners, or BiDi text affect column counting. We will use logical (Unicode scalar value) columns; visual columns are a best-effort status-bar hint only.

These properties and risks must be referenced (by ID) in the comments of the corresponding implementation and in the property tests that attempt to falsify them.

## 4. Text Editing Engine & Rendering

**Initial approach (Phase 1):** Wrap `egui::TextEdit` inside a container that gives us the status bar and find bar. This lets us deliver a working editor in hours instead of days.

**Production approach (Phase 2 - The Zero-Latency Engine):**
- Custom `EditorWidget` rendering engine focused on achieving <16ms latency for massive (500MB+) files and 120Hz scrolling:
  1. **Viewport Virtualization:** Only fetches `rope.chunks()` mapping strictly to the currently visible vertical scroll window (approx 80-120 lines).
  2. **Galley Caching:** `egui::Galley` (text layout) objects are cached per logical line. Scrolling vertically re-uses layout caches.
  3. **Memory Mapped Loading:** Files larger than a threshold (e.g. 10MB) are memory-mapped (`mmap`) into the `Rope` rather than loaded into heap `Vec<u8>`.
  4. Handles its own cursor painting, selection rectangles, and caret blinking on the paint layer without triggering full UI passes.
  5. Translates events to atomic `EditorCommand`s, keeping the main UI thread pristine.
- This gives us full control over undo grouping, typographic spacing, sub-pixel rendering, and unmatched performance.

Long lines (> 2000 chars) are clipped and horizontally scrolled using a subtle horizontal scrollbar when wrap is off.

## 5. File I/O & Reliability Subsystem (The Heart of Trust)

### 5.1 Load Path
1. Read entire file into `Vec<u8>` (memory map is future optimization).
2. Detect BOM + line ending style using a single pass.
3. Convert to `String` (lossy or strict per user choice).
4. Build `Rope` from the string.
5. Record original `line_ending` and `had_bom`.

### 5.2 Atomic Save (Non-negotiable implementation sketch)

```rust
fn save_atomic(&self) -> Result<()> {
    let path = self.path.as_ref().ok_or(NoterError::NoPath)?;
    let tmp = path.with_extension("txt.tmp");   // or a better unique name

    {
        let mut f = File::create(&tmp)?;
        let bytes = self.to_bytes();
        f.write_all(&bytes)?;
        f.sync_all()?;           // crucial on Windows and Linux
    }

    // On Windows, rename can fail if target exists and is locked.
    // We may need a small retry or a "replace if exists" dance.
    fs::rename(&tmp, path)?;

    // Update our metadata (mtime, dirty=false, etc.)
    Ok(())
}
```

We will write extensive tests using `tempfile::TempDir`, including simulated power-loss by writing the tmp file, then killing the conceptual "process" before rename, then verifying recovery logic.

### 5.3 Autosave & Recovery

- Autosave writes to a well-known location in `std::env::temp_dir()` with a name containing pid + timestamp or a uuid.
- On launch we look for files matching `noter-autosave-*` that are newer than N hours and whose owning process is dead.
- Recovery presents the content in a special "Recovered" document with a big "Save As…" prompt.
- We keep the autosave until the user explicitly saves or discards.

### 5.4 File Changed on Disk

Simple but effective:
- When the window regains focus or on a 3-second timer (while focused), stat the file.
- If mtime or size differs from what we last knew, read the first 4 KiB + last 4 KiB + total size. If they differ, raise the prompt.
- Full content hash only on user request ("Reload").

## 6. UI Layout (egui)

Typical frame:

```rust
fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
    // Top menu
    egui::TopBottomPanel::top("menu").show(ctx, |ui| {
        ui.horizontal(|ui| { self.build_menu(ui); });
    });

    // Optional find bar (appears below menu when active)
    if self.find_shown {
        egui::TopBottomPanel::top("find").show(...);
    }

    // Main area (Inline Markdown styling applied natively within the text view)
    egui::CentralPanel::default().show(ctx, |ui| {
        self.editor_widget.ui(ui, &mut self.editor);
    });

    // Status
    egui::TopBottomPanel::bottom("status").show(...);
}
```

Menus are built with `egui::menu::bar` + submenus. We will define a `Command` enum and a central dispatch so keyboard shortcuts and menu items share the same code path.

## 7. Theming & System Integration

`platform/theme.rs` (or a thin wrapper) will use the `dark-light` crate (2.0.0 as of June 2026) for the heavy lifting:

- `dark_light::detect()` gives us the current system preference.
- On Windows it reads the expected registry key.
- On macOS it queries effective appearance.
- On Linux it uses gsettings / freedesktop standards.

We still own a small `ThemePreference` enum (`System | Light | Dark`) and the persistence logic. `dark-light` is just the detector.

When the user chooses "System", we react to changes:
- Windows: hook `WM_SETTINGCHANGE` via the eframe/winit event loop.
- Other platforms: reasonable polling on window focus is acceptable for v1.

egui `Visuals` are swapped by calling `ctx.set_visuals(visuals_for(theme))`. We will define (or heavily customize) two clean `Visuals` structs with excellent contrast and selection colors in both modes. We do not blindly accept egui's default light/dark.

- Fenced code blocks with a slightly different background and monospace
- Emphasis / strong
## 8. The "Ruff" of Markdown (Strict Linter & Inline Styling)

**Parser:** `pulldown-cmark` 0.13+ (current GA June 2026 is 0.13.4). We use the event iterator to build an `egui::text::LayoutJob`.

**Rendering & Formatting Philosophy:**
We do NOT use a split-pane preview. The text editor buffer applies text styles (bold, headers, links) directly to the plaintext source as you type, identical to a word processor but leaving the raw markdown characters intact.

**The "Ruff" Linter Rules:**
Noter is not just a viewer; it is a strict structural enforcer. When in Markdown mode, the editor provides:
1. **Smart Indentation:** Hitting enter on a bullet list (`- item`) automatically inserts the next `- ` at the exact correct indentation level.
2. **One-Keystroke Alignment:** A global format shortcut (e.g., `Ctrl+Shift+F`) instantly normalizes the entire `.md` file to strict Markdown standards:
   - Ensuring a single space after `#` for headers.
   - Re-aligning markdown tables with perfect padding.
   - Fixing disordered numbered lists (`1.`, `2.`, `3.`).
   - Stripping trailing whitespace.

Because everything is pure Rust + egui primitives, this adds almost no attack surface, while keeping your `.md` files immaculate.

## 9. Error Handling & User Communication

We will have two error layers:

1. `NoterError` (thiserror) — internal, rich, for logging.
2. User-facing messages produced by a small `fn user_message(err: &NoterError) -> (String, Severity)`.

Dialogs will be simple egui windows or `egui::Modal` (or the pattern that exists in 2026). Types: Info, Warning, Error, Confirmation (Save/Discard/Cancel).

Never use `panic!` or `unwrap` to communicate user errors.

## 10. Code Quality Standards (Enforced)

- **Formatting & Lints:** `cargo fmt` + `cargo clippy -- -D warnings` on every push. We will gradually enable more pedantic lints.
- **Documentation:** Every item in `core/` and every UI command must have a doc comment.
- **No silent loss:** Any path that can drop user data must be explicit and tested.
- **Strong types over strings:** `LineEnding`, `ThemePreference`, `Command`, `FindOptions`, etc.
- **Resource cleanup:** Use `Drop` guards or `scopeguard` for temp file cleanup where appropriate.
- **Dependencies:** Any addition must be discussed in a short note in the commit or a DESIGN update. We will run `cargo tree --duplicates` regularly.

## 11. Testing Strategy (Designed for Coverage & Trust)

### 11.1 Levels

1. **Unit tests** — inside each module (`#[cfg(test)] mod tests`). Especially heavy in `core/document.rs`, `core/undo.rs`, `core/recovery.rs`.
2. **Property-based tests** (`proptest`) — located in `tests/proptests.rs` or inside modules:
   - Arbitrary edit sequences + undo/redo must produce identical final rope + cursor state.
   - Line ending roundtrips for all three styles + BOM combinations.
   - Config serialization is stable and roundtrippable.
3. **Golden / integration tests** (in `tests/integration/`) — real small files with known tricky content. We assert that `Document::from_bytes(...).to_bytes()` produces byte-identical output, and that atomic save produces the expected file.
4. **UI smoke tests & Automated UI Testing** — By Phase 2, we will integrate the new 2026 `egui_mcp` inspection protocol. Instead of relying solely on manual testing, we will write programmatic tests that inspect the live UI tree (e.g., verifying the "Modified" flag appears in the status bar without manual clicking). For v1, we rely on "the app starts and renders" + extensive manual testing.
5. **Crash & recovery simulation** — scripts or test harnesses that write an autosave, then corrupt the main process state, then launch a fresh `noter` binary (or call the recovery scanner directly) and assert the content is offered.

### 11.2 Coverage Target

- Core logic (`src/core/**`): ≥ 85% line, 70% branch by Phase 2 gate.
- Whole workspace: ≥ 60% by v0.1 (UI code is harder to cover meaningfully).
- We will use `cargo-llvm-cov` in CI and publish the HTML report as an artifact.

### 11.3 Manual Test Matrix (documented in ROADMAP)

For every phase gate we will run a written checklist on:
- Windows 11 (author's machine)
- At least one Linux desktop (Wayland + X11 if possible)
- macOS (if available to contributor or via CI VM)

Checklist items include large files, mixed line endings, power-loss simulation (actually pulling the power on a test laptop is dramatic but effective for early versions), theme switching, high-DPI scaling, etc.

## 12. Packaging & Distribution

We will use `cargo-dist` (the 2025–2026 standard) for releases.

`Cargo.toml` will contain:

```toml
[workspace.metadata.dist]
cargo-dist-version = "0.28.0"   # or whatever current
ci = ["github"]
installers = ["msi", "shell", "homebrew", "npm"]  # as appropriate
targets = ["x86_64-unknown-linux-gnu", "x86_64-apple-darwin", "aarch64-apple-darwin", "x86_64-pc-windows-msvc"]
```

Release profile:

```toml
[profile.release]
opt-level = "z"   # or 3 if size is acceptable
lto = true
codegen-units = 1
strip = true
panic = "abort"
```

We will also produce a pure portable `.exe` / binary that requires no installer.

## 12.1 Project Stewardship and Longevity (Response to Critical Review)

A tool that aspires to be "the one you trust for a decade" must plan for the day the original author steps away (bus factor, life, loss of interest). By Phase 4 we will produce a committed `STEWARDSHIP.md` containing at minimum:

- Exact reproducibility recipe: the pinned `rust-toolchain.toml` (or rustup commands), the exact `cargo dist` version used for the release, and a one-command "build the signed release artifacts on a fresh machine" script.
- Known fragile platform assumptions (e.g., "we assume rename is atomic within the same directory on the target FS"; "we rely on `fsync` having the documented durability semantics").
- Criteria for declaring the project "unmaintained" and the recommended migration path (point to a maintained fork, or to a different tool, with data export guidance).
- A minimal set of integration tests that a future maintainer can run in < 10 minutes to gain confidence that a patch has not broken S1–S4.

This is not over-engineering for a notepad; it is the minimum hygiene for any artifact that claims reliability as its primary value proposition.

## 13. Risks, Mitigations, and Failure Modes Analysis

### 13.1 Initial Failure Modes and Effects Analysis (FMEA)

In response to the critical review (RIGOROUS_REVIEW.md §3.1 and §4.2), we maintain an explicit FMEA. This table is a living artifact; it is updated at each phase gate with new modes discovered during implementation or testing. Severity is from the user's perspective (data loss = 10, annoyance = 3).

| ID  | Failure Mode                                      | Potential Effect on User                                      | Sev | Current / Planned Detection & Mitigation                                                                 | Residual Risk (after mitigation)                          | Phase First Addressed |
|-----|---------------------------------------------------|---------------------------------------------------------------|-----|----------------------------------------------------------------------------------------------------------|-----------------------------------------------------------|-----------------------|
| F1  | Partial/truncated write during save (power loss, kill -9, disk full mid-rename) | Silent data loss or zero-byte file replacing original        | 10  | Atomic write to sibling `.tmp` + `fsync` before `rename`. Recovery scanner on launch. Fault-injection harness that corrupts the `.tmp` or simulates rename failure. | Network/OneDrive filesystems can still lose visibility or durability. Documented. | 1 |
| F2  | Line-ending or BOM detection is wrong or "helpful" normalization occurs on save | User's version control or downstream tools see spurious diffs; trust destroyed | 9   | Single-pass detection at load; `to_bytes()` is the *only* emitter and is driven exclusively by the recorded `line_ending`/`had_bom`. Golden-file roundtrip tests for all three endings + BOM combinations. Property test S2. | Mixed-ending files inside one document have no perfect single-style representation; we pick one and stay consistent. | 1 |
| F3  | Autosave file itself is corrupted, from another instance, or from a different Noter version | Recovery offers garbage or the wrong document                 | 8   | Autosave filenames contain PID + timestamp + a small header magic + version. Recovery code verifies header and that the owning process is dead. Content is still presented as "Recovered — Save As required." | User may still be confused; clear UI language and "Discard this recovery" are mandatory. | 1–2 |
| F4  | External writer (or OneDrive sync) changes the file while we have it open | Lost work or "which version is real?" confusion               | 7   | Periodic mtime+size check on focus regain + 3s timer. Quick content fingerprint (first+last 4 KiB + len). Prompt: Reload / Keep Mine / (future) Diff. | True concurrent editing is out of scope; we only detect after the fact. | 2 |
| F5  | Very long lines or pathological Unicode (combining chars, ZWJ, BiDi) cause cursor/column mismatch or OOM during layout | User cannot reliably edit or the editor freezes               | 6   | Logical (scalar-value) columns only. Viewport virtualization + line-length clamping for layout. Explicit horizontal scroll when wrap is off. Performance tests on 10k-char lines. | Visual column count in status bar is best-effort only; documented. | 2 |
| F6  | Undo stack grows unbounded or coalescing is incorrect | Memory exhaustion or "undo did something surprising"          | 7   | Bounded undo stack (entries + approximate byte cost). Coalescing rules are property-tested (U1). | Very long editing sessions may lose the very first edits; this is accepted and the bound is user-visible in config. | 1–2 |
| F7  | Markdown preview renders something surprising or expensive from a crafted .md | User thinks the file contains active content; performance cliff | 5   | Pure event-based renderer from pulldown-cmark only. No HTML, no images, no network. Explicit scope contract (see 8). Time-budgeted rendering with "first N lines" cutoff + user affordance to render more. | Users may still paste weird Markdown expecting rich behavior; we never claim to be a full renderer. | 3 |
| F8  | Theme detection fails or goes stale on a platform update | App looks wrong or fights the system setting                  | 4   | `dark-light` 2.0 as primary + manual override always available + live Windows `WM_SETTINGCHANGE` hook. Fall back to "Light" + status message. | Platform APIs can change; we treat this as a high-priority bug on any reported mismatch. | 1 |
| F9  | IME (Input Method Editor) input fails, crashes, or corrupts text during composition | User typing in CJK/other languages loses text or crashes the app | 9 | Explicit manual test pass using IME on Windows and macOS. Leverage latest `egui` IME improvements. Automated tests via `egui_mcp` simulating complex input events if possible. | IME state machines are notoriously platform-dependent and brittle; regressions are common. | 1–2 |

New rows are added whenever a test, code review, or dogfooding session reveals a mode not previously considered. The table is the single source of truth for "what can still go wrong and why we accept it."

### 13.2 Narrative Risk Mitigations (Retained & Updated)

- **Editor performance on huge files** — Mitigated by designing the virtual widget from the start and having clear performance tests. Fallback: keep `TextEdit` path as a "SimpleEditor" mode for Phase 1 delivery. See performance numbers in NFR-PERF.
- **Theme detection bit-rot on macOS/Windows** — See F7 row and the dark-light governance note above.
- **Atomic save races on Windows** — See F1. We will research `ReplaceFile` / transactional NTFS patterns current in 2026 and implement the best portable compromise we can prove in the fault harness.
- **OneDrive / cloud sync folders** — See F1 and F4. We will add a one-time "Using Noter with cloud-synced folders" warning dialog on first save into a known sync root, plus clear documentation.
- **Scope creep** — The REQUIREMENTS.md non-goals list, the explicit Markdown scope contract (section 8), the mental-model impact statements, and the phase gates that require "no new features until quality bar is met" are the primary defenses. Any proposed addition must also survive the "Classic Notepad power user would this make their life better or introduce hidden state?" test.

## 14. Open Questions (to be closed before or during Phase 1)

- Exact default font stack and whether we embed a font (increases binary size).
- Whether the find bar should support regex in v0.1 (probably not — start with literal).
- How aggressive to be with horizontal scrolling vs forcing wrap on extremely long lines.
- Whether to persist "last used directory" separately from recent files.

---

This design is the blueprint. Implementation should feel like translating well-understood specifications into clean Rust, not like exploratory coding.
