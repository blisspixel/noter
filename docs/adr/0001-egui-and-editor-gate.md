# ADR-0001: Keep egui and gate the production editor

**Status:** Accepted

**Date:** 2026-07-25

## Context

Noter needs a small cross-platform GUI, system dialogs, custom drawing, IME, and
accessibility. egui and eframe already form the prototype. egui explicitly does
not target native widget appearance, and its standard text-buffer contract
requires contiguous string access. A rope-backed production editor therefore
cannot be assumed to emerge from wrapping `TextEdit`.

Changing GUI stacks now would discard useful prototype work without evidence
that another stack satisfies the complete product contract.

## Decision

Keep egui and eframe as the application shell through the M4 correctness alpha.
Describe the UI as system-integrated and consistent, not native-looking.

Use the built-in `TextEdit` only as a bounded correctness adapter. Before
building a production custom editor, run the M5A vertical slice defined in the
roadmap. It must prove:

- correct rope edits, selection, caret, and hit testing;
- bounded visible-row and long-line layout;
- real IME pre-edit and candidate-window behavior;
- accessibility text, caret, selection, and edit actions;
- measured interaction latency and a credible 50 MiB route.

Failure does not authorize an inaccessible or IME-incomplete editor. The team
must retain the bounded adapter with reduced claims or explicitly evaluate
another GUI and text stack.

## Consequences

- v0.1 performance claims depend on the M5 gate.
- The core edit and application models remain GUI-independent.
- M4 can validate the full trust workflow before renderer R&D.
- A custom editor carries explicit accessibility and international-input cost.

## Evidence

- [Research record](../RESEARCH.md#gui-and-editor-architecture)
- [Roadmap M5](../ROADMAP.md#m5-custom-editor-feasibility-gate-and-production-engine)
- [Technical design, GUI strategy](../DESIGN.md#9-gui-and-editor-strategy)
