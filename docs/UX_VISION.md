# Exceptional UX for Noter

**Reviewed:** 2026-07-25

This is the experiential direction. [REQUIREMENTS.md](REQUIREMENTS.md) owns
behavior and [ROADMAP.md](ROADMAP.md) owns measurable delivery gates.

## 1. Trust is the primary interaction

Noter should feel predictable before it feels clever:

- Open shows the selected bytes or an explicit unsupported-encoding error.
- Save has one meaning and never performs an invisible conversion.
- Every destructive action uses the same Save / Discard / Cancel decision.
- Recovery is quiet while healthy, visible when needed, and never impersonates a
  successful Save.
- Errors explain what happened to the original file and where current work
  remains.

The best trust UI is consistent behavior. Status indicators and dialogs support
that behavior rather than compensating for ambiguity.

## 2. Text is the visual hierarchy

The editing surface receives most of the window. Chrome is compact, calm, and
legible:

- a crisp default monospace stack with tested fallbacks;
- comfortable line height and margins;
- obvious caret, selection, focus, modified, error, and conflict states;
- System, Light, and Dark themes with measured contrast;
- stable layout while menus, find, recovery, and errors appear;
- no decorative animation that delays input or hides state.

Noter is system-integrated through dialogs, shortcuts, theme preference,
accessibility, IME, and window behavior. It does not imitate native widget
appearance.

## 3. Responsiveness is measured

The visible result of ordinary input should arrive within one display frame in
the common case. Launch, open, search, scroll, memory, and long-line behavior are
measured against the corpus and percentile budgets in
[REQUIREMENTS.md](REQUIREMENTS.md).

The product does not promise instant editing of 500 MB files. A future large-file
viewer or incremental mode must earn a separate contract.

## 4. Quiet intelligence stays reversible

Find, conflict detection, recovery, list continuation, and Markdown diagnostics
may help without taking ownership from the user:

- background results are revision-tagged and cannot overwrite newer intent;
- automatic-looking edits are visible and one-step undoable;
- a changed file produces a calm but unmistakable conflict state;
- no alert relies on color alone;
- keyboard and screen-reader users receive the same state and choices.

## 5. Markdown follows the plain-text release

Markdown assistance keeps punctuation visible. Diagnostics do not mutate text.
Formatting requires an explicit command, a diff preview, a semantic-equivalence
check, and confirmation. Markdown disabled means no background parser and no
behavioral difference from a plain-text document.

## 6. The quality test

An experienced Notepad user should complete the ordinary workflow without
documentation. An unfamiliar user should recover from every error without
guessing. A keyboard, IME, or screen-reader user should not receive a reduced
editor. Performance should be demonstrated, not described with adjectives.

That combination of restraint, evidence, and care is the exceptional UX.
