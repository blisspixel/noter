# Noter

**A deliberately pure, reliable, cross-platform plain text editor written in Rust.**

Noter aims to be the notepad you actually want in 2026: fast, trustworthy, boring in the best way, with zero telemetry, zero bloat, and zero "we know better than you" decisions. It respects classic Notepad muscle memory while planning only the minimum modern quality-of-life improvements: system-matched light and dark themes, followed later by opt-in Markdown assistance.

> "I just want to open a file, edit text, and save it without drama, across every machine I use."

## The "Slam Dunk" Goal

The choice between modern bloated spyware slop and Noter must be an absolute slam dunk.
When you open Noter, it should not just feel "okay for a hobby app". It must feel like holding a perfectly balanced, professionally machined tool. It opens quickly. The typography is excellent. The scrolling is smooth. You know, without having to inspect hidden settings, that it will not phone home, nag you for a subscription, or discard your text. It gets in and then gets out of your way so you can write.

## Philosophy

- **Purity first.** Core experience is always plain text. No hidden formats, no "smart" rewriting of your content.
- **Reliability is non-negotiable.** Atomic saves, crash recovery, line-ending fidelity, and "never lose user data" are first-class.
- **Minimal surface area.** Small binary, small RAM, small dependency tree, small attack surface. No network calls ever.
- **System-integrated where it matters.** Follows the OS theme preference, uses native file dialogs, respects platform shortcuts, and renders a deliberately consistent interface. egui does not provide native-looking widgets, and Noter does not claim otherwise.
- **Keyboard-centric.** Every important action has a discoverable shortcut. Mouse is optional.
- **Markdown follows trust.** Plain text is the strict v0.1 scope. A later opt-in Markdown mode may add inline styling, non-mutating diagnostics, and an explicit, previewed, one-step-undoable format command. It will never rewrite a document in the background.

Noter will never become a second VS Code, a note-taking app with accounts, or a "productivity suite."

## Engineering Philosophy - How We Intend to Avoid Slopware

Your original pain points were very specific:

- The Windows 11 Notepad rewrite feels like spyware + bloat (telemetry, "modern" UI that got slower and more opinionated, OneDrive nagging, features nobody who loved the old one asked for).
- You want something that feels like the *earlier* Notepads: instant, dead simple, plain text, no surprises, keyboard native.
- Cross-platform that actually works well on Win/mac/Linux without becoming Electron garbage.
- A tiny bit of 2026 QOL (system theme + beautiful inline Markdown styling) but "keep it pretty pure" otherwise.

**Is this still a good idea in July 2026?**

Yes, *conditionally*.

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

This project has completed **M0: Truthful and Green Foundation** and is now in
**M1: Document and Durable I/O Trust Kernel**. The current branch contains an
early egui editor prototype with text entry and basic Open, Save, and Save As
flows. It is not yet safe or complete enough for daily use.

The 2026-07-25 audit found that dirty work could be discarded, invalid UTF-8
was converted silently, most menu commands were placeholders, recovery did not
exist, and the claimed Phase 1 quality gate had not been met. M0 repaired the
truthfulness and engineering foundation, including explicit invalid-UTF-8
rejection, a pinned toolchain, enforced coverage, dependency cleanup, and
warning-free CI on Windows, macOS, and Linux. The exact M0 evidence commit is
`7512534`, verified by [GitHub Actions run 30176526028](https://github.com/blisspixel/noter/actions/runs/30176526028).

A structured adversarial design review was performed on the initial planning corpus. The review and our responses are captured in [docs/RIGOROUS_REVIEW.md](docs/RIGOROUS_REVIEW.md). That document, together with the expansions it drove (explicit safety/liveness properties, FMEA table, dependency governance, mental model alignment, stewardship planning), is the primary mechanism we are using to ensure this does not become "just another slopware text editor."

See:

- [docs/RESEARCH.md](docs/RESEARCH.md) - repository audit, ecosystem research, and decisions
- [docs/BASELINE.md](docs/BASELINE.md) - measured M0 quality, coverage, size, and dependency baseline
- [docs/ROADMAP.md](docs/ROADMAP.md) - milestone order, gates, metrics, and immediate backlog
- [docs/REQUIREMENTS.md](docs/REQUIREMENTS.md) - ratified v0.1 and v0.2 product contract
- [docs/DESIGN.md](docs/DESIGN.md) - active architecture, trust protocols, verification strategy, and FMEA
- [docs/RIGOROUS_REVIEW.md](docs/RIGOROUS_REVIEW.md) - prior internal critical analysis and response

No milestone is marked complete until its tests, measurements, manual sign-off, and documentation exist on the same green commit.

## Planned Features (High Level)

**v0.1 trust and classic-notepad release**
- New, Open, Save, Save As, recent files (capped)
- Cut / Copy / Paste / Delete / Undo / Redo (excellent coalescing)
- Find + Find Next, basic Replace
- Word wrap toggle
- Status bar (line, column, selection, encoding, line endings, modified)
- System light/dark + manual override that persists
- Proper handling of line endings (preserve what was on disk)
- Durable atomic replacement plus private local crash recovery
- One tested Save / Discard / Cancel lifecycle for every destructive action
- External file-change detection
- Keyboard-only, IME, screen-reader, high-DPI, and cross-platform verification

**v0.1 production editor and polish**
- Go To Line
- Improved search (case, whole word, live match highlighting)
- A custom rope-backed editor only if a time-boxed IME and accessibility feasibility gate passes
- Font size zoom (Ctrl+wheel and menu)
- Window state persistence (position, size, maximized)

**v0.2 Markdown assist**

- Inline source styling with Markdown punctuation still visible
- Non-mutating diagnostics
- Explicit formatting with diff preview, AST-equivalence validation, and one-step undo
- No remote images, HTML execution, link fetching, or hidden rewrites

**Explicit non-goals for v0.1 and v0.2**
- Tabs or "workspaces" by default (you can launch multiple instances)
- Built-in syntax highlighting in the editor
- LSP, git integration, terminals, plugins
- Cloud sync, accounts, or any network behavior
- Themes beyond System, Light, and Dark
- Rich text editing (the file on disk is always plain UTF-8 text)

## Building & Running

### Prerequisites

- Rust 1.97.1 is the verified and pinned toolchain. Install it via [rustup](https://rustup.rs/); the repository toolchain file selects it automatically.
- On Windows: the MSVC toolchain (default via rustup on Windows).
- For full cross-platform testing you will eventually need macOS and Linux machines (or CI).

### Development

```bash
git clone https://github.com/blisspixel/noter
cd noter

# Recommended: develop outside sync folders (see .gitignore)
cargo run --release
```

Useful quality commands (these will be enforced in CI and before every phase gate):

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo install cargo-llvm-cov --locked
cargo llvm-cov --locked --all-targets --all-features --workspace \
  --ignore-filename-regex 'src[/\\](app|main)\.rs$' \
  --fail-under-lines 80 --summary-only
```

### Release binary size (target)

We aim for final stripped release binaries under about 8 to 12 MiB on all platforms through:
- `opt-level = "z"` or "3" + LTO + strip in `Cargo.toml` profiles
- Careful dependency selection
- `cargo bloat` and `cargo tree` audits

## Platform Support (Target)

| Platform          | Minimum          | Notes                              |
|-------------------|------------------|------------------------------------|
| Windows           | 10 (build 19041+) / 11 | Primary dev platform for author   |
| macOS             | 13 Ventura+      | Apple Silicon + Intel              |
| Linux             | Ubuntu 22.04 / Fedora 39+ | X11 and Wayland are release targets |

We will maintain a small manual test matrix on real hardware for each release.

## Data Safety & Privacy

- Zero network activity. The binary makes no outgoing connections.
- All state is local. Configuration contains preferences and window state, not document content.
- Versioned recovery records will live in a private per-user application state directory because the general OS temp directory is not a persistence contract.
- Recovery never silently writes the original file. Save remains an explicit operation.
- We will never read files you did not explicitly open.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).

## Acknowledgments

- The egui and eframe maintainers for making a pure-Rust immediate-mode GUI that is actually pleasant to build real tools with.
- The ropey authors for the best text rope in the business.
- Every developer who has ever been annoyed by the direction of their OS's default text editor.

---

**Noter exists because sometimes the best software is the one that gets out of your way.**

See [docs/ROADMAP.md](docs/ROADMAP.md) for how we intend to build this exceptionally well.
