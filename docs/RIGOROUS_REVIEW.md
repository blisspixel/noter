# Noter Structured Adversarial Review

**Original review:** June 2026

**Disposition reviewed:** 2026-07-25

**Status:** Historical findings with current responses

This document preserves the useful challenges from the first adversarial review
without treating a persona or prose critique as evidence. Current requirements,
design, roadmap gates, tests, benchmarks, and release records are authoritative.

## 1. Executive finding

The original planning showed unusual care for a small editor but confused
ambition with proof. It named atomic saves, recovery, large-file performance,
undo fidelity, Markdown formatting, and cross-platform quality before defining
their state machines, commit points, failure models, or evidence.

The central corrective action is:

> Every trust or performance claim must identify an invariant, implementation
> boundary, failure case, and artifact that can falsify it.

## 2. Findings and current disposition

### R-01 Save was described too loosely

**Finding:** "Write, sync, rename" did not define destination identity races,
metadata, symlinks, platform replacement, parent-directory durability, or a
post-commit sync failure.

**Disposition:** Accepted. [ADR-0003](adr/0003-durable-replacement.md) now
requires an injected storage boundary, explicit commit outcomes, pre-commit
revalidation, platform semantics, and fault evidence. It remains Proposed until
those questions close.

### R-02 Recovery was not a protocol

**Finding:** A temporary file and timer did not define ownership, schema,
integrity, retention, multiple instances, corruption, or failure behavior.

**Disposition:** Accepted. Recovery now uses private per-user application state,
versioned records, random identities, revisions, checksums, atomic manifests,
bounded scheduling, quarantine, and a controlled crash harness.

### R-03 Close behavior contradicted the classic mental model

**Finding:** Frictionless close backed only by recovery made Save ambiguous and
could turn cache retention into hidden document storage.

**Disposition:** Accepted. Every destructive action uses one
Save / Discard / Cancel state machine. Recovery is independent and never
authorizes silent close.

### R-04 Encoding and EOL fidelity lacked a mixed-file model

**Finding:** A single line-ending enum could not represent mixed-EOL content.
Lossy UTF-8 loading contradicted byte fidelity.

**Disposition:** Accepted. [ADR-0002](adr/0002-encoding-and-line-endings.md)
requires strict UTF-8, exact existing newline bytes, a mixed profile, local
insertion rules, and explicit undoable conversion.

### R-05 Undo requirements were prose, not a model

**Finding:** Coalescing examples did not prove that edits, selections, and
inverse operations preserve information.

**Disposition:** Accepted. M2 now requires revision-tagged edit transactions,
exact inverses, a bounded history, a simple reference model, and property tests
after every undo and redo.

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

**Disposition:** Accepted. Markdown follows v0.1. Styling is source-visible,
diagnostics are non-mutating, and Format requires a diff preview, parse
equivalence, EOL/BOM preservation, confirmation, and one-step undo.

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

**Disposition:** Accepted for M6. Stewardship, release reproduction, known
platform assumptions, and an unmaintained-state policy remain required release
documents.

## 3. Required reasoning template

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

## 4. Adversarial release questions

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

## 5. Living response

This review is rechecked at each milestone gate. New failures become FMEA rows,
tests, or explicit residual risks. Closed findings remain in history so future
changes can see why the constraints exist.
