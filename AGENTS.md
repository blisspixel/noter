# Repository Working Agreement

Noter is a local, private, cross-platform editor for ordinary `.txt` and `.md`
files: a native egui desktop window plus a terminal mode (`noter --tui`), one
document per window. Its product promise is data safety and privacy first,
editing quality second, and features last.

## Product law

Keep Noter focused, private, defensive, and cross-platform. Do not add accounts,
telemetry, advertising, cloud document formats, background network access,
plugin systems, or bundled AI. Update networking is explicit, release-only, and
carries no document data or identifier. Preserve ordinary text and Markdown
source byte for byte: Markdown Mode is a projection of source, never a second
document model. Never weaken data safety to simplify an interface. Scope changes
to these boundaries need an explicit product decision recorded in
`docs/REQUIREMENTS.md` and `docs/ROADMAP.md` first.

## Orient before changing behavior

Read the root README, `docs/ROADMAP.md`, the relevant product contract, and
`docs/CODE-QUALITY-STANDARDS.md`. `docs/README.md` routes to every other
document. Each topic has one home: requirements in `docs/REQUIREMENTS.md`,
sequence and status in `docs/ROADMAP.md`, architecture in `docs/DESIGN.md`,
costly decisions in `docs/adr/`, measured evidence in the named evidence
records and `docs/evidence/`, and user-visible changes in `CHANGELOG.md`.

Source, tests, `Cargo.toml`, `Cargo.lock`, CI, and git history outrank prose.
Docs in this repository have claimed unbuilt features before, so confirm any
capability in code before relying on or repeating it. Keep planned,
implemented, tested, released, and verified distinct; a roadmap item is Verified
only with same-commit evidence named by its exit criteria.

## Architecture and canonical seams

- `src/core/` is the UI-free trust kernel in the library crate. It owns the
  document (`ropey` rope), revisions, line endings, observation, conflict, save,
  recovery records, edit transactions, undo, search, navigation, lifecycle, and
  limits.
- `src/app.rs`, `src/markdown_ui.rs`, `src/bounded_text_input.rs`, and the other
  top-level `src/*.rs` UI files adapt user intent to the core. They must not
  re-decide trust-kernel policy.
- `src/tui/` is the terminal front end. It must reach the same core seams as
  the GUI rather than growing parallel ones.
- `crates/noter-platform/` is the only home for operating-system primitives and
  `unsafe` code. The application crate forbids `unsafe`.

One way to do each important thing:

- Every text change is an `EditTransaction` (`src/core/edit.rs`) applied through
  `Document`; history is `core::undo::UndoHistory`. Do not keep a separate undo
  stack or write the rope directly.
- Every save goes through the `Document` save path (`src/core/save.rs`,
  `src/core/fs_storage.rs`) and must surface Committed, Conflict, Not Committed,
  and Commit State Unknown distinctly. Never write document bytes with `std::fs`
  directly, and never treat a non-Committed outcome as permission to discard
  work.
- Crash recovery goes through `src/crash_recovery.rs` and
  `src/core/recovery_store.rs`.
- Markdown structure comes from `pulldown-cmark`; do not add a second parser.
- Untrusted text written to a terminal (CLI values, paths, file names, document
  content) must not emit raw C0, C1, bidi-control, or line-separator
  characters. The CLI escapers live in `src/main.rs`; when another caller needs
  one, promote a single shared implementation instead of writing a second.

Before adding a module, dependency, or helper, search for an existing one. New
runtime dependencies need a rationale and must pass `deny.toml`; no network
client crates. Prefer extracting shared logic from the large UI files over
adding to them.

## Verify

The merge contract and full command list live in
`docs/CODE-QUALITY-STANDARDS.md`, and `.github/workflows/ci.yml` is the exact
gate. Rust is pinned by `rust-toolchain.toml`. The usual local loop is:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps
python3 scripts/check_doc_links.py
python3 -m unittest discover -s scripts -p "test_*.py"
```

CI additionally runs `cargo audit`, `cargo deny`, ruff on `scripts/`, the
README asset and release-config checks, 80 percent whole-workspace and 90
percent trust-kernel line coverage (`cargo llvm-cov`), native Windows, macOS,
and Linux tests, and sharded `cargo-mutants` campaigns over `src/core/` and
`crates/noter-platform/`. Work in a loop: implement, run the gates, fix root
causes, rerun, then review your own diff adversarially before calling it done.

Never make a gate pass by weakening it: no new `#[allow]`, Clippy allow,
`.cargo/mutants.toml` exclusion, coverage ignore pattern, lowered threshold,
removed assertion, or test rewritten to accept wrong behavior unless the
exception is narrow, justified in a comment, and reviewed. Excluding a whole
subsystem from mutation or coverage is not narrow.

## Evidence a change needs

- Every behavior change comes with the smallest test that fails without it.
  Byte fidelity needs golden or property tests; save, recovery, and conflict
  paths need failure-path assertions and must stay inside the mutation scope.
- Edits to input handling, rendering of untrusted text, or anything indexed by
  byte offset need tests with multibyte, wide, and control characters.
- Performance claims need measurements (`benches/`, `scripts/run_m1_baseline.py`).
- UI changes need regenerated Light and Dark screenshots and visual review; see
  `docs/DEVELOPMENT.md`.
- Filesystem, recovery, security-sensitive, and release changes need the native
  fixture, review, and evidence records defined in the quality standards and
  roadmap, plus an independent fresh-context review before merge.

## Keep project state current

A change is finished when the artifacts it makes true are updated: tests,
`CHANGELOG.md` under Unreleased, roadmap status, affected contracts, and an ADR
for a costly-to-reverse decision. Remove or correct stale claims you encounter
in the area you touch. When the same mistake recurs, add a test, lint, or check
that catches the class rather than another warning here.

Temporary agent state (scratch plans, indexes, mutation output, local receipts)
belongs in the gitignored `.agent/` directory. Never put secrets there, and
promote anything durable into tracked docs, tests, or source.

## Code and writing style

Use the existing source layout. Avoid duplicated domain logic, placeholders,
TODOs, dead code, and commented-out implementations. Keep comments accurate and
limited to invariants, platform contracts, and decisions the code cannot express.
Repository text, comments, commit messages, and PR descriptions are concise and
technical, with no emojis, no em or en dashes, and no tool, model, or AI
attribution of any kind (including co-author trailers and generated-by lines).

## Git and releases

Keep one protected `main` branch as the integration branch. Work on a branch,
keep each change narrow, stage only the intended files, and require exact-head
CI before merge. Do not commit local automation state, logs, build output,
secrets, or release credentials. Tagging, publishing, and version bumps are
release decisions that follow `docs/RELEASING.md` and require the checkpoint's
evidence; never perform them as a side effect of other work.
