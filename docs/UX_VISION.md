# Noter UX Direction

**Reviewed:** 2026-07-26

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

## 2. The document is the visual hierarchy

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

## 5. Text Mode and Markdown Mode serve different needs

Text Mode is the classic notepad surface and can open any supported text file,
including Markdown, as exact source. Markdown Mode displays the same `.md` source
as clean formatted content and remains directly editable. Bold appears bold,
headings have hierarchy, and list items behave like lists; each edit is mapped
back to the smallest practical Markdown source transaction.

Switching modes never changes bytes. Text Mode remains available when source is
malformed or a construct is unsupported. Markdown Mode is a native editor and
viewer, not a web preview or proprietary rich-text document.

Mode and Theme are quiet top-level controls, aligned opposite the application
menus. Markdown formatting actions use a contextual row that disappears in Text
Mode instead of reserving empty document space.

Diagnostics do not mutate text. Whole-document formatting requires an explicit
command, a diff, a supported semantic-equivalence check, and confirmation. Text
Mode schedules no Markdown work.

## 6. The quality test

An experienced Notepad user should complete the ordinary workflow without
documentation. An unfamiliar user should recover from every error without
guessing. A keyboard, IME, or screen-reader user should not receive a reduced
editor. Performance should be demonstrated, not described with adjectives.

That combination of restraint, evidence, and care is the intended experience.
