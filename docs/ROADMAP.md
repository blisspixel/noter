# Noter Roadmap

**Updated:** 2026-07-31

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
| M3 | Editing transactions, undo, search, and text commands | In progress |
| M4 | Lifecycle, recovery, and external-change handling | In progress |
| M5 | Production editor, accessibility, and performance | Planned |
| M6 | Native Markdown editor and quality engine | In progress |
| M7 | Cross-platform distribution and first public-quality release | Planned |

`Verified` means the implementation and every named automated, manual, and
documentation artifact exist on the same green commit. Local implementation is
not verification.

## Next checkpoint: correctness alpha

The next product checkpoint is a dogfoodable correctness alpha, not a relabeling
of incomplete work. It requires the remaining M1 filesystem evidence,
installed-product M2 checks, completion of the ordinary M3 text
commands, and the M4 recovery and external-change safety path. M5 through M7
remain first-release work after that checkpoint.

The current implementation closes three earlier blockers: deterministic Undo
coalescing, bounded literal Find and Replace, and the pure destructive-action
lifecycle reducer. The shortest path to correctness alpha is now:

1. finish the remaining M1 manual filesystem fixtures;
2. prove About, updates, themes, and source installation in installed builds;
3. finish cross-platform navigation and clipboard policy, Markdown
   document-selection parity, and long-session M3 evidence;
4. implement private recovery records and external-change decisions through the
   M4 reducer; and
5. run the cross-platform correctness matrix on one immutable green commit.

## Product boundaries

- One document per window.
- Ordinary UTF-8 `.txt` and `.md` files remain the on-disk format.
- No accounts, telemetry, cloud sync, remote Markdown content, plugin system, or
  hidden document transformation.
- Native Markdown views are projections of source, not a proprietary rich-text
  model.
- System, Light, Dark, Green Screen, and Amber Screen are the supported built-in
  theme set for the first release.
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
progress arithmetic. Each defect has a focused regression. The historical
741-candidate supported-platform union is recorded in
[M1_MUTATION_EVIDENCE.md](M1_MUTATION_EVIDENCE.md).

The latest verified implementation checkpoint is commit `08fd8a5`.
Exact-commit workflow-dispatch run
[30702655806](https://github.com/blisspixel/noter/actions/runs/30702655806)
passes all nine Windows, macOS, Linux, documentation, dependency, coverage, and
mutation contexts for exact commit
`08fd8a5e074da6a88e12e5fcc9c7908d148b088c`. Hosted Linux line coverage is
93.02 percent for the workspace and 94.36 percent for the trust kernel. The
current platform mutation scopes report 970 Linux candidates, 939 Windows
candidates across two required shards, and 47 macOS candidates, with no miss or
timeout. The infrastructure validator reports no recognized tool, compiler,
linker, process, or storage failure hidden as unviable. These scopes overlap and
are not claimed as a newly deduplicated cross-platform union.

The schema-v2 reproducible benchmark harness is implemented at commit
`580f164`. A 30-sample Windows reference from that clean detached commit records
all raw latency and peak-working-set samples, the exact deterministic corpus,
release binary size and hash, and a four-target dependency summary. Its
self-reported local provenance and complete limitations are recorded in
[M1_BASELINE_EVIDENCE.md](M1_BASELINE_EVIDENCE.md). Exact-head hosted validation
and the remaining platform fixtures remain open.

A 2026-07-31 local fixture record against source bytes later committed
unchanged as `65ac25f` now covers native NTFS replacement, new-file privacy,
and read-only failure; native WSL2 ext4
replacement and metadata retention; and the Windows-to-WSL UNC boundary. The
bridge exposed a broader-than-owner Linux mode, and the repaired source now
fails closed before writing document bytes when the requested Windows owner and
DACL cannot be verified. [M1_FILESYSTEM_EVIDENCE.md](M1_FILESYSTEM_EVIDENCE.md)
records exact checksums, classifications, provenance, and limitations. Native
macOS, SMB, cloud-synchronized, removable, weak-filesystem, second-identity,
and crash-persistence fixtures remain open.

Exact clean-detached mutation validation at `994e0a3` closes three token-length
boundary survivors exposed by the first focused private-security campaign. The
settled campaign catches all 20 candidates with no unviable, missed, or timed-
out result, and its infrastructure validator passes. The
[machine-readable record](evidence/m1-windows-private-security-mutation-2026-07-31.json)
binds the commands, source trees, candidate set, equivalence exclusion,
outcomes, and local artifact hashes. That `994e0a3` checkpoint passes 425
Windows-local workspace tests with 93.49 percent whole-workspace, 95.23 percent
trust-kernel, and 92.14 percent platform-adapter line coverage.

Current detailed evidence and known gaps are maintained in:

- [M1_SECURITY_REVIEW.md](M1_SECURITY_REVIEW.md)
- [M1_MUTATION_EVIDENCE.md](M1_MUTATION_EVIDENCE.md)
- [M1_BASELINE_EVIDENCE.md](M1_BASELINE_EVIDENCE.md)
- [M1_FILESYSTEM_EVIDENCE.md](M1_FILESYSTEM_EVIDENCE.md)
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

- System, Light, Dark, Green Screen, and Amber Screen themes with a persisted
  override;
- a fail-closed declarative theme extension boundary;
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

System, Light, Dark, Green Screen, and Amber Screen themes, working About and
update-status dialogs, source install helpers, a responsive upper-right Mode
control and Theme menu, a contextual Markdown document bar, a bundled variable
document typeface, and a deterministic five-capture native README set are
implemented locally. That set pairs the same Light document in Text and
Markdown modes and gives Dark, Green Screen, and Amber Screen their own Markdown
evidence. Specialty palettes have deterministic enhanced-contrast checks.
Standard Light and Dark error text now uses palette-owned colors with automated
4.5:1 checks against the actual panel and window surfaces. File command state
also reflects document state: Reload is unavailable for an untitled document.
Built-in palette extension is data-driven and invalid palettes fail closed to
Dark. The safe custom-theme contract is documented;
runtime theme-file loading, persistence, and error UX remain open work.
Hinting, subpixel positioning, real heading and emphasis weights, and
theme-correct text coverage transfer are verified explicitly rather than
assumed from toolkit defaults. The early source-backed Markdown slice is visible
for product evaluation but remains governed by M6. M2 still needs installed-app
semantic automation, cross-platform visual and persistence evidence, and
disposable clean-user installer tests before verification.

### Exit criteria

- Theme behavior passes visual, high-contrast, persistence, and system-change
  tests on all target platforms.
- Declarative custom-theme rejection passes malformed-data and external-access
  tests.
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

### Current state

The first M3 foundation is implemented. `Document` accepts one atomic,
revision-checked `EditTransaction` authority with ordered UTF-8 byte ranges,
exact expected removals, exact inverses, directional before and after
selections, origin, and adapter-supplied monotonic time. Text Mode, direct
Markdown edits, and Markdown formatting actions route through that authority.
Undo and Redo are available from the Edit menu and platform keyboard paths.
History is bounded to 1,024 transactions and 32 MiB of retained source by
default, clears stale branches defensively, and restores saved-content identity
without reusing revisions.

Direct edits now carry conservative Insert, Backspace, Delete, Replace
Selection, Paste, Formatting, Replace, conversion, or programmatic intent.
Adjacent typing, Backspace, and forward Delete coalesce independently inside a
750 millisecond and 16 KiB boundary; time regression, selection movement,
origin change, intent change, non-adjacency, paste, formatting, and resource
ceilings end the group. Text and Markdown adapters identify paste explicitly.

A non-modal Find and Replace bar now provides bounded literal queries, Unicode
case matching, next and previous navigation, wrap reporting, match counts, and
explicit selection or whole-document Replace All scope. Query and replacement
input payloads are bounded before focused widgets process text, paste, or IME
commits, and an exact mutation-boundary byte ceiling covers Enter, Tab,
selection changes, and replacements.
Search caches are revision-keyed and retain counts rather than a document-sized
match vector. Replacement calculates its BOM-aware bounded result before
allocation and enters shared Undo as one explicit Replace transaction. Find
field Undo remains local, and closing the bar restores immediate document input.
The status bar reports modified state, one-based logical line and Unicode-scalar
column, and selection size from a revision-and-selection keyed cache.

Text and Markdown modes now include Select All. A valid directional selection
can move from Text Mode into one contiguous source-backed Markdown edit region
across parsed blocks without changing UTF-8 bytes or native and mixed line
endings. Text Mode includes validated allocation-free Go To Line across LF,
CRLF, CR, and mixed files plus persistent word wrap. Go To Line input is
bounded before widget processing. Document-only zoom is available in both modes
from 50 to 300 percent through keyboard, menu, and supported pointer gestures
without scaling application controls or changing source bytes. At the 420-pixel
minimum width, a compact More menu keeps Edit, View, and Help commands pointer
reachable. Markdown Mode now uses the complete continuous borderless canvas
with only the ordinary editor inset, deliberate vertical rhythm, and a
document-bar zoom cluster wired to the same bounded state. The live percentage
accepts vertical pointer-wheel zoom and remains a click target for reset.
Formatted content remains wrapped by design. Cross-block pointer dragging in
inactive Markdown content and the remaining clipboard and navigation parity are
still open.

Escape now commits same-frame Markdown input before leaving the active range
and carries the final directional source selection into shared history. Go To
Line discards its document-specific input and focus state when a document is
replaced or the application leaves Text Mode.

Deterministic 512-case properties cover single replacements, ordered disjoint
multi-edit transactions, arbitrary edit sequences, literal search, and
lifecycle decisions against independent reference models. The
[exact-commit M3 editing record](M3_EDITING_EVIDENCE.md) reports 256 generated
mutations: 216 caught, 40 compiler-unviable, zero missed, and zero timed out,
with no recognized infrastructure failure. This is focused Windows-local
evidence, not M3 completion. Clipboard and complete navigation policy,
revision-safe background indexing, cross-platform manual evidence, and the
long-session memory fixture remain open.

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

Dirty New, Open, Reload, Quit, and native window-close requests now share one
pure `LifecycleState` reducer and one visible Save, Discard Changes, and Cancel
decision path. Explicit Prompting, Saving, and Closing phases bind every intent,
save completion, and close authorization to the exact document revision that
created it. Repeated requests cannot replace the active intent, and stale or
unsolicited completions cannot authorize destructive work. A cancelled, failed,
or still-interactive save preserves the document and returns to a safe explicit
decision. Indeterminate-save recovery guidance survives these decisions and
Cancel. Independent in-memory records retain every unresolved destination and
instruction instead of allowing a later Save As to replace earlier evidence.
An indeterminate outcome stops every Save and Save As before destination work
until the user explicitly reconciles each record. New, Open, and notice
dismissal never release the block. A confirmation removes one record without
writing or retrying. Each bounded record reserves its vector slot, selected
destination, 1-KiB display label, and 4-KiB diagnostic before mutation; encoded
paths above 128 KiB are refused before save work, the ledger retains at most 16
records, and the scroll-bounded surfaces expose the destination plus an explicit
path-copy action. Non-Unicode paths use a labeled reversible hexadecimal
operating-system representation rather than lossy replacement text. The
confirmation repeats the diagnostic and path action; removing the last record
clears only its stale block error. Save availability is a constant-time
in-memory decision with no repaint-time filesystem inspection. Exhaustive
transition tests and a
fixed-seed 512-case command-sequence property compare the reducer with an
independent model. Durable restart-spanning recovery records, external-change
handling through the reducer, and crash-fault evidence remain open.

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
- high-DPI, high-contrast, bidirectional text, combining marks, and emoji;
- an optional local spell-check adapter with explicit language and enablement,
  no document upload, no background network access, and a clean unavailable
  state on platforms without a supported local provider; and
- reproducible cold-start, typing, scrolling, search, memory, and size
  benchmarks.

### Current state

The trust-kernel loader remains bounded at 64 MiB, but the current egui editor
mirrors the complete document as a `String`. A local Windows measurement found a
665.3 MiB process peak when a 64 MiB file reached that widget path. The current
interface therefore refuses files above 8 MiB before creating the mirror,
preserves the open document, and explains the limit. The same 64 MiB run then
peaked at 196 MiB without entering the editor. This is defensive containment,
not M5 completion; the release still requires a measured 50 MiB editable path.
The current bundled Inter configuration retains egui's complete default
fallback chain, including its emoji fonts, so pasted Unicode remains intact.
The current renderer's emoji output is monochrome and is not accepted as final
cross-platform appearance evidence. No spell-check provider is implemented.

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
  content as the required editing surfaces;
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
supported delimiters, and maps one paragraph-style selector with Paragraph and
all six ATX heading levels, plus six formatting actions, back to ordinary
Markdown source. Paragraph and heading choices are exact idempotent styles;
the remaining actions are selection-aware toggles. Paragraph styles use
parser-verified top-level blocks and fail closed for code, setext headings,
nested blocks, unsupported structures, and paragraph content that cannot round
trip through ATX syntax byte-exact, including ambiguous leading whitespace and
trailing closing-style hashes. Controls avoid invented link text and targets,
expose accessible current or pressed state, and provide focused
keyboard paths for Bold, Italic, and Link. The permanent non-modal formatting
bar has no Done state; Escape returns a focused active range to rendered form
after synchronizing its pending edit. Click-and-drag selection maps
rendered characters to complete source spans, including hidden delimiters,
escapes, and supported character
references; synthesis without a safe mapping falls back to visible source. A
Text Mode selection may span parsed blocks and restores as one contiguous
source-backed active region with its direction and line endings intact. Select
All uses the same path in either view. Pointer dragging across separate inactive
blocks remains open.
link target is revealed while it is edited and hidden again
after the caret leaves it. Text Mode always exposes exact source. Shared bounded
Undo and Redo use the deterministic intent and coalescing policy. Continuous
whole-document editing, complete complex-block layout, full syntax conformance,
accessibility, asynchronous parsing, and the quality engine remain open.

The current synchronous formatted slice enforces explicit source-byte, line,
line-length, block-count, block-span, and parser-event ceilings. Over-budget
Markdown files stay unchanged in Text Mode when they remain within the current
8 MiB interactive-file ceiling; larger files are refused without replacing the
open document. Diagnostic counts are cached by document generation and
revision. M5 and M6 still own incremental parsing, virtualized layout, and the
measured final limits.

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
- parity-verified consolidation of release-critical Python automation into a
  Rust `xtask`, beginning with validators and generators and retaining mature
  platform benchmark tooling until a Rust replacement is demonstrably simpler;
- clean-machine install, upgrade, rollback, portable-use, and uninstall tests;
- complete user, troubleshooting, privacy, stewardship, and release notes; and
- a minimum 14-day release-candidate dogfood period on multiple platforms.

See [INSTALLATION.md](INSTALLATION.md) for the normative installer and updater
contract.

### Current state

The source installers already build the locked checkout and verify the installed
CLI contract on Windows, macOS, and Linux. A pinned cargo-dist plan now produces
four target archives, PowerShell and POSIX installers, Homebrew and MSI
packaging, checksums, four target-specific CycloneDX 1.5 SBOMs, and GitHub
attestations. The SBOMs are declared cargo-dist artifacts, so the generated
release manifest names the same four files that the workflow builds and
publishes. Every binary package includes the generated third-party dependency
inventory and bundled-font license. Publication is restricted to a prerelease
tag on the protected `main` tip, rechecks that tip immediately before atomic tag
creation, and remains an explicit human decision. Native signing, clean-machine
artifact tests, updater authentication, platform evidence, and the required
dogfood period remain open.

The static musl target is deferred until the release inventory can account for
its non-Cargo runtime and ship the corresponding notices and SBOM evidence.

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
2. Complete: build the reproducible M1 benchmark harness and record the
   canonical 30-sample Windows reference. Evidence:
   [M1_BASELINE_EVIDENCE.md](M1_BASELINE_EVIDENCE.md).
3. Partially complete: native NTFS and WSL2 ext4 fixtures pass, and the
   Windows-to-WSL boundary now fails closed. Continue the remaining native
   macOS, SMB, cloud, removable, weak-filesystem, second-identity, and
   crash-persistence fixtures without overstating unavailable evidence.
   Evidence: [M1_FILESYSTEM_EVIDENCE.md](M1_FILESYSTEM_EVIDENCE.md).
4. Complete M2 evidence for installed About and update actions, theme
   persistence, cross-platform visual behavior, and disposable source installs.
5. Complete the remaining M3 navigation, clipboard, Markdown document-selection,
   background-work, cross-platform keyboard, and long-session requirements atop
   the implemented Undo, Find and Replace, Go To Line, wrap, and zoom foundation.
6. Complete M4 recovery records and external-change decisions through the pure
   lifecycle reducer, including fault and stale-effect evidence.
7. Execute the M5 editor feasibility gate, including native typography, IME,
   accessibility, display-scale, and large-file evidence. Keep the early
   block-focused Markdown slice bounded until the transaction, lifecycle, and
   production-editor contracts it depends on are stable.

This dependency order protects source fidelity and accessibility. It does not
reduce native Markdown to an optional side feature; Markdown is a required
outcome of the first public-quality release.
