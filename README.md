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

## Engineering Philosophy - How We Intend to Avoid Slopware

Your original pain points were very specific:

- The Windows 11 Notepad rewrite feels like spyware + bloat (telemetry, "modern" UI that got slower and more opinionated, OneDrive nagging, features nobody who loved the old one asked for).
- You want something that feels like the *earlier* Notepads: instant, dead simple, plain text, no surprises, keyboard native.
- Cross-platform that actually works well on Win/mac/Linux without becoming Electron garbage.
- A tiny bit of 2026 QOL (system theme + optional Markdown view) but "keep it pretty pure" otherwise.

**Is this a good idea in June 2026?**

Yes - *conditionally*.

The world does not need another "I built a text editor in a weekend" project. But there is still room for a *deliberately* small, zero-compromise, reliability-obsessed plain text tool whose entire reason for existence is "I am sick of the shit the OS vendors ship and I want something I can actually trust for the next 10 years."

Risks of turning into slopware are real and high:
- Scope creep ( "just tabs", "just some syntax", "just a plugin system", "just Copilot integration because it's 2026").
- Dependency bloat (one "nice" crate pulls in 40 others).
- UI/UX debt (egui makes it easy to add things that feel half-baked).
- Maintenance collapse (solo project that becomes too big to touch).

**How we make this exceptionally well made instead:**

- The phased roadmap + hard quality gates (coverage numbers, property tests proving invariants, simulated crash recovery, multi-platform manual sign-off) are not theater. They are the primary defense.
- Every new feature or dependency must survive the "Classic 2015 Notepad power user" test: Would they be happy, or would they feel the tool has started making decisions for them?
- Core logic (`core/`) has zero knowledge of egui. This is architectural hygiene that also makes the thing testable and potentially portable later.
- We publish binary size, RAM, and reliability numbers with releases. If they regress, we treat it as a bug.
- We maintain an explicit, living "Non-Goals and Why We Said No" list (this README + DESIGN).
- Dependency policy is draconian (see Cargo.toml and DESIGN). Latest GA only when it makes sense; betas almost never for core pieces (see the ropey 2.0 call we made).
- We dogfood it ourselves for weeks as a daily driver before calling any phase "done".
- We treat data loss and line-ending corruption as security-level bugs.

If at any point this starts feeling like "yet another text editor with ambitions", we delete features and go back to the boring notepad that just works.

This is the bar. The planning documents exist to make it hard to lower that bar later under time pressure or "just one more thing" requests.

## Current Status

This project is in **Phase 0: planning skeleton**. The repository has a Rust binary crate, CI, formatting and lint gates, a minimal tested status output, and planning documents. It is not yet a usable text editor.

What is currently proven:
- `cargo fmt --all -- --check` is expected to pass.
- `cargo clippy --all-targets --all-features -- -D warnings` is expected to pass.
- `cargo test --all-targets --all-features` is expected to pass.
- `cargo llvm-cov --all-targets --all-features --workspace --fail-under-lines 80` is expected to pass for the skeleton.

What is not built yet:
- No GUI window, editor widget, open/save flow, autosave, recovery, or markdown preview exists on `master`.
- The roadmap quality gates for Phase 1 and later are design targets, not shipped capability.
- Cross-platform behavior is CI-compiled only until real UI work lands and is manually verified.

See:
- [RIGOROUS_REVIEW.md](RIGOROUS_REVIEW.md) - internal critical planning review and action plan
- [REQUIREMENTS.md](REQUIREMENTS.md) - what must be true
- [DESIGN.md](DESIGN.md) - how the editor is intended to be built
- [ROADMAP.md](ROADMAP.md) - shipped status versus future phases

Implementation will proceed in strict phases with explicit quality gates that include code quality, test coverage, and cross-platform verification.

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

**Phase 2-3 (Polish + 2026 QOL)**
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
cargo test --all-targets --all-features
# Coverage gate used by CI
cargo install cargo-llvm-cov
cargo llvm-cov --all-targets --all-features --workspace --fail-under-lines 80
```

### Release binary size (target)

We aim for final stripped release binaries under ~8-12 MiB on all platforms through:
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
