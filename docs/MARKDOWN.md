# Native Markdown Mode

**Reviewed:** 2026-08-02

**Status:** Early source-backed formatted editing is available. The complete
contract below remains in progress.

## Purpose

Noter treats `.txt` and `.md` as first-class, ordinary files. Text Mode is the
classic notepad surface for exact source. Markdown Mode presents the same `.md`
source as clean formatted content and maps direct edits back to standard
Markdown. Neither mode creates a proprietary document or uses a webview.

The source bytes, revision, encoding policy, and line endings remain
authoritative. Changing modes never rewrites a file.

## What the current build does

- `.md` and `.markdown` files open in Markdown Mode by default; any supported
  file can be viewed in Text Mode.
- Markdown is projected into borderless native egui text layouts before
  shaping. One continuous editor background owns the window, without a nested
  page card, border, or shadow. Content uses the available canvas with only the
  same outer editor gutter as Text Mode, so Markdown does not begin at an
  artificial page offset. The inactive document and active editor share
  explicit Inter body, heading, and strong-emphasis weights, deliberate line
  height, and stable block spacing rather than simulated bold.
- Selecting formatted content keeps supported heading and inline syntax
  visually formatted while editing the exact backing source range. Markdown
  delimiters remain in the edit buffer and on disk even when they are not
  painted.
- Clicking or dragging directly over inactive formatted content activates the
  corresponding source caret or selection. A primary-pointer drag can continue
  forward or backward across separately rendered blocks, keeps the native live
  selection visible, and scrolls at a bounded speed near the document edges.
  Escape, pointer loss without a completed primary release, or window focus
  loss cancels without activating an edit and collapses selection to the drag
  origin caret so lagged application state cannot keep an aborted multi-block
  range. A completed touch release followed by pointer loss still activates at
  its retained final position. Dirty active drafts commit before selection
  restore so formatting and unfinished typing are not discarded. The mapping
  includes complete hidden delimiter, escape, and parser-decoded
  character-reference spans so an edit cannot split their source syntax.
  Synthesized text without a safe source substring remains visible and editable
  as raw source.
- A selected link destination is temporarily revealed and underlined while it
  is edited, then hidden again after the caret leaves that target.
- One fixed-width paragraph-style selector sets selected logical lines to
  Paragraph or any of the six ATX heading levels and exposes the current or
  mixed state to assistive technology. Style selection is idempotent and
  preserves selection direction plus native or mixed line endings. Only
  parser-verified top-level paragraphs and ATX headings are styleable. Code,
  setext headings, nested blocks, and other unsupported structures report
  Unavailable and remain byte-exact. Paragraph promotion also requires an exact
  style round trip; ambiguous leading whitespace or content that would become
  closing ATX syntax fails closed so changing styles cannot discard source.
  Indented and tab-separated ATX markers and optional closing sequences are
  handled without exposing syntax as content.
  Bold, Italic, Link, Code, List, and Quote actions are selection-aware toggles.
  Repeating a command removes only a simple parser-verified construct, and line
  commands change only selected logical lines. Malformed, asymmetric,
  multi-backtick, or deeper repeated-star
  delimiters fail closed instead of being partially removed. Empty inline
  selections insert paired delimiters with the caret between them, but an empty
  caret alone does not claim ownership of existing literal delimiters.
  Link inserts `[]()` or `[selected text]()` without inventing a label or URL,
  rejects a selection that would not parse as the exact new link, and toggling
  a supported inline link preserves its complete source label, including code
  spans. Bold, Italic, and Link use
  Ctrl+B, Ctrl+I, and Ctrl+K on Windows and Linux and the corresponding Command
  shortcuts on macOS while the Markdown editor owns focus. Keyboard auto-repeat
  does not repeatedly toggle formatting. Toolbar buttons
  expose their active state to assistive technology. The bar is permanent and
  non-modal: there is no Done state, and Escape returns a focused active range
  to rendered form after its pending source edit is synchronized. Same-frame
  input is committed first, and the final directional source selection is
  retained so shared Undo and Redo restore the correct post-edit caret.
- The primary Text and Markdown switch remains in the upper-right application
  row. The contextual document bar appears only in Markdown Mode, with
  formatting groups on the left and a visually separated zoom cluster on the
  right when space permits.
- Edit > Select All and the platform-standard shortcut select the complete
  source in Markdown Mode without changing bytes. A valid directional Text Mode
  selection can span parsed blocks and native or mixed line endings; switching
  views activates one contiguous source-backed edit region and preserves the
  exact anchor and caret. Invalid UTF-8 boundaries fail closed in Text Mode.
- Switching to Text Mode exposes the exact source produced by those edits.
- Source diagnostics currently report skipped heading levels, spaces that
  prevent portable emphasis, unsafe trailing spaces, repeated blank lines, and
  a missing final newline with stable rule identifiers.
- A conservative line-wide `*text *`, `**text **`, `_text _`, or `__text __`
  mistake is displayed with the intended emphasis in Markdown Mode while
  MD037 reports the non-portable source. Merely changing or viewing modes never
  rewrites the spacing.
- System, Light, Dark, Green Screen, and Amber Screen themes share the same
  document model. Green and Amber use native monospace text, square controls,
  and a bounded static scanline and edge treatment without animation, document
  inspection, or network access. Standard themes restore their proportional
  application type and rounded controls completely.
- Editor zoom scales formatted body, heading, emphasis, code type, and their
  line heights together from 50 to 300 percent while preserving hierarchy. The
  Markdown document bar, View menu, standard keyboard shortcuts, supported
  pointer gesture, and status indicator all use the same bounded zoom state.
  The document-bar reset control reports the live percentage to assistive
  technology, accepts vertical pointer-wheel motion for zoom, and still resets
  to 100 percent when clicked. Document-bar zoom activation preserves control
  focus for repeated operation instead of activating document content.
  Formatted content remains wrapped by design; the Text Mode word-wrap
  preference does not change Markdown source or layout policy.
- Remote images are not loaded, raw HTML is not executed, and no content is
  fetched in the background. Markdown-link opening is not implemented in this
  bounded slice; the final command requires an explicit user action.

This is an intentionally bounded implementation slice. It proves direct native,
source-backed formatted editing without claiming the final editing model is
complete.

## Current limitations

- Editing is still range-focused rather than permanently continuous across the
  full formatted document. Select All, a selection carried from Text Mode, and
  a pointer drag across inactive rendered blocks can activate one contiguous
  source-backed region. Supported headings, emphasis, links, and inline code
  remain styled in the active range; complex and unsupported punctuation may
  remain visible.
- Tables, images, nested structures, reference resolution, and other complex
  constructs do not yet have complete native layout or editing behavior.
- Cross-block Markdown references may not resolve in every rendered fragment.
- Direct edits and formatting actions use the shared revision-checked
  transaction and bounded Undo and Redo history. Direct input has conservative
  operation intent, and adjacent typing, Backspace, and forward Delete use the
  shared bounded coalescing policy. Parser workers and complete keyboard
  selection semantics are not implemented.
- Enter at the end of a simple bold, italic, strikethrough, or inline-code run
  closes that run on the current line. The same markers reopen only when the
  next character is typed, so the file never stores an empty marker pair such as
  `**\n**`. A line break in the middle of a run that CommonMark still treats as
  one span is left unchanged. Escape after such an Enter commits the closed run
  without writing empty markers.
- Enter in a `- ` list item or `> ` quote continues the same marker (and indent)
  on the next line. Enter on an empty list or quote item removes that marker and
  leaves a paragraph. Fenced and indented code remain literal: list-like and
  emphasis-like text inside code does not trigger these Enter transforms.
  Native line endings are preserved. Cancelling an unfinished IME pre-edit with
  Enter restores the canonical source selection before applying the line break.
- GFM and CommonMark conformance, malformed-input behavior, IME, screen-reader,
  high-DPI, and large-file requirements still require release evidence.
- The synchronous prototype accepts at most 1 MiB of source, 8,192 logical
  lines, 64 KiB per line, 512 projected blocks, 64 KiB per block span, and
  8,192 parser events. A document that exceeds a Markdown ceiling but remains
  within the current 8 MiB interactive-file ceiling opens unchanged in Text
  Mode instead of entering an unproven formatted path. Larger files are refused
  without replacing the open document until M5 delivers a virtualized editor.
  A live draft is structurally checked before semantic or source-style parsing.
  Toolbar mutations wait until all command states have been derived from the
  previous bounded draft. Every synchronized edit is then checked as a complete
  document before block discovery, including aggregate source, block, and
  parser-event ceilings. Inline and link commands check their expanded local
  candidate before any semantic validation pass. An adversarial same-frame edit
  or formatting expansion receives one plain bounded layout section, commits
  exact source, and falls back to Text Mode with the exceeded budget.
- Diagnostics are a conservative initial set, not a complete Markdown linter.
- Whole-document Format, reviewed diffs, semantic-equivalence checks, and safe
  fixes are not implemented.

Text Mode remains the recovery path for unsupported or malformed source.

## Final document model

- Text Mode exposes complete source and punctuation without Markdown work.
- Markdown Mode is a directly editable projection of that same source.
- Untouched regions are preserved byte-for-byte.
- Every Markdown operation changes the smallest practical source range and is
  one reversible transaction.
- Ambiguous edits reveal source instead of guessing.
- Unsupported constructs remain visible and editable as source.
- Parser failure cannot block Text Mode or saving.

## Formatting controls

The completed M6 toolbar and accessible menus will cover headings, emphasis,
strikethrough, inline and fenced code, links, quotes, ordered and unordered
lists, task lists, and supported tables. Commands must work with empty and
non-empty selections, expose keyboard paths, and avoid stacking invalid
delimiters when toggled repeatedly.

## Markdown quality engine

Diagnostics are non-mutating and tied to an exact document revision. Each rule
has a stable identifier, severity, concise explanation, and a safe fix only when
the transformation is unambiguous. Correctness rules remain separate from style
preferences.

The default profile targets portable CommonMark plus an explicitly documented
GitHub Flavored Markdown subset. Rules cover structural consistency, ambiguous
syntax, heading and list hygiene, fenced-code clarity, local link integrity,
table structure, and formatter conflicts.

### Format Document

Whole-document formatting is always explicit. Before applying it, Noter must:

1. parse the original under the selected syntax profile;
2. create deterministic candidate source;
3. parse the candidate under the same profile;
4. reject unsupported semantic differences;
5. present an accurate diff;
6. preserve encoding, BOM, line endings, front matter, and opaque regions under
   documented rules; and
7. commit the accepted result as one undoable transaction.

The formatter must be idempotent. A regex-only document formatter is not an
acceptable implementation.

## Safety and performance

- Raw HTML is inert and remote content is never fetched automatically.
- Parse and render results carry revisions; stale work is discarded.
- Parser depth, tokens, caches, diagnostics, and formatter output are bounded.
- Markdown work is incremental where the parser permits it and never blocks
  ordinary Text Mode editing.
- Typing, selection, IME, and scrolling take priority over decorative styling.

Exact latency and memory budgets are in [REQUIREMENTS.md](REQUIREMENTS.md).

## Completion evidence

Markdown Mode is not release-complete until automated and manual evidence proves
CommonMark and selected GFM conformance, exact source preservation, minimal and
reversible transactions, formatter idempotence and semantic safety,
stale-result rejection, keyboard and IME behavior, accessibility, high-DPI and
theme behavior, inert remote content, and bounded malformed and large-file
performance.
