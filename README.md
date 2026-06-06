# Noter

**A deliberately pure, reliable, cross-platform plain text editor written in Rust.**

Noter is the notepad you actually want in 2026: fast, trustworthy, boring in the best way, with zero telemetry, zero bloat, and zero "we know better than you" decisions. It respects classic Notepad muscle memory while adding the minimum modern quality-of-life improvements (system-matched light/dark theme + an optional, non-intrusive Markdown preview).

> "I just want to open a file, edit text, and save it without drama, across every machine I use."

## Philosophy

- **Purity first.** Core experience is always plain text. No hidden formats, no "smart" rewriting of your content.
- **Reliability is non-negotiable.** Atomic saves, crash recovery, line-ending fidelity, and "never lose user data" are first-class.
- **Minimal surface area.** Small binary, small RAM, small dependency tree, small attack surface. No network calls ever.
- **System native where it matters.** Follows your OS light/dark setting. Uses real native file dialogs. Feels at home on Windows, macOS, and Linux.
- **Keyboard-centric.** Every important action has a discoverable shortcut. Mouse is optional.
- **Markdown is a view, not a mode.** Optional preview pane exists for comfort in 2026. It never changes how your `.md` file is saved.

Noter will never become a second VS Code, a note-taking app with accounts, or a "productivity suite."

## Current Status

This project is in the **exceptional planning phase**. All major requirements, design decisions, architecture, testing strategy, and phased roadmap have been written out in detail before any significant code was written.

See:
- [REQUIREMENTS.md](REQUIREMENTS.md) — what must be true
- [DESIGN.md](DESIGN.md) — how it will be built (deep technical)
- [ROADMAP.md](ROADMAP.md) — how we get there with quality gates

Implementation will proceed in strict phases with explicit "Definition of Done" criteria that include code quality, test coverage, and cross-platform verification.

## Planned Features (High Level)

**MVP / Phase 1 (Pure classic notepad experience)**
- New, Open, Save, Save As, recent files (capped)
- Cut / Copy / Paste / Delete / Undo / Redo (excellent coalescing)
- Find + Find Next, basic Replace
- Word wrap toggle
- Status bar (line, column, selection, encoding, line endings, modified)
- System light/dark + manual override that persists
- Proper handling of line endings (preserve what was on disk)
- Atomic safe saves + basic autosave/recovery
- Close prompt when dirty

**Phase 2–3 (Polish + 2026 QOL)**
- Go To Line
- Improved search (case, whole word, live match highlighting)
- File-changed-on-disk detection with reload prompt
- Mature autosave + recovery workflow
- Optional split or toggle Markdown preview (pure Rust rendered, read-only, does not affect save)
- Font size zoom (Ctrl+wheel and menu)
- Window state persistence (position, size, maximized)

**Explicit Non-Goals (at least for v0.1 / v0.2)**
- Tabs or "workspaces" by default (you can launch multiple instances)
- Built-in syntax highlighting in the editor
- LSP, git integration, terminals, plugins
- Cloud sync, accounts, or any network behavior
- Heavy theming or "beautiful" UI beyond clean + system-appropriate
- Rich text editing (the file on disk is always plain UTF-8 text)

## Building & Running

### Prerequisites

- Rust 1.85+ (we target edition 2024). Install via [rustup](https://rustup.rs/).
- On Windows: the MSVC toolchain (default via rustup on Windows).
- For full cross-platform testing you will eventually need macOS and Linux machines (or CI).

### Development

```bash
git clone https://github.com/yourname/noter   # (placeholder)
cd noter

# Recommended: develop outside sync folders (see .gitignore)
cargo run --release
```

Useful quality commands (these will be enforced in CI and before every phase gate):

```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
# Coverage (once set up)
cargo install cargo-llvm-cov
cargo llvm-cov --all-features --workspace --lcov --output-path lcov.info
```

### Release binary size (target)

We aim for final stripped release binaries under ~8–12 MiB on all platforms through:
- `opt-level = "z"` or "3" + LTO + strip in `Cargo.toml` profiles
- Careful dependency selection
- `cargo bloat` and `cargo tree` audits

## Platform Support (Target)

| Platform          | Minimum          | Notes                              |
|-------------------|------------------|------------------------------------|
| Windows           | 10 (build 19041+) / 11 | Primary dev platform for author   |
| macOS             | 13 Ventura+      | Apple Silicon + Intel              |
| Linux             | Ubuntu 22.04 / Fedora 39+ | X11 and Wayland (both tested)     |

We will maintain a small manual test matrix on real hardware for each release.

## Data Safety & Privacy

- Zero network activity. The binary makes no outgoing connections.
- All state is local (small TOML config + window state).
- Autosave/recovery files live in the OS temp directory and are cleaned on normal exit when possible.
- We will never read files you did not explicitly open.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Acknowledgments

- The egui and eframe maintainers for making a pure-Rust immediate-mode GUI that is actually pleasant to build real tools with.
- The ropey authors for the best text rope in the business.
- Every developer who has ever been annoyed by the direction of their OS's default text editor.

---

**Noter exists because sometimes the best software is the one that gets out of your way.**

See [ROADMAP.md](ROADMAP.md) for how we intend to build this exceptionally well.
