# Rigorous Review: Noter - Planning Critique

**Review type:** Internal adversarial planning critique.
**Date of Review:** June 2026 (planning baseline)
**Document under Review:** README.md, REQUIREMENTS.md, DESIGN.md, ROADMAP.md, and supporting artifacts (as of the post-version-hygiene state).
**Context:** Private consultation requested by the lead engineer to "make this not a pile of complete dog shit."

---

## 1. Executive Summary from the Reviewer

The current planning corpus for Noter is already markedly superior to the typical "I was annoyed at Notepad and wrote a Rust GUI toy" project. The separation of concerns (core vs. UI), explicit non-goals, phased gates with quantitative coverage targets, property-based testing mandate, atomic I/O emphasis, and the recent addition of an "Engineering Philosophy" section demonstrate unusual self-awareness for a solo desktop tool effort.

However, it is not yet at the level of an *exceptionally engineered* artifact. It still carries the latent risks of:
- Informal specification of critical behaviors (save semantics, undo model, recovery protocol).
- Insufficient treatment of failure modes beyond "we will test atomic rename."
- Version and dependency selection that is "latest GA" but lacks a documented decision framework (stability, maintenance burden, proof of correctness in context).
- Absence of a lightweight formal model or invariant catalog that can be mechanically checked (even if only via exhaustive property tests and model checking lite).
- No explicit long-term sustainability or "end-of-life" criteria for a tool that aspires to be "the one you trust for a decade."
- Risk that the "purity" aesthetic becomes an excuse for under-engineering the parts that *must* be complex (the text buffer, cursor model, concurrent file observation).

**Verdict:** Promising foundation. With the additions below, this can become a reference example of how to build a small, trustworthy, cross-platform interactive application in 2026 without descending into the usual accretion of accidental complexity. Without them, it will be "better than Win11 Notepad for a niche" but still fundamentally slop-adjacent in its engineering discipline.

I recommend treating the next 2-3 weeks of "Phase 0 refinement" as a miniature research project: produce a minimal formal model, a first-cut FMEA, and an expanded verification plan before writing the first line of the real editor widget.

## 2. Discussion of the Stated Pain Points

**Engineer:** The user hates the Win11 Notepad because of telemetry, bloat, loss of the "just works" quality of the classic versions, and a general feeling that the OS vendor no longer respects the simple text-editing workflow.

**Reviewer:** This is a classic case of technological displacement of user values. The original Notepad embodied a minimalist artifact: low cognitive load, high predictability, immediate feedback, no hidden state that violates the user's mental model of "a bag of characters on disk."

The Win11 version (and many "modern" rewrites) optimizes for different stakeholders: telemetry for product telemetry teams, discoverability for novice users via "modern" UI chrome, and integration with cloud identity surfaces. The result is a violation of the *principle of least astonishment* for the original user population.

Your project is an attempt at *restorative design*. That is legitimate, but it carries a burden of proof: you must demonstrate, not merely assert, that Noter restores the valued properties (instant open, byte-faithful save, no side effects) while adding the two 2026 concessions (system theme fidelity and a non-mutating Markdown view) without re-introducing hidden state or performance cliffs.

**Recommendation:** Add an explicit "Mental Model Alignment" subsection to REQUIREMENTS.md. List the user's expected model ("the file on disk is exactly the characters I see, modulo line-ending normalization that I control") and require every major feature to have a "mental model impact statement."

## 3. Critical Weaknesses Identified in Current Artifacts

### 3.1 Specification Gaps (The Most Serious)

The DESIGN.md describes a `Document` with `save_atomic`, line-ending detection, etc. This is good prose, but it is not a *specification*. In dependable systems we distinguish:

- Informal description
- Semi-formal (state machines, pre/post conditions)
- Mechanically checkable (TLA+, Alloy, or even just a reference implementation + exhaustive tests)

Current state: mostly informal + some Rust-like pseudocode.

**Concrete gap example:** What is the exact contract for "atomic save" under the following conditions?
- Target directory is on a network filesystem (NFS, SMB) with delayed visibility.
- Another process has the file open with a delete-on-close or mandatory lock (Windows).
- Disk is full after the `.tmp` write but before the rename.
- Power loss after `fsync` but before rename completes on a journaling vs. non-journaling FS.

The plan mentions "test aggressively" and "simulated power loss." That is necessary but insufficient without first writing down the intended *guarantees* and the *accepted residual risk*.

**Action:** In DESIGN.md, add a "Core Behavioral Specification" section (lightweight) with:
- State machine for Document (Closed / Loaded / Dirty / Saving / Saved).
- Pre- and post-conditions for `save_atomic`.
- Explicit "liveness" vs "safety" properties (e.g., "if save returns success, the on-disk bytes match `to_bytes()` at the moment of the call, modulo FS reordering that we cannot control").

### 3.2 Dependency Selection Lacks a Framework

You correctly chose conservative versions (ropey 1.6 over 2.0-beta, polling before notify 9-rc). However, the rationale is ad-hoc ("we like stable").

A PhD-level treatment would define *selection criteria*:
1. Maintenance health (last release, number of active maintainers, response time to issues, bus factor).
2. Proof burden in your context (does the crate come with its own test suite that covers your usage?).
3. API surface and transitive risk (use `cargo tree -i` + `cargo audit` + manual review of "does this pull in anything that can do I/O or networking at init time?").
4. Long-term stability of the abstraction (will upgrading this crate in 2028 require rewriting core logic?).

**Action:** Add a short "Dependency Governance Policy" subsection to DESIGN.md (and reference it from Cargo.toml). Require that every Phase gate include an updated dependency health table for the crates introduced so far.

### 3.3 Verification Strategy Is Still "Testing + Dogfooding"

This is the part that most often turns "serious hobby project" into "we shipped and then found the data-loss bug in the wild."

Current plan has:
- Unit + property tests
- Golden files
- Manual matrix
- Simulated crashes

Missing or weak:
- Systematic fault injection (not just "kill -9", but targeted corruption of the `.tmp` file, simulated rename failure, simulated mtime race).
- Model-based testing or state-machine coverage for the Editor + Document interaction.
- Fuzzing of the load path (arbitrary byte sequences that must either parse or fail with a user-actionable message, never panic or produce garbage on-disk).
- Usability validation beyond "I used it for two weeks." Even solo, you can do structured tasks: "open a 40 MiB log with mixed line endings, find a specific error string, replace all occurrences of X with Y, save under a new name, verify byte identity with external tool."

**Action:** Create (or expand into DESIGN) a "Verification and Validation Plan" that distinguishes:
- Safety verification (data integrity, no loss, fidelity)
- Liveness / performance verification
- Usability / mental model verification
- Regression harness that can be run by a future maintainer in 2029.

### 3.4 Sustainability and "The Bus Factor of One"

You aspire to something users can rely on "for a decade." A solo project with no succession plan is, by definition, time-bombed.

**Reviewer:** Every dependable system I have studied that survived its original author had at minimum:
- A written "handover" or "stewardship" document.
- A minimal "reproducibility envelope" (exact pinned toolchain + how to build the release binaries on a fresh machine in 2031).
- Criteria for "when to declare the project unmaintained and point users at alternatives."

**Action:** Add a "Project Stewardship and Longevity" section to ROADMAP.md (and a short reference in README). At Phase 4, produce a `STEWARDSHIP.md` that includes the reproducibility recipe, known fragile platform assumptions, and a "sunset" decision process.

### 3.5 The Markdown Preview Risk (Subtle Scope Creep Vector)

The plan is good: "view only, pure Rust, no mutation." But Markdown rendering has a long history of turning into "almost a browser." Even with pulldown-cmark events, the temptation will appear to support tables, task lists, footnotes, embedded images (local only?), math, etc.

Each addition increases the attack surface for "weird file makes the preview do something surprising" and the maintenance surface.

**Recommendation:** In REQUIREMENTS and DESIGN, add an explicit "Markdown Preview Scope Contract" that lists the *exact* CommonMark subset supported in v0.1 and the process for adding any extension (requires new mental-model impact statement + new test vectors that exercise both editor and preview on the same input).

## 4. Recommended Expansions and Formalizations

### 4.1 Add a Lightweight Formal Model (Prose + Rust Enums + Properties)

Create in DESIGN.md (or a new `CORE_SPEC.md`) a section titled "Core Model - Safety and Liveness Properties."

Example skeleton the review expects:

```text
Safety Property S1 (Save Fidelity):
  for all doc, path.
    if save_atomic(doc, path) returns Ok(()) then
      on_disk_bytes(path) == to_bytes(doc)   (modulo the documented line-ending and BOM policy at load time)

Liveness Property L1 (Progress under Normal Conditions):
  If the user requests Save and the FS is writable and has space, the call terminates in bounded time (modulo FS latency).

Undo Invariant U1 (Information Preservation):
  For any sequence of EditorCommands C1..Cn that are undoable,
  apply(undo_stack, apply(C1..Cn, initial_state)) == initial_state
  (modulo viewport and selection, which are explicitly not part of the undoable document content).
```

These are then turned into property tests with clear comments: "This test is the executable form of Safety Property S1."

### 4.2 Failure Modes and Effects Analysis (FMEA) Table

A one- or two-page table in DESIGN.md is worth more than paragraphs of "we will be careful."

Columns the review expects:
- Failure Mode
- Potential Effect (on user data, user trust, etc.)
- Severity (1-10)
- Detection Method (current plan)
- Mitigation / Prevention
- Residual Risk (after mitigation)
- Owner / Phase when addressed

Example rows:
- Partial write during save (power loss mid-rename)
- Line ending detection fails on file with mixed \r\n and \n in the middle
- Autosave file is itself corrupted or from a different Noter instance
- User opens file from OneDrive while another device is editing it
- egui font shaping produces different column counts than the user's mental "character" count on certain Unicode sequences

Doing even a first-cut FMEA forces you to confront that some risks (network FS, concurrent writers) will have residual "user must be careful" documentation rather than perfect technical mitigation.

### 4.3 Dependency Health and Evolution Table

At each phase gate, produce (and commit) a small table:

Crate | Version | Last Upstream Release | Bus Factor (est.) | Our Usage Surface | Upgrade Risk | Decision Rationale

This makes the "latest GA that makes sense" claim auditable rather than vibes.

### 4.4 Expanded Non-Goals with "Why We Rejected" Justifications

Current non-goals list is good. Make it stronger by adding 1-2 sentences of rejection rationale for each (e.g., "Tabs rejected in v0.x because they introduce hidden per-tab dirty state and focus management complexity that violates the single-document mental model of classic Notepad. Revisit only after a formal model of multi-document state machine exists and has been property-tested.").

## 5. On "Purity" vs. Engineering Rigor

"Purity" (plain text on disk, no web tech in core, no telemetry) is a *value* and a *constraint*. It is not a substitute for rigorous engineering of the parts that must be complex.

The text editing core (rope + cursor + undo + viewport) will be the most complex and bug-prone part of the system regardless of how "pure" the rest of the UI is. The plan already recognizes this by mandating ropey and a custom widget. The review asks you to make the complexity explicit and verified rather than hoping "simple UI" will make the hard parts easy.

## 6. Final Recommendations (Prioritized)

1. **Immediate (before any substantial implementation code):** Add the Core Behavioral Specification (safety/liveness properties) and a first FMEA table to DESIGN.md. This is the highest-leverage anti-slop activity.
2. **Phase 0 gate enhancement:** Require the dependency health table and mental model impact statements for the first features.
3. **Phase 1-2:** Build a small fault-injection harness (separate from the main binary) that can corrupt `.tmp` files, simulate rename failures, etc., and assert that recovery or error paths behave as specified.
4. **Phase 4 (release):** Produce `STEWARDSHIP.md` + reproducibility recipe + the Markdown scope contract as part of the release artifacts.
5. **Ongoing:** Treat the RIGOROUS_REVIEW.md (this document) as a living artifact. Re-read it at the start of each phase and add a review response subsection noting which recommendations have been actioned and which were consciously deferred with rationale.

## 7. Closing Remark

Small tools are not exempt from the laws of dependable systems; they are simply smaller instances of the same problems. The difference between a tool that users quietly trust for years and one that eventually betrays them with a silent data loss or a "why did my line endings change?" surprise is almost always the presence or absence of the kind of explicit reasoning you are now being asked to document.

If you do the work outlined here, Noter will not merely be "the notepad I wish existed." It can become a case study in how to do minimalist interactive software with professional-grade engineering discipline in the 2020s.

---

**Appendix A (for the engineer):** Suggested minimal structure to add to DESIGN.md immediately:

```markdown
## 4.5 Core Behavioral Specification (Lightweight)

### Safety Properties
- S1: Save Fidelity (see above)
- S2: Line Ending & BOM Preservation
- S3: Undo Information Preservation (U1 above)

### Liveness & Progress Properties
- ...

### Accepted Residual Risks
- ...

## 4.6 Failure Modes and Effects Analysis (Initial)

| ID | Failure Mode | Effect | Sev | Current Detection/Mitigation | Residual Risk | Phase Addressed |
|----|--------------|--------|-----|------------------------------|---------------|-----------------|
| F1 | ...          | ...    | 9   | ...                          | ...           | 1               |
```

**Appendix B:** The mental model impact statement template (one paragraph per major user-facing operation).

This concludes the review.
