# Native Markdown Mode

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
  shaping. The inactive document and active editor share explicit Inter body,
  heading, and strong-emphasis weights rather than simulated bold.
- Selecting formatted content keeps supported heading and inline syntax
  visually formatted while editing the exact backing source range. Markdown
  delimiters remain in the edit buffer and on disk even when they are not
  painted.
- Clicking or dragging directly over inactive formatted content activates the
  corresponding source caret or selection. The mapping includes complete
  hidden delimiter, escape, and parser-decoded character-reference spans so an
  edit cannot split their source syntax. Synthesized text without a safe source
  substring remains visible and editable as raw source.
- A selected link destination is temporarily revealed and underlined while it
  is edited, then hidden again after the caret leaves that target.
- H1, H2, Bold, Italic, Link, Code, List, and Quote actions update the selected
  block's Markdown source immediately.
- The primary Text and Markdown switch remains in the upper-right application
  row. The contextual formatting bar appears only in Markdown Mode.
- Switching to Text Mode exposes the exact source produced by those edits.
- Source diagnostics currently report skipped heading levels, unsafe trailing
  spaces, repeated blank lines, and a missing final newline with stable rule
  identifiers.
- System, Light, Dark, Green Screen, and Amber Screen themes share the same
  document model.
- Remote images are not loaded, raw HTML is not executed, and no content is
  fetched in the background. Markdown-link opening is not implemented in this
  bounded slice; the final command requires an explicit user action.

This is an intentionally bounded implementation slice. It proves direct native,
source-backed formatted editing without claiming the final editing model is
complete.

## Current limitations

- Editing is range-focused rather than continuous across the full formatted
  document. Supported headings, emphasis, links, and inline code remain styled
  in the active range; complex and unsupported punctuation may remain visible.
- Tables, images, nested structures, reference resolution, and other complex
  constructs do not yet have complete native layout or editing behavior.
- Cross-block Markdown references may not resolve in every rendered fragment.
- Direct edits and formatting actions use the shared revision-checked
  transaction and bounded Undo and Redo history. Deterministic coalescing,
  complete operation classification, parser workers, and complete keyboard
  selection semantics are not implemented.
- GFM and CommonMark conformance, malformed-input behavior, IME, screen-reader,
  high-DPI, and large-file requirements still require release evidence.
- The synchronous prototype accepts at most 1 MiB of source, 8,192 logical
  lines, 64 KiB per line, 512 projected blocks, 64 KiB per block span, and
  8,192 parser events. A document that exceeds any ceiling opens unchanged and
  remains fully editable in Text Mode instead of entering an unproven formatted
  path.
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

The completed toolbar and accessible menus cover headings, emphasis,
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
