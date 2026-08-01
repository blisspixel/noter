# Noter Architecture and Product Review

**Original review:** June 2026

**Disposition reviewed:** 2026-07-30

**Status:** Current ranked review plus historical findings

This document preserves the useful challenges from the first adversarial review
and tracks their current disposition. Requirements, design, roadmap gates,
tests, benchmarks, and release records remain authoritative.

## 1. Executive finding

The original planning showed unusual care for a small editor but confused
ambition with proof. It named atomic saves, recovery, large-file performance,
undo fidelity, Markdown formatting, and cross-platform quality before defining
their state machines, commit points, failure models, or evidence.

The central corrective action is:

> Every trust or performance claim must identify an invariant, implementation
> boundary, failure case, and artifact that can falsify it.

The durable-save trust kernel currently has substantially more evidence than
the rest of the application. That imbalance matters: a complete mutation
campaign protects the save protocol while the transaction, Undo, literal
search, and lifecycle foundations are newer. The editor still lacks recovery,
external-change handling, incremental layout, and release-grade accessibility
evidence. Coverage cannot compensate for missing product architecture.

## 2. Current assessment

Noter is an active development build, not yet a public-quality release. The
strongest current evidence covers explicit save failure states, native save
semantics, dependency controls, reversible editing, deterministic Undo
coalescing, bounded literal search, and the destructive-action reducer.
Recovery, external changes, performance, accessibility, Markdown conformance,
installation, and updates remain below the release bar because their contracts
are not yet fully implemented and tested on supported systems.

The next investment is recovery and external-change safety through the shared
lifecycle foundation, while completing ordinary text commands and the
production-editor feasibility evidence. The completed slices are meaningful
progress, not evidence that the full editor contract is complete.

## 3. Ranked improvement program

### 1. Establish one transaction-based source authority

**Evidence:** `src/core/edit.rs` now validates revision, ordered UTF-8 ranges,
exact expected removals, directional before and after selections, and the
resulting selection before atomically replacing `Document` content. It returns
an exact inverse. `src/core/undo.rs` bounds history by count and retained bytes,
rejects unexpected revisions, and maintains monotonic revisions. Both framework
editing modes route changes through this authority. Three fixed-seed 512-case
properties compare single, ordered multi-edit, and Undo and Redo sequences to a
`String` model.

`NoterApp` still uses a mutable contiguous `String` as the framework adapter,
and `MarkdownEditor` still owns one contiguous active source draft, which may
span parsed blocks for Select All or a selection carried from Text Mode. Direct
input is now classified conservatively, paste is explicit, and typing,
Backspace, and forward Delete coalesce under bounded deterministic rules.
Literal Find and Replace also enter the same transaction history. Both views
now have Select All, while Text Mode has mixed-EOL Go To Line and byte-preserving
word wrap; bounded editor zoom works in both modes.

**Risk:** the common model now protects source mutation, inverse history, saved
content identity, selection direction, and common edit grouping. The remaining
adapter copy, navigation and clipboard policy, and synchronous full-source work
can still produce performance or platform-behavior gaps until the production
editor gate is complete.

**Completion standard:** Implement a revision-checked `EditTransaction` with
validated UTF-8 boundaries, exact inverse operations, before and after
selections, origin, and bounded byte cost. Route typing, deletion, paste,
replace, EOL conversion, and Markdown commands through it. Prove inverse and
coalescing properties against a simple reference model.

### 2. Complete recovery and conflict handling through the lifecycle reducer

**Evidence:** `LifecycleState::reduce` now owns explicit Prompting, Saving, and
Closing phases for New, Open, Reload, or Quit. Each phase binds its intent,
completion, and native-close authorization to an exact document revision.
Save, Discard, Cancel, repeated requests, stale and unsolicited completions,
dirty save outcomes, pending hard-link confirmation, and blocking post-save
warnings pass exhaustive unit cases and a fixed-seed 512-case model property.
Versioned recovery records and external-change effects remain absent.

**Risk:** recovery and external observation can still lose coherence if they
bypass the reducer or accept stale revision effects. The current reducer is a
safe control point, not the recovery implementation itself.

**Completion standard:** Carry revision and operation correlation through every
recovery and external-observation effect, then implement private checksummed
recovery records, startup review, quarantine, external-change decisions, and
crash-fault tests.

### 3. Pass the production-editor feasibility gate before expanding features

**Evidence:** Text Mode uses egui `TextEdit` over a complete `String`.
Markdown Mode synchronously finds all block ranges and builds every rendered
block inside a non-virtualized scroll area. The authoritative `Rope` therefore
does not provide incremental display behavior.

**Risk:** large documents and pathological lines can perform work proportional
to the complete document on input and paint. More features added to this path
increase the cost of replacing it.

**Immediate containment:** Markdown projection rejects work above explicit
source-byte, logical-line, line-length, block-count, block-span, and parser-event
ceilings. The framework-backed interface refuses files above 8 MiB before
constructing its complete widget string and preserves the already open document.
A measured Windows run reduced the 64 MiB case from a 665.3 MiB process peak to
196 MiB without entering the editor. This bounds the observed amplification but
does not satisfy the 50 MiB feasibility gate.

**Completion standard:** Time-box the documented editor spike. Demonstrate
rope-backed edits, visible-row layout, bounded caches, long-line behavior, hit
testing, selection, IME pre-edit, candidate placement, and AccessKit actions.
Retain the framework adapter until measured parity exists.

### 4. Make Unicode, IME, and accessibility executable contracts

**Evidence:** the requirements name grapheme, word, bidirectional, IME, and
screen-reader behavior, but current automation primarily inspects menu labels
and bounds. The manual matrix remains the only planned evidence for real NVDA,
VoiceOver, Orca, and CJK input.

**Risk:** byte-correct storage can still produce a text editor that corrupts
composition, splits user-perceived characters, or is unusable without sight or
a mouse.

**Completion standard:** Define navigation and deletion against a declared
Unicode text segmentation profile, add generated conformance data, expose
editable text and selection actions semantically, and pass real IME and
screen-reader matrices on Windows, macOS, X11, and Wayland.

### 5. Replace the Markdown slice with a conformance-driven semantic model

**Evidence:** the current projection reconstructs block ranges around
`pulldown-cmark` events, reparses individual blocks for presentation decisions,
and supports eight local formatting commands plus four diagnostics. It has no
complete CommonMark or GFM corpus, whole-document formatter, or
semantic-equivalence proof.

**Risk:** ad hoc source transformations can change meaning or damage unsupported
syntax while appearing visually correct.

**Completion standard:** Ratify the dialect, run the complete applicable
conformance corpus, map semantic nodes to stable source ranges, preserve
unsupported constructs, and make Format explicit, idempotent, diff-reviewed,
byte-policy-preserving, and equivalent under the supported parser model.

### 6. Build reproducible performance evidence, not performance adjectives

**Evidence:** the repository now has a schema-validated deterministic M1
trust-kernel harness and a canonical 30-sample Windows record with raw latency,
peak-memory, binary-size, dependency, corpus, and environment evidence. The
record is self-reported local evidence, not authenticated telemetry, cross-
platform comparison, or M5 GUI and input evidence.

**Risk:** current full-document copies, parsing, diagnostics, and layout can
regress without a gate even while all functional tests pass.

**Completion standard:** Validate the committed harness and record at exact
head, add the supported-platform comparisons, then complete the separate M5
startup, input, scroll, Markdown-frame, and interactive-memory measurements.

### 7. Centralize commands, effects, enabled state, and shortcuts

**Evidence:** file commands have a local dispatch path, while view, theme, help,
dialogs, and editor actions mutate application state through separate methods.
`src/app.rs` remains the combined shell, coordinator, dialog controller, and
large test host instead of the target `src/ui/` and `src/platform/` boundaries.

**Risk:** platform shortcuts, menu labels, accessibility metadata, enabled
state, and help text can drift. State-dependent commands are difficult to model
or replay.

**Completion standard:** Derive every visible action from one typed command
descriptor and pure reducer, keep native effects behind narrow adapters, then
split UI modules along those proven boundaries rather than by arbitrary file
size.

### 8. Automate the installed product on every supported platform

**Evidence:** unit and native adapter coverage are strong, but installed-app
semantic verification is not a repeatable tracked suite. The roadmap still
requires cross-platform visual, theme-persistence, dialog, keyboard, and clean
installer evidence.

**Risk:** a green library can ship an inert menu, wrong platform shortcut,
broken focus order, inaccessible dialog, or installer that only works on a
developer machine.

**Completion standard:** Run semantic installed-binary workflows for every
visible action on Windows, macOS, X11, and Wayland, retain artifacts, and pair
them with signed manual evidence for behaviors automation cannot establish.

### 9. Ship a real distribution and secure update system

**Evidence:** current helpers compile from source, the update command opens a
truthful status dialog, and a manually dispatched cargo-dist workflow now plans
archives, installer scripts, Homebrew, MSI, checksums, an SBOM, and GitHub
attestations. The workflow has not produced or published a supported release,
and there is no authenticated update manifest, rollback implementation,
platform signing evidence, or clean-system uninstall record.

**Risk:** source installation is high friction, while a naive self-updater would
create a more serious supply-chain risk than having no updater.

**Completion standard:** Produce reproducible prebuilt artifacts and per-user
installers, publish checksums, SBOMs, attestations, and signatures where
available, and use one bounded manifest policy for the menu and CLI. Defend
artifact authenticity, rollback, freeze, mix-and-match, interruption, and
package-manager ownership.

### 10. Close the last evidence gaps and calibrate quality claims

**Evidence:** native CI, coverage, dependency policy, and the mutation union are
extensive. M1 still lacks weak-filesystem and manual metadata fixtures; release
criteria still require clean systems, multiple platforms, and a 14-day
multi-user candidate period. Internal rubric scores should not be mistaken for
release evidence.

**Risk:** strong automated evidence for selected properties can create false
confidence about properties that were never exercised.

**Completion standard:** Maintain a requirement-to-evidence ledger on the exact
commit, record residual risks, complete native filesystem and clean-machine
matrices, reproduce releases, and require the stated dogfood period before
calling v0.1 public-quality.

## 4. Historical findings and current disposition

### R-01 Save was described too loosely

**Finding:** "Write, sync, rename" did not define destination identity races,
metadata, symlinks, platform replacement, parent-directory durability, or a
post-commit sync failure.

**Disposition:** Accepted and substantially implemented.
[ADR-0003](adr/0003-durable-replacement.md) requires an injected storage
boundary, explicit commit outcomes, pre-commit revalidation, platform semantics,
and fault evidence. Its remaining manual filesystem evidence is tracked under
M1; the trust-kernel benchmark record now exists while M5 GUI and input
performance evidence remains open.

### R-02 Recovery was not a protocol

**Finding:** A temporary file and timer did not define ownership, schema,
integrity, retention, multiple instances, corruption, or failure behavior.

**Disposition:** Accepted in design, not implemented. The recovery contract
requires private per-user application state, versioned records, random
identities, revisions, checksums, atomic manifests, bounded scheduling,
quarantine, and a controlled crash harness under M4.

### R-03 Close behavior contradicted the classic mental model

**Finding:** Frictionless close backed only by recovery made Save ambiguous and
could turn cache retention into hidden document storage.

**Disposition:** Partially implemented. Dirty New, Open, Reload, Close, and Quit
use one pure reducer and one Save / Discard / Cancel decision path. Recovery and
external-change effects remain M4 work and cannot authorize silent close.

### R-04 Encoding and EOL fidelity lacked a mixed-file model

**Finding:** A single line-ending enum could not represent mixed-EOL content.
Lossy UTF-8 loading contradicted byte fidelity.

**Disposition:** Accepted. [ADR-0002](adr/0002-encoding-and-line-endings.md)
requires strict UTF-8, exact existing newline bytes, a mixed profile, local
insertion rules, and explicit undoable conversion.

### R-05 Undo requirements were prose, not a model

**Finding:** Coalescing examples did not prove that edits, selections, and
inverse operations preserve information.

**Disposition:** Partially implemented. Revision-tagged transactions, exact
inverses, bounded history, directional selections, deterministic coalescing,
literal Find and Replace, and reference-model properties now exist. M3 remains
open for navigation, clipboard, remaining commands, and platform evidence.

### R-06 Large-file claims were technically unsupported

**Finding:** Memory mapping does not make an owned rope lazy, and opening 500 MB
inside one frame was not credible.

**Disposition:** Accepted. v0.1 has measured 1 MiB and 50 MiB budgets. A 500 MB
editor guarantee is deferred. The custom renderer must pass a feasibility gate
covering long lines, bounded layout, IME, accessibility, and latency.

### R-07 Accessibility and IME were treated as testing details

**Finding:** A custom text editor owns candidate placement, pre-edit state,
selection semantics, screen-reader text exposure, and platform editing actions.
Keyboard smoke tests cannot prove these.

**Disposition:** Accepted. The architecture makes IME and accessibility editor
contracts. M5 cannot pass without real IME and screen-reader evidence.

### R-08 Markdown scope hid mutation risk

**Finding:** A "Ruff for Markdown" can change meaning through heading, list,
table, whitespace, or extension decisions. Regex formatting was unsafe.

**Disposition:** Accepted. The first public-quality release includes Markdown
after its text, transaction, lifecycle, and accessibility prerequisites. Text
Mode remains exact source, Markdown Mode is a directly editable source-backed
projection, diagnostics are non-mutating, and Format requires a reviewed diff,
parse equivalence, EOL/BOM preservation, confirmation, and one-step undo.

### R-09 Dependency selection lacked governance

**Finding:** "Latest stable" did not assess maintainer health, capabilities,
build scripts, native code, transitive size, duplicate versions, or removal
cost.

**Disposition:** Accepted. Every direct dependency now requires a requirement,
feature, license, maintenance, advisory, capability, duplicate, size, and exit
review. Releases add SBOM and provenance evidence.

### R-10 Coverage risked becoming a vanity metric

**Finding:** A high percentage could consist of happy paths while save,
recovery, undo, and UI failure behavior remained untested.

**Disposition:** Accepted. Coverage remains a gate, but critical behavior also
requires property tests, reference models, fault injection, mutation testing,
semantic UI tests, crash tests, and signed manual matrices.

### R-11 Documentation claimed phases complete without evidence

**Finding:** The repository called an early GUI prototype a completed daily-use
phase even though dirty actions discarded content and visible commands were
placeholders.

**Disposition:** Accepted. The README and roadmap now distinguish Planned, In
progress, Verified, and Deferred. Verification requires evidence on the same
green commit.

### R-12 Long-term stewardship was missing

**Finding:** A decade-trust product needs reproducibility, ownership transfer,
end-of-life criteria, and a small confidence suite for future maintainers.

**Disposition:** Accepted for M7. Stewardship, release reproduction, known
platform assumptions, and an unmaintained-state policy remain required release
documents.

## 5. Required reasoning template

Every trust-critical feature answers:

1. What user expectation does it protect?
2. What is the authoritative state?
3. What is the commit point?
4. What can fail before and after that point?
5. What stale or duplicate event can arrive?
6. What remains preserved for the user after each failure?
7. Which automated artifact attempts to falsify the guarantee?
8. Which platform behavior still needs manual evidence?
9. What residual risk is accepted, and where is it explained?

## 6. Adversarial release questions

Before v0.1, reviewers should be able to answer these from evidence:

- Can any input, menu, dialog, window, recovery, or error path bypass the dirty
  decision state machine?
- Can a Save completion clear a newer dirty revision?
- Can an external writer replace the destination between inspection and commit?
- Does every pre-commit failure preserve the original?
- Does a post-commit durability warning report that new bytes are visible?
- Can a corrupt or foreign recovery record replace an in-memory document?
- Can a stale search, parser, or worker result update a newer revision?
- Can invalid UTF-8 create replacement characters without an explicit import?
- Can editing one mixed-EOL line rewrite unrelated endings?
- Can undo restore content but lose selection or caret intent?
- Can IME pre-edit enter undo or recovery before commit?
- Can a screen-reader user inspect and change the same text as a visual user?
- Can a long line allocate or lay out work proportional to the full line every
  frame?
- Can any dependency or Markdown input create an outgoing connection?
- Can the release claim green CI, coverage, or performance without evidence from
  that exact commit?

Any unanswered question is incomplete work, not an accepted guarantee.

## 7. Living response

This review is rechecked at each milestone gate. New failures become FMEA rows,
tests, or explicit residual risks. Closed findings remain in history so future
changes can see why the constraints exist.
