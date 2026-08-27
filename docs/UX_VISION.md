# Noter UX Direction

**Reviewed:** 2026-08-04

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

### 1.1 Recovery and conflict surfaces (interaction contract)

When recovery and conflict UI are fully wired, they follow calm system-dialog
patterns rather than toast spam or silent auto-restore:

- **Startup recovery** appears before a normal untitled document: short plain
  language, optional original path label (never full content preview by
  default), primary action Restore, Later to keep the private copy without
  opening it, Discard, and an explicit note that restore does not overwrite the
  file on disk until Save. Escape is Later, never Discard.
- **Damaged recovery** uses quarantine language: what failed (checksum, schema,
  truncated), that the original document was not modified, and that the damaged
  record was set aside when possible.
- **Persist failure** is a durable status or bar, not a flash: recovery could
  not be written; continue editing; Save remains available; close still uses the
  classic dirty prompt.
- **External change** keeps one decision surface: Reload Disk Version, Keep
  Editing, Save As, and Overwrite only after a second confirm that names the
  irreversible disk replacement.
- Keyboard-only and screen-reader users receive the same choices, names, and
  default focus as pointer users. Escape cancels without discarding recovery
  offers unless the user chooses Discard.

## 2. The document is the visual hierarchy

The editing surface receives most of the window. Chrome is compact, calm, and
legible:

- a tested monospace stack for exact source and a bundled document face with
  real weights for formatted Markdown;
- comfortable line height and stable block rhythm;
- one continuous borderless Markdown canvas using the same outer gutter as Text
  Mode, with no centered measure or simulated paper card;
- obvious caret, selection, focus, modified, error, and conflict states;
- five restrained built-in themes with measured contrast, including Green
  Screen and Amber Screen specialty palettes;
- stable layout while menus, find, recovery, and errors appear;
- any non-editor visual layer leaves document state unchanged, dismisses on the
  first activity, and returns immediately to the document without replaying or
  retaining that dismissal input.

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
menus. Markdown formatting actions and their separated document-zoom cluster
use a contextual row that disappears in Text Mode instead of reserving empty
document space.

Diagnostics do not mutate text. Whole-document formatting requires an explicit
command, a diff, a supported semantic-equivalence check, and confirmation. Text
Mode schedules no Markdown work.

## 6. The quality test

An experienced Notepad user should complete the ordinary workflow without
documentation. An unfamiliar user should recover from every error without
guessing. A keyboard, IME, or screen-reader user should not receive a reduced
editor. Performance should be demonstrated, not described with adjectives.

That combination of restraint, evidence, and care is the intended experience.
