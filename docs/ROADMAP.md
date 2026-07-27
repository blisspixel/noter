# Noter Roadmap

**Updated:** 2026-07-27

**Release objective:** a trustworthy, focused editor for `.txt` and `.md` files
with classic notepad ergonomics, native Markdown editing, explicit Markdown
quality tools, private local operation, and straightforward installation and
updates on Windows, macOS, and Linux.

This file defines sequence and exit criteria. Product behavior belongs in
[REQUIREMENTS.md](REQUIREMENTS.md), the Markdown experience in
[MARKDOWN.md](MARKDOWN.md), installation and update behavior in
[INSTALLATION.md](INSTALLATION.md), and implementation detail in
[DESIGN.md](DESIGN.md). Evidence belongs in the dedicated baseline, security,
mutation, benchmark, and release records.

## Status

| Milestone | Outcome | Status |
| --- | --- | --- |
| M0 | Truthful, reproducible engineering foundation | Verified |
| M1 | Document and durable I/O trust kernel | In progress |
| M2 | Usable prototype shell, themes, and update entry points | In progress |
| M3 | Editing transactions, undo, search, and text commands | Planned |
| M4 | Lifecycle, recovery, and external-change handling | In progress |
| M5 | Production editor, accessibility, and performance | Planned |
| M6 | Native Markdown editor and quality engine | In progress |
| M7 | Cross-platform distribution and first public-quality release | Planned |

`Verified` means the implementation and every named automated, manual, and
documentation artifact exist on the same green commit. Local implementation is
not verification.

## Product boundaries

- One document per window.
- Ordinary UTF-8 `.txt` and `.md` files remain the on-disk format.
- No accounts, telemetry, cloud sync, remote Markdown content, plugin system, or
  hidden document transformation.
- Native Markdown views are projections of source, not a proprietary rich-text
  model.
- System, Light, and Dark are the supported theme set for the first release.
- Update networking is explicit, release-only, and carries no document data or
  persistent identifier.

## M0: Truthful Foundation

**Outcome:** the repository builds reproducibly, states its real status, and
enforces basic quality gates.

Completed work includes the pinned Rust toolchain, warning-free CI, strict
format and Clippy checks, coverage enforcement, dependency cleanup, a coherent
source layout, and documentation that distinguishes Planned, In Progress, and
Verified. Historical M0 evidence is recorded in [BASELINE.md](BASELINE.md).

**Exit:** complete.

## M1: Document and Durable I/O Trust Kernel

**Outcome:** loading and saving preserve supported bytes and metadata, classify
conflicts and uncertain commits correctly, and fail without silently losing the
original or staged recovery information.

### Scope

- strict UTF-8 and UTF-8 BOM handling;
- exact LF, CRLF, CR, and mixed-EOL preservation;
- immutable revisions and content fingerprints;
- bounded stable-handle loading and external-version observation;
- private unpredictable same-directory staging;
- atomic platform replacement and exclusive new-file installation;
- ratified metadata transfer on Linux and macOS;
- native metadata merge and handle-bound cleanup on Windows;
- hard-link, final-link, read-only, and special-file policy;
- exact Committed, Conflict, Not Committed, and Commit State Unknown outcomes;
- file and parent durability reporting; and
- bounded metadata and document resource use.

### Current state

Most automated trust-kernel behavior is implemented. Native macOS evidence
exposed ACL-absence handling and inherited-ACL staging risk, while hosted
mutation evidence exposed repeated native decisions plus mutable line-scanner
progress arithmetic. Each defect has a focused regression. Exact-commit run
[30221793209](https://github.com/blisspixel/noter/actions/runs/30221793209)
passes native Windows, macOS, and Linux tests, both coverage gates, dependency
policy, and the complete 741-candidate supported-platform mutation union with no
miss, timeout, infrastructure misclassification, or scope gap. The reproducible
benchmark harness and required manual filesystem fixtures remain open.

Current detailed evidence and known gaps are maintained in:

- [M1_SECURITY_REVIEW.md](M1_SECURITY_REVIEW.md)
- [M1_MUTATION_EVIDENCE.md](M1_MUTATION_EVIDENCE.md)
- [BASELINE.md](BASELINE.md)
- [adr/0003-durable-replacement.md](adr/0003-durable-replacement.md)

### Exit criteria

- Exact-commit Windows, macOS, and Linux CI is green.
- Strict lint, rustdoc, dependency audit, tests, and required coverage pass.
- The complete supported-platform mutation union has no unexplained miss,
  timeout, infrastructure misclassification, or scope gap.
- Native metadata and replacement fixtures pass on the supported local
  filesystems.
- Network, cloud, removable, and weaker-filesystem behavior is measured and
  reported without overstating durability.
- Reproducible latency, memory, binary-size, and dependency baselines exist.
- Security review and public documentation match the verified commit.

## M2: Usable Prototype Shell

**Outcome:** the prototype is visually usable and truthful while deeper editor
work continues.

### Scope

- System, Light, and Dark themes with a persisted override;
- a readable classic-notepad layout with deliberate typography and spacing;
- working Help > About Noter and Help > Check for Updates actions;
- clear pre-release update status when no release is available;
- initial source-install helpers for Windows, macOS, and Linux;
- menus that execute, explain why they are disabled, or stay absent;
- platform-correct shortcuts and focus behavior for implemented commands; and
- semantic UI tests for every visible action in this milestone.

The update action must follow [INSTALLATION.md](INSTALLATION.md). It cannot imply
that a release exists when the repository has no published artifact.

### Current state

System, Light, and Dark themes, working About and update-status dialogs, source
install helpers, a responsive upper-right Mode control and Theme menu, a
contextual Markdown format bar, a bundled variable document typeface, and
deterministic native README screenshots are implemented locally. Hinting,
subpixel positioning, real heading and emphasis weights, and theme-correct text
coverage transfer are verified explicitly rather than assumed from toolkit
defaults. The early source-backed Markdown slice is visible for product
evaluation but remains governed by M6. M2 still needs installed-app semantic
automation, cross-platform visual and persistence evidence, and disposable
clean-user installer tests before verification.

### Exit criteria

- Theme behavior passes visual, high-contrast, persistence, and system-change
  tests on all target platforms.
- About and update actions are verified through semantic UI automation and the
  installed application, not only state-unit tests.
- Source install helpers are idempotent, path-safe, and tested in disposable
  standard-user environments.
- The public README remains concise and truthful.

## M3: Editing Transactions and Text Commands

**Outcome:** every edit is a reversible, revision-aware transaction with
predictable classic editor behavior.

### Scope

- a single `EditTransaction` model for typing, deletion, paste, replace,
  formatting, and EOL conversion;
- bounded undo and redo with explicit coalescing rules;
- Unicode navigation, selection, clipboard, and word-boundary behavior;
- Find, Find Next, Find Previous, Replace, and Replace All;
- Go To Line, Select All, word wrap, and zoom;
- revision-safe background work; and
- status-bar position, selection, encoding, line-ending, and modified state.

### Exit criteria

- Property and model tests prove transaction inverses and coalescing boundaries.
- Keyboard behavior is tested on Windows, macOS, and Linux.
- Long operations cannot apply stale results.
- Memory use remains bounded under long editing sessions.

## M4: Lifecycle, Recovery, and Conflicts

**Outcome:** no destructive action or crash silently discards acknowledged work.

### Scope

- one Save, Discard, and Cancel state machine for New, Open, Reload, Close, and
  Quit;
- private versioned recovery records with checksums and atomic manifests;
- startup recovery review, corrupt-record quarantine, and explicit cleanup;
- external change detection and conflict resolution;
- recent files with bounded, privacy-preserving storage; and
- multiple-instance behavior without a fragile global lock.

### Current state

Dirty New, Open, Quit, and native window-close requests now share one visible
Save, Discard Changes, and Cancel decision path. Saving continues the requested
action only after the document is clean; a cancelled or failed save retains both
the document and decision. Indeterminate-save recovery guidance survives these
decisions and Cancel, and ordinary Save remains blocked until the state is
reconciled or the user cancels the decision and chooses Save As. The pure
lifecycle reducer, Reload, recovery records, external-change handling, and
crash-fault evidence remain open.

### Exit criteria

- State-machine and crash-fault tests cover every destructive intent and save
  outcome.
- Recovery meets the documented recovery-point objective.
- Recovery never writes the original file without an explicit Save.
- Cross-instance and external-change scenarios preserve every recoverable
  revision.

## M5: Production Editor, Accessibility, and Performance

**Outcome:** the editor engine meets the responsiveness, IME, accessibility, and
large-file requirements needed by both text and native Markdown editing.

### Scope

- retain the framework editor only if it meets the measured requirements;
- otherwise introduce a rope-backed editor behind a time-boxed feasibility gate;
- incremental layout, hit testing, selection, scrolling, and bounded caches;
- IME pre-edit and candidate-window placement;
- accessibility semantics and editable-text actions;
- native shaping and fallback, stable font metrics, theme-correct coverage
  transfer, hinting, and subpixel positioning validated across display scales;
- high-DPI, high-contrast, bidirectional text, combining marks, and emoji; and
- reproducible cold-start, typing, scrolling, search, memory, and size
  benchmarks.

### Exit criteria

- Performance budgets in [REQUIREMENTS.md](REQUIREMENTS.md) pass on named
  reference systems.
- NVDA, VoiceOver, Orca, CJK IME, dead-key, emoji, high-DPI, and keyboard-only
  matrices pass.
- Any custom editor has differential tests against the reference transaction
  model and no accessibility regression.

## M6: Native Markdown Editor and Quality Engine

**Outcome:** Markdown files can be read and edited in Markdown Mode as native
formatted content while remaining ordinary, inspectable Markdown source.
Diagnostics and explicit formatting help produce portable, consistent files
without hidden rewrites.

### Scope

- Text Mode for exact source and Markdown Mode for directly editable formatted
  content, plus reading-focused and synchronized split layouts;
- direct editing of supported formatted blocks and inline constructs;
- minimal deterministic mapping from Markdown Mode operations to source
  transactions;
- accessible formatting toolbar, menus, and keyboard commands;
- CommonMark plus a ratified GitHub Flavored Markdown subset;
- stable-ID diagnostics with conservative safe fixes;
- deterministic, idempotent Format Document with a diff and supported
  semantic-equivalence check;
- restricted native rendering with inert HTML and no remote fetches;
- revision-tagged incremental parsing, diagnostics, rendering, and scroll maps;
  and
- bounded behavior for malformed, adversarial, and large Markdown files.

The normative interaction, source-preservation, formatting, privacy, and
completion contract is [MARKDOWN.md](MARKDOWN.md).

### Current state

The current vertical slice builds a borderless native layout before shaping,
uses real body, heading, and strong-emphasis weights in inactive and active
content, activates one source range for direct editing without exposing
supported delimiters, and maps eight formatting actions back to ordinary
Markdown source. A link target is revealed while it is edited and hidden again
after the caret leaves it. Text Mode always exposes exact source. Continuous
whole-document editing, shared undo transactions, complete complex-block
layout, full syntax conformance, accessibility, asynchronous parsing, and the
quality engine remain open.

### Exit criteria

- Conformance fixtures pass for every supported syntax feature.
- Switching modes without edits preserves bytes exactly.
- Markdown Mode operations are minimal, reversible, and preserve unsupported
  source.
- Formatter output is idempotent and equivalent under the supported parser
  model.
- Parser failure always leaves Text Mode accessible and saveable.
- Accessibility, IME, themes, keyboard use, and performance pass in every view.
- Runtime inspection proves no HTML execution or remote content access.

## M7: Distribution and First Public-Quality Release

**Outcome:** Noter installs, updates, runs, and uninstalls predictably on clean
supported systems, with verifiable artifacts and honest release evidence.

### Scope

- reproducible cargo-dist archives and platform packages;
- PowerShell and POSIX install scripts;
- `noter update` and Help > Check for Updates backed by one manifest policy;
- per-user installation without elevation by default;
- package-manager ownership and update behavior;
- checksums, SBOM, build provenance, and signing where credentials exist;
- clean-machine install, upgrade, rollback, portable-use, and uninstall tests;
- complete user, troubleshooting, privacy, stewardship, and release notes; and
- a minimum 14-day release-candidate dogfood period on multiple platforms.

See [INSTALLATION.md](INSTALLATION.md) for the normative installer and updater
contract.

### Exit criteria

- Every first-release requirement has traceable evidence on the exact release
  commit.
- Windows, macOS Intel and Apple Silicon, X11, and Wayland matrices pass.
- Install, same-version reinstall, upgrade, failure rollback, and uninstall pass
  as a standard user.
- Published artifacts match the manifest, checksums, SBOM, and provenance.
- No critical or high data-safety or security issue is open.
- Two people, including one non-primary developer and one non-Windows user, use
  the candidate for at least 14 days without data loss.

## Immediate backlog

1. Complete: run the settled 741-candidate supported-platform mutation union on
   one immutable commit, reconcile the evidence, and pass independent review.
   Evidence: `97371d8`,
   [run 30221793209](https://github.com/blisspixel/noter/actions/runs/30221793209).
2. Build the reproducible M1 benchmark harness and execute the remaining manual
   filesystem fixtures.
3. Complete M2 evidence for installed About and update actions, theme
   persistence, cross-platform visual behavior, and disposable source installs.
4. Begin the M3 transaction and undo model.
5. Execute the M5 editor feasibility gate, including native typography, IME,
   accessibility, display-scale, and large-file evidence. Keep the early
   block-focused Markdown slice bounded until the transaction, lifecycle, and
   production-editor contracts it depends on are stable.

This dependency order protects source fidelity and accessibility. It does not
reduce native Markdown to an optional side feature; Markdown is a required
outcome of the first public-quality release.
