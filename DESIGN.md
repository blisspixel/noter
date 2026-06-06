# Noter Design Document

**Deep technical design for a high-quality, pure, reliable 2026 cross-platform notepad.**

This document is intentionally long and specific. It exists so that implementation decisions are not rediscovered under time pressure and so that code quality + test coverage goals are designed in from the beginning, not bolted on.

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
- Markdown preview will require a custom renderer (see section 8). It will be "good enough" rather than pixel-perfect beautiful.
- Native menu bar on macOS is slightly more work (egui can do global menus or we can use platform-specific code via `objc2` / `windows` crate later if needed; start with in-window top panel for simplicity and consistency).

**Considered & Rejected Alternatives (recorded for posterity):**
- **Tauri v2**: Excellent packaging and trivial Markdown (just HTML). Loses on binary size, purity (webview + web technologies in the stack), startup time, and "this is a native tool" feel. Would be the right choice for a more complex app or one where rich preview was the #1 feature. For a notepad whose soul is "plain text that I trust," it is the wrong default.
- **Iced**: More principled Elm-like architecture. More boilerplate for the simple things we need. Better for very large, complex UIs that will live for years. Overkill here.
- **Slint**: Promising declarative UI. Still smaller ecosystem for custom text-heavy widgets in mid-2026.
- **GTK4-rs / Adwaita**: Poor native experience on Windows and macOS. Not worth the pain for a tool that must feel at home everywhere.
- **Winit + raw wgpu + manual everything**: Too much work for the scope. egui gives us the right 80% for free.

We will re-evaluate only if egui fundamentally cannot deliver the editor performance or accessibility we need after Phase 2 prototype.

### 1.2 Core Text Buffer: ropey

`ropey` (v1.x) is the industry standard in the Rust text editor community for good reason:
- Logarithmic insert/delete at arbitrary positions.
- Excellent slicing and chunk iteration (critical for rendering only visible lines).
- Clone is O(1) (reference counted + COW under the hood) — perfect for undo snapshots.
- Battle tested in Lapce, multiple personal editors, and various tools.

We will wrap it in our own `Document` type rather than leaking `Rope` everywhere.

### 1.3 Other Core Crates (Planned, to be introduced by phase)

**Phase 0–1 baseline (must be justified):**
- `rfd` — real native file dialogs on all platforms. Non-negotiable.
- `directories` — correct config/cache locations.
- `serde` + `toml` — human-readable, diffable config.
- `thiserror` — clean domain error types.
- `tracing` + `tracing-subscriber` (with a file appender + simple rolling) for post-mortem diagnostics. We will not log PII.

**Phase 2+:**
- `pulldown-cmark` — CommonMark compliant, fast, event-based parser. Perfect for a controlled renderer.
- `proptest` — property-based testing of core invariants.
- `notify` (optional, low priority) — for file change detection. We can start with simple mtime polling (every 2–3 seconds when window is focused) which is dramatically simpler and sufficient for a notepad.
- `tempfile` — for test fixtures and safe temp work.

**Explicitly avoided (unless extraordinary justification appears later):**
- Any async runtime (tokio, async-std) in the main thread.
- `egui_extras` or large widget crates unless a specific small module is needed.
- Image loading crates (preview will not load external images).
- Regex engines heavier than what we need for find (we can start with `memchr` + simple loops, or `regex` crate only when justified).

### 1.4 Rust Edition & Tooling

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

## 4. Text Editing Engine & Rendering

**Initial approach (Phase 1):** Wrap `egui::TextEdit` inside a container that gives us the status bar and find bar. This lets us deliver a working editor in hours instead of days.

**Production approach (must be reached by end of Phase 2 or early Phase 3):**
- Custom `EditorWidget` that:
  1. Uses `rope.chunks()` + line iteration to only layout the visible ~80–120 lines.
  2. Caches `Galley` (egui's laid-out text) per visible line or per chunk.
  3. Handles its own cursor painting, selection rectangles, and caret blinking.
  4. Translates mouse and keyboard events into `EditorCommand`s that mutate the document + cursor atomically.
- This gives us full control over undo grouping, exact column behavior on wrapped lines, and performance.

Long lines (> 2000 chars) are a known difficult case. We will clip or horizontally scroll them with an explicit horizontal scrollbar when word wrap is off.

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

    // Main area + conditional preview
    if self.markdown_preview_open {
        egui::SidePanel::right("preview").resizable(true).show(...);
    }

    egui::CentralPanel::default().show(ctx, |ui| {
        self.editor_widget.ui(ui, &mut self.editor);
    });

    // Status
    egui::TopBottomPanel::bottom("status").show(...);
}
```

Menus are built with `egui::menu::bar` + submenus. We will define a `Command` enum and a central dispatch so keyboard shortcuts and menu items share the same code path.

## 7. Theming & System Integration

`platform/theme.rs` will contain:

- `fn detect_system_theme() -> ThemePreference`
- On Windows: use `winreg` (or the `windows` crate in 2026) to read `HKCU\...\Personalize\AppsUseLightTheme`.
- On macOS: use `objc2` or `cocoa` bindings to query `NSApp.effectiveAppearance`.
- On Linux: `gsettings get org.gnome.desktop.interface color-scheme` or fall back to `xdg` environment.

When preference is "System", we install a listener (on Windows via `WM_SETTINGCHANGE` hook in the winit event loop that eframe exposes; on other platforms we can poll or use platform notifiers).

egui `Visuals` are swapped by calling `ctx.set_visuals(visuals_for(theme))`. We will define two clean `Visuals` structs (light and dark) with good selection colors, rather than relying only on the built-in ones.

Font: We will load a small set of good monospace fallbacks (JetBrains Mono, Fira Mono, Consolas, Menlo, DejaVu Sans Mono, Noto Sans Mono) using egui's font loader. User can only control size in v1.

## 8. Markdown Preview (Purity-Constrained)

**Parser:** `pulldown-cmark` 0.12+ (or current). We use the event iterator, not HTML output.

**Renderer:** A custom `fn render_markdown(ui: &mut Ui, events: impl Iterator<Item = Event>)` that draws:
- Headings with larger font + bold
- Bullet and numbered lists with proper indentation
- Fenced code blocks with a slightly different background and monospace
- Emphasis / strong
- Links as colored text (non-interactive in v1; later we can make them copy-to-clipboard on click)
- Blockquotes with a left border

We will **not** support:
- Raw HTML
- Images (or only data: URIs with extreme caution)
- Tables (stretch goal; pulldown-cmark supports them with an extension)

The preview widget will be given a `&Rope` slice or the full rope + a "render up to line N" limit for large files.

Because everything is pure Rust + egui primitives, the preview adds almost no attack surface.

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
4. **UI smoke tests** — limited. egui has some testing support via `egui::Context` in a headless way or by recording frames. For v1 we will rely on "the app starts and the main widget renders without panic" + extensive manual testing.
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

## 13. Risks & Mitigations

- **Editor performance on huge files** — Mitigated by designing the virtual widget from the start and having clear performance tests. Fallback: keep `TextEdit` path as a "SimpleEditor" mode.
- **Theme detection bit-rot on macOS/Windows** — Mitigated by having a manual override always available and by writing the detection in small, well-commented platform modules with fallback to "Light".
- **Atomic save races on Windows** — Well-known problem. Mitigation: research current best practice in 2026 (the `atomicwrites` crate or hand-rolled with `ReplaceFile` on Windows). Test aggressively.
- **OneDrive / cloud sync folders** — Users will put their notes in synced folders. We must handle the case where the file disappears or is conflicted during save. Document the risk and offer clear messages.
- **Scope creep** — The REQUIREMENTS.md non-goals list + the phase gates with "Definition of Done" that explicitly says "no new features until quality bar is met" are the primary defenses.

## 14. Open Questions (to be closed before or during Phase 1)

- Exact default font stack and whether we embed a font (increases binary size).
- Whether the find bar should support regex in v0.1 (probably not — start with literal).
- How aggressive to be with horizontal scrolling vs forcing wrap on extremely long lines.
- Whether to persist "last used directory" separately from recent files.

---

This design is the blueprint. Implementation should feel like translating well-understood specifications into clean Rust, not like exploratory coding.
