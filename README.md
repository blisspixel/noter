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

M1 now has exact UTF-8 BOM and mixed-line-ending profiles, an external golden
byte corpus, generated round-trip properties, explicit revisions, and a
fault-injected save protocol that never confuses a failed call with a proven
non-commit. The line-ending, protocol, digest, and stable-observation slices are
verified on Windows, macOS, and Linux in GitHub Actions runs
[30177403255](https://github.com/blisspixel/noter/actions/runs/30177403255),
[30177953025](https://github.com/blisspixel/noter/actions/runs/30177953025),
[30178217482](https://github.com/blisspixel/noter/actions/runs/30178217482), and
[30178728784](https://github.com/blisspixel/noter/actions/runs/30178728784).
Private exclusive sibling creation and identity-safe cleanup are verified at
commit `d44b1ec` in
[GitHub Actions run 30179090177](https://github.com/blisspixel/noter/actions/runs/30179090177).

The production storage adapter checkpoint adds stable-handle loading,
metadata-only conflict tokens, Linux mode and extended-attribute preservation,
macOS ACL and extended-attribute transfer, native Windows and Unix commit
operations, exact post-commit reconciliation, and revision-aware Document Save
and Save As. Final links are refused, read-only destinations are not changed
implicitly, and hard-linked destinations require explicit confirmation. Its
evidence commit `c76515c` passed all Windows, macOS, Linux, strict lint, rustdoc,
documentation, and 90 percent coverage jobs in
[GitHub Actions run 30181088267](https://github.com/blisspixel/noter/actions/runs/30181088267).

A scoped M1 security review of the runtime document, storage, platform, and GUI
paths found two reportable issues in the audited revision: Windows staging files
could inherit a readable parent ACL, and document loading had no resource
ceiling. The remediation creates Windows staging files with an exact protected
owner-and-system DACL and rejects files above the explicit 64 MiB v0.1 document
limit before unbounded allocation or hashing. Windows cleanup now observes and
deletes the same open file object while denying competing writers, so neither a
rebound pathname nor a same-object write can invalidate the verified deletion.
Portable Unix cleanup cannot provide that guarantee, so failed saves and atomic
exchanges conservatively retain any private artifact and report its safe basename
plus inspection and removal guidance instead of issuing a pathname unlink.
Creation-time identity failures preserve the primary failure and a separate,
typed cleanup warning naming the retained sibling when handle-bound deletion is
unavailable. Unix existing-file metadata is captured into an immutable,
handle-ratified snapshot before commit while staging remains owner-only. After
atomic exchange, Noter verifies the displaced original's identity, bytes, and
link facts, compares its ownership, mode, ACL, and visible extended attributes
with the snapshot, and applies only an exact match to the committed handle. A
final-window metadata change leaves the committed file private and produces a
warning instead of restoring stale metadata. Noter never copies unratified
post-commit metadata. Unix extended-attribute capture, including macOS resource
forks, is separately bounded to 4,096 entries and 64 MiB of aggregate names and
values before any value allocation. macOS replays only the bounded xattr values
and a serialized immutable ACL snapshot through the destination descriptor.
A failed post-commit file barrier is reported as reduced durability. The GUI
now provides the explicit confirmation required to save one
entry of a hard-linked file. Save As confirmation retains the exact target
version observed before the dialog, so rebinding that path while the dialog is
visible produces a conflict instead of replacing the newer entry.
Unknown commit state and failed cleanup surface the safe random artifact
basename plus inspection, recovery, retry, and removal guidance.
New, Open, Quit, and native window close now fail closed while work is dirty;
M3 still owns the complete Save, Discard, and Cancel experience. The review
scope, evidence, and remaining platform gaps are recorded in
[docs/M1_SECURITY_REVIEW.md](docs/M1_SECURITY_REVIEW.md).

The original paired mutation gate is verified at commit `3830cdd` in
[GitHub Actions run 30184163737](https://github.com/blisspixel/noter/actions/runs/30184163737):
Linux-common and Windows-full both completed with zero missed mutations and zero
timeouts. The expanded gate now includes the native platform adapter and a
macOS-specific job. Its current 639-mutation supported-platform union assigns
556 to Linux, 476 to Windows, and 169 to macOS with no gap. Runner scopes
intentionally overlap on common code, and the union deduplicates exact mutation
descriptions. The historical 418-mutation
Windows core is classified as 270 caught and 148 compiler-rejected. A new local
58-mutation Windows native-adapter pass is independently clean at 40 caught and
18 compiler-rejected, with no miss, timeout, or infrastructure failure. The
expanded three-platform gate still requires one exact-commit hosted run. The
configuration, negative evidence, and exact results are documented in
[docs/M1_MUTATION_EVIDENCE.md](docs/M1_MUTATION_EVIDENCE.md).
The prototype now opens a real About dialog and visibly disables unfinished
menu commands instead of accepting no-op clicks. The dialog explains that its
project link opens in the default browser. Markdown preview, source styling,
diagnostics, and formatting remain intentionally absent until the opt-in M7
work after the trustworthy v0.1 release.

All 134 Windows-local workspace tests pass with 93.13 percent measured
trust-kernel line coverage and 90.18 percent whole-workspace line coverage. The
339-package
lockfile has a clean RustSec audit, and the current measured stripped Windows
checkpoint is 4.72 MiB. The manual metadata and filesystem matrix plus
reproducible benchmarks still gate this M1 slice. The temporary dirty-work
interlock prevents silent discard, but recovery, the complete dirty-document
decision flow, and the production UI remain unfinished, so the GUI is still a
prototype rather than a safe daily editor.

A structured adversarial design review was performed on the initial planning corpus. The review and our responses are captured in [docs/RIGOROUS_REVIEW.md](docs/RIGOROUS_REVIEW.md). That document, together with the expansions it drove (explicit safety/liveness properties, FMEA table, dependency governance, mental model alignment, stewardship planning), is the primary mechanism we are using to ensure this does not become "just another slopware text editor."

See:

- [CHANGELOG.md](CHANGELOG.md) - unreleased product and engineering changes
- [docs/RESEARCH.md](docs/RESEARCH.md) - repository audit, ecosystem research, and decisions
- [docs/BASELINE.md](docs/BASELINE.md) - measured M0 quality, coverage, size, and dependency baseline
- [docs/M1_MUTATION_EVIDENCE.md](docs/M1_MUTATION_EVIDENCE.md) - reproducible M1 mutation-testing evidence
- [docs/M1_SECURITY_REVIEW.md](docs/M1_SECURITY_REVIEW.md) - M1 security findings, remediation, and residual evidence gaps
- [docs/ROADMAP.md](docs/ROADMAP.md) - milestone order, gates, metrics, and immediate backlog
- [docs/REQUIREMENTS.md](docs/REQUIREMENTS.md) - ratified v0.1 and v0.2 product contract
- [docs/DESIGN.md](docs/DESIGN.md) - active architecture, trust protocols, verification strategy, and FMEA
- [docs/CODE-QUALITY-STANDARDS.md](docs/CODE-QUALITY-STANDARDS.md) - non-negotiable implementation, evidence, and merge gates
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
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps
cargo install cargo-llvm-cov --locked
cargo llvm-cov --locked --all-targets --all-features --workspace \
  --ignore-filename-regex 'src[/\\](app|main)\.rs$' \
  --fail-under-lines 90 --summary-only
```

Mutation evidence is platform-partitioned because an unfiltered single-platform
run can misclassify inactive target code. Use the exact runner command in
[docs/M1_MUTATION_EVIDENCE.md](docs/M1_MUTATION_EVIDENCE.md) for the platform
and scope being verified.

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
