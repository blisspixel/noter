# Changelog

All notable project changes are recorded here. Noter has not published a stable
release. Prerelease changes move from Unreleased to a dated version only when
that candidate is frozen for publication.

## Unreleased

## 0.1.0-alpha.2 - 2026-08-23

### Added

- Add a [playtest brief](docs/PLAYTEST.md) that records what changed for the
  current round, which findings were fixed or declined, what is deliberately
  still open, and which product surfaces remain unexercised.
- Fail closed on a startup document argument that cannot be opened: a missing
  path, a directory, or an unreadable file now prints one line to standard error
  and exits 2 instead of opening a window that looks like a new blank document.
  Content problems such as invalid UTF-8 still open the window and report there,
  because those also arrive from the Open dialog and desktop file associations.
- Name the window `Update status - Noter` while the status opened by
  `noter update` is showing, so that session is distinguishable from a blank
  editor without reading the window contents.
- Answer `noter update --help` with the usage block instead of an error, and
  state the FILE and local-only update contract in that block.
- Add `RecoveryScheduleState::next_persist_delay`, the pure wake-up deadline an
  interface needs to keep the recovery-point objective while it sleeps.
- Record the `0.1.0-alpha.2` correctness matrix against implementation commit
  `2aa4a89`, mapping dogfood-critical recovery, conflict, clipboard,
  navigation, coverage, security review, and live native recovery evidence to
  the scoped prerelease decision.
- Record partial M2 installed-product evidence for a disposable Windows
  PowerShell source install at commit `91ed8d7` with exact-head main CI.
- Route Markdown active-block word, Home/End, and document keyboard gestures
  through the same pure caret path as Text Mode.
- Route Text Mode word, Home/End, and document keyboard gestures through pure
  caret navigation with Windows-like Ctrl and macOS Option/Cmd policies, leaving
  plain arrows to egui.
- Add pure UTF-8 caret navigation for character, classic word token, logical
  line step, line home/end, and document endpoints, plus a long-session undo
  history fixture that proves retained history never exceeds configured ceilings.
- Wire private crash recovery into the application: dirty-session persist under
  the per-user state directory, startup Restore / Discard, quarantine notices,
  Save and Discard cleanup, and a visible recovery persist-failure notice.
- Add external-change Overwrite Disk Version behind an explicit second
  confirmation; Keep Editing still never rebaselines the trusted disk version.
- Add Edit menu Cut / Copy / Paste on the shared edit-command path with platform
  shortcuts and `EditOrigin::Paste` / programmatic cut intent.
- Add View menu Full Screen toggle with the F11 shortcut.
- Add Markdown Mode strikethrough formatting on the formatting bar.
- Add pure M4 crash-recovery scheduling, versioned record integrity, and startup
  disposition logic with 2-second idle and 15-second max dirty persistence
  policy, epoch-correlated persist acknowledgements, checksum validation, UTF-8
  character-boundary selection checks, and quarantine reasons that never write a
  user document path.
- Add durable private recovery storage that stages owner-restricted siblings,
  installs or replaces instance records atomically, cleans exact displaced
  artifacts where supported or retains bounded recovery slots, bounds startup
  review by entry and aggregate byte budgets, offers at most 32 restores per
  launch, and reports quarantine relocation failures instead of claiming
  success.

### Fixed

- Preserve LF, CRLF, CR, and mixed source exactly through Text Mode plus active
  editing and inactive rendering in Markdown Mode. Logical newlines render,
  select, and delete atomically; Text Mode and active Markdown source editors
  copy and cut exact native source. Enter, paste, and IME input follow the
  nearest whole-document convention within the exact source-byte ceiling.
- Keep the exact Unix parent directory opened for atomic replacement or
  exclusive installation in a one-shot commit receipt. General-save durability
  and cleanup use that held directory instead of reopening a pathname that may
  have been renamed or rebound. Recovery creates its single keyed stage through
  the same held parent and consumes that stage with a descriptor-relative rename,
  so a successful commit leaves no recovery sibling to clean. Windows retains
  its explicit file-only durability result.
- Reconcile every Windows replacement result against the intended recovery
  bytes and captured predecessor before cleanup. Proven commits remove only
  the exact verification handles, using immediate unlink semantics when the
  filesystem supports them; ambiguous outcomes fail closed and preserve the
  staged content and predecessor needed for recovery. Deterministic per-instance
  stage and backup slots make later retries return `ResourceBusy` before they
  can accumulate another pair of artifacts.
- Ignore only a recovery candidate that disappears between startup enumeration
  and metadata lookup. Preserve and surface every other metadata I/O error so
  an inaccessible recovery record cannot be silently omitted from a successful
  scan.
- Make quarantine durability explicit: sync the verified quarantine file and
  its parent before deleting the exact source, then sync the source parent.
  Failures retain the original or report the durable quarantine copy instead of
  claiming cleanup that may not survive a crash.
- Retry a failed dirty recovery persist after a bounded one-second backoff while
  retaining the newest revision and epoch. Clock regression reanchors that
  backoff without creating a zero-delay repaint loop.
- Bind external-change overwrite confirmation to the exact regular-file
  observation reviewed. If the disk entry changes before the second
  confirmation, Noter preserves the newer revision and opens a fresh conflict
  decision instead of silently rebaselining and overwriting it.
- Let keyboard and screen-reader users activate inactive formatted Markdown
  with Enter, Space, or the platform accessibility Click action. Text Mode and
  active Markdown editing now expose stable, distinct accessible editor names.
- Reject repository documentation links through every symbolic path component,
  including links that resolve back inside the checkout, so validation behaves
  consistently across Git hosts and cannot approve a repository escape.
- Serialize publication attempts for the same release tag without canceling or
  replacing a queued run. Pull-request plans and dry runs retain isolated
  concurrency groups.

- Open private recovery storage only once during application construction, and
  isolate application unit tests from the platform recovery directory so
  parallel test leases cannot race or inspect a developer's recovery state.
- Retain a recovery artifact whose canonical or keyed pathname names a different
  instance than its encoded header. Once neither identity is live, startup
  reports the exact mismatch without offering or moving the record, and public
  quarantine refuses to move it without a single unambiguous dead-instance
  claim.
- Replace mutation-sensitive recovery verification and coalescing index loops
  with structurally finite traversal. Partition the expanded Linux and Windows
  mutation scopes behind one fail-closed aggregate check so every candidate
  remains required without exceeding the per-job execution ceiling.
- Fence recovery persistence before Save deletes a clean record or Discard,
  Restore, New, or Open retires an instance. Delete both independently locked
  lease paths by verified file identity before releasing either lock, so a
  rebound pathname cannot expose or erase a living window's recovery record.
- Treat ignored recovery backup cleanup or parent-directory sync as a failed
  successor transfer, so Restore retains its predecessor until the new record
  is durably established. Schema v1 metadata can no longer suppress a genuine
  schema v2 record with forged revision fields.
- Bind Keep Editing to the exact successfully inspected disk state. A later
  revision with the same broad conflict category prompts again, and an
  uninspectable state is never permanently acknowledged. Text, paste, and IME
  input from the frame that opens the prompt resumes after the decision.
- Preserve canonical Markdown selection when Enter cancels an unfinished IME
  pre-edit, leave list markers and emphasis-like text inside code blocks
  literal, and ignore keyboard auto-repeat for formatting toggles.
- Keep CJK IME pre-edit text in a transient widget draft in Text and Markdown
  modes. Composition remains visible but does not enter the document, dirty
  state, Undo history, or crash recovery until Commit; cancellation restores
  the authoritative text and selection. An active composition retains its
  final Commit when another control claims focus in the same frame.
- Discard deferred editor, Find, and Markdown input whenever New, Open, Reload,
  or Restore advances the document generation, so queued input cannot replay
  into replacement text. Recovery-unavailable guidance also preserves a more
  specific Open or Reload failure already shown to the user.
- Reschedule crash recovery after Undo and Redo, including dirty-to-dirty
  history transitions and cleanup when history returns to the saved baseline.
- Protect a clean in-memory revision immediately when its disk file changes.
  Native Close and destructive actions now treat it as unsaved, status and the
  title show Modified, recovery preserves it, and ordinary Save still fails
  closed against the external revision. Focus-regain inspection runs before
  same-frame file commands and invalidates any older close authorization.
- Keep startup recovery memory bounded by retaining metadata and exact open
  handles rather than document content. Reload and revalidate a selected record
  under an exclusive instance claim immediately before Restore or Discard, and
  surface incomplete bounded scans without touching unreviewed records.
- Add recovery schema v2 causal generations and whole-record integrity while
  preserving exact schema v1 reads. Coalesce only strict same-instance revision
  order and proven predecessor links; retain legacy, sibling, identity-conflict,
  and equal-revision divergent branches as separate offers without trusting wall
  clocks.
- Make recovery cleanup ownership-specific. Save removes only canonical and
  keyed temporary artifacts for its leased instance; Restore and Discard remove
  only revalidated exact handles and never sweep another live window's document
  lineage. Once Restore persists its successor, later cleanup failure leaves the
  successor durable, opens the recovered document, and surfaces a warning that
  later successful cleanup cannot erase. Successful external Reload also retires
  the explicitly discarded private copy, while failed Reload preserves it.
- Bind damaged-record quarantine to the exact opened artifact and refuse
  relocation if the pathname changed or its instance is live. Filter irrelevant
  and live artifacts before charging startup candidate budgets so bounded scans
  cannot deterministically starve a dead canonical record.
- Reject a startup argument naming a final symlink, Windows reparse point,
  directory, FIFO, or other special file through the same no-follow regular-file
  preflight as the loader. Content validation remains deferred to the GUI.
- Preserve Find and Go To Line focus across keyboard zoom and fullscreen
  commands. View-menu zoom actions are disabled at their limits, and Find Next
  or Find Previous with no query opens Find instead of doing nothing.
- Correct prerelease integrity guidance to distinguish archive and MSI
  checksum coverage from attestation-only assets, disclose MSI elevation and
  system PATH behavior, and put backup, signing, platform-evidence, changelog,
  and correctness-matrix guidance directly in the release notes. Require
  successful exact-main CI before creating the verified tag, create or safely
  refresh one private GitHub draft with the exact workflow artifacts, attest
  that payload, then apply reviewed notes and publish it.
- Bound Markdown's automatic inline reopening and list, quote, and inline-run
  Enter transforms before they mutate the active draft. Input at the document
  ceiling is rejected or UTF-8-truncated without removing an existing suffix.
- Preserve repeated and interleaved Enter, text, paste, and IME events in
  Markdown Mode, keep following lines separate, and apply automatic list or
  quote continuation only when the caret is at or after the complete source
  marker. Pending inline-marker reopening also stops at the first earlier
  order-sensitive event.
- Keep custom word, line, and document navigation behind earlier text, paste,
  IME, pointer, and accessibility events from the same input frame in both
  editor modes.
- Keep Markdown formatting shortcuts behind earlier ordered editor input from
  the same frame instead of applying them before those bytes.
- Preserve Find and Replace input order around Enter so navigation and
  replacement use only text that arrived before the command.
- Serialize file, edit, view, navigation, formatting, Enter, and Find or Replace
  commands through one ordered input path. Repeated commands execute once per
  event, and text, paste, IME, wheel, pointer, touch, and accessibility input
  cannot be overtaken by a later shortcut from the same frame.
- Preserve deferred Markdown ownership across frames only while the active
  editor still owns the sequence. Pointer, touch, Tab, accessibility-focus, and
  window-focus changes release later input instead of routing it back into a
  control that lost focus.
- Isolate destructive confirmation and recovery dialogs from editor input.
  Input that opened a blocking prompt is discarded behind it, and deferred
  editor input is held until an already-open prompt is resolved.
- Keep a window's recovery lease for its complete lifetime, including after
  Save, and remove only the recovery record when content becomes clean.
- Treat recovery lease acquisition and ownership-probe errors as unavailable
  recovery instead of exposing a potentially live record to another window.
- Keep recovery worker epochs monotonic across New, Open, Restore, and Discard,
  and ignore stale UI completions without deleting a newer record.
- Transfer Restore to a newly leased durable record before deleting the startup
  copy. Identity, lease, or write failure keeps the original offer and reports
  the unavailable recovery state instead of leaving restored text only in
  memory.
- Rearm dirty-document recovery when Open is cancelled or Open or Reload fails
  after an explicit Discard decision, so the document left on screen does not
  remain without a scheduled recovery record.
- Let the Windows benchmark harness briefly observe a process whose exit raced
  `TerminateProcess`, avoiding a false cleanup failure after bounded-output
  termination while sharing the existing absolute process-tree deadline across
  every captured handle.
- Close simple bold, italic, strikethrough, and inline-code runs before a line
  break at the end of the run in Markdown Mode. The same markers reopen only
  when the next character is typed, so the file never stores an empty pair such
  as `**\n**`.
- Focus the untitled editor on first launch so typing does not require a click.
- Report `Unsaved` for a never-saved document instead of labeling it `Saved`.
- Skip the startup recovery prompt on an explicit file open without deleting
  the private recovery records. Discard remains an explicit choice.
- Book a follow-up frame when only the caret moves so the status bar line and
  column catch up after an idle window.
- Persist crash-recovery records on a dedicated I/O thread so durable write and
  `fsync` do not stall typing. Completions stay epoch-tagged; a late write
  cannot revive a record after Save or Discard.
- Keep Editing no longer reopens the external-change prompt just because the
  local document revision moved. A different disk classification still prompts.
- Escape or Later on the startup recovery offer hides it without deleting the
  private copy. Restore no longer immediately replaces the recovered document
  with the next offer.
- Enter in a Markdown list item or quote continues the marker; Enter on an empty
  item exits to a paragraph.
- Alt+Z toggles Word Wrap in Text Mode. Ctrl+Shift+M (Command+Shift+M on macOS)
  switches Text and Markdown Mode. Both are ignored while Find or Go To Line
  owns the caret.
- Enter in the Replace field replaces a selected match, otherwise finds the next
  match. Opening Find with a selection no longer forces Replace All to Selection
  scope.
- A living Noter window holds an exclusive recovery lease so another window
  cannot Restore or Discard its in-flight private copy. A crash releases the
  lease so the next launch can offer restore.
- Update `webbrowser` to 1.2.4 for RUSTSEC-2026-0257, where Unix `BROWSER`
  handling allowed browser argument injection. The dependency reaches Noter
  through `eframe`, and only an explicitly clicked link uses it.
- An idle window no longer holds a CPU core. The window title was sent as a
  viewport command on every frame, and every viewport command requests a
  repaint, so the event loop never slept. The title is now sent only when it
  changes, and a dirty document books its own repaint from the recovery
  schedule so sleeping cannot lengthen the recovery-point objective. A local
  Windows release measurement fell from 100.2 percent of one core to 0.42
  percent over 15 idle seconds, with the working set down from 87.2 MiB to
  67.2 MiB.
- Recovery persist no longer leaves Unix-exchanged or hard-linked stage files
  after a successful commit.
- Recovery scan no longer treats a missing quarantine source as success or
  swallows quarantine I/O errors.
- Recovery scheduling no longer disables the recovery-point objective when the
  monotonic clock regresses, and no longer accepts late persist acks after Save
  or Discard.
- Mid-codepoint recovery selection offsets are rejected instead of offering a
  restore that would corrupt caret placement.
- Markdown strikethrough marker ambiguity handling shares the bold double-marker
  path without Clippy-identical arms; active-state checks avoid redundant
  selection clones.

### Changed

- Define the scoped alpha publication gate separately from the complete RC and
  stable release matrix. Alpha limitations must be recorded and assigned to a
  later blocking milestone, and critical or high security or data-safety
  defects can never be deferred.
- Give the top-level menu names visible space instead of a two-point gap, and
  keep every name clear of the Mode and Theme controls at every expanded width.
- Give both Mode segments one width sized for the longest label, so the switch
  no longer changes shape when the active view changes.
- Accept command-line `--theme` and `--view` values in any letter case.
- Track the update status as one state instead of two independent flags, so the
  status cannot be open and unnamed at the same time.
- Lead the public README with a clean-app stance: privacy first, zero spyware,
  zero telemetry, zero activity logging, free writing, and only the tool you
  asked for, while keeping an honest alpha status and network claims.
- Roadmap version train documents the ordered path from `0.1.0-alpha.1` through
  correctness alpha (`0.1.0-alpha.2`), beta, RC, and first public `0.1.0`.
- Add exact-checksum local M1 filesystem evidence for native NTFS, native WSL2
  ext4, and the Windows-to-WSL boundary, including explicit provenance,
  durability limits, unavailable environments, and remaining milestone gaps.
- Add a reproducible M1 trust-kernel benchmark harness with an exact synthetic
  corpus, process-cold and warm raw samples, recomputable nearest-rank
  percentiles, platform-named peak memory, release binary and four-target
  dependency evidence, bounded schema validation, and a canonical 30-sample
  Windows reference from a clean detached commit.
- Add a pinned cross-platform release workflow with archives, POSIX and
  PowerShell installers, MSI and Homebrew packaging, SHA-256 checksums, a
  target-specific CycloneDX 1.5 SBOM for each release platform, GitHub artifact
  attestations, and a non-publishing dry-run path. Release-tool bootstraps are
  versioned and checksum-pinned, the MSI keeps permanent product identities and
  embeds Apache-2.0 terms, and publication remains an explicit release gate.
- Add a release guide that distinguishes provenance from platform code signing
  and requires exact-commit CI, platform, installer, screenshot, privacy, and
  dogfood evidence before publication.
- Add a reproducible third-party dependency inventory and ship it together with
  the bundled-font license in standalone archives and the Windows MSI.
- Defer static musl archives until non-Cargo runtime licenses and SBOM evidence
  are represented in the validated release payload.
- Add a public vulnerability-reporting policy with a private-reporting path and
  guidance that keeps sensitive details and real documents out of public issues.
- Add persisted System, Light, Dark, Green Screen, and Amber Screen themes with
  deliberate document typography. The specialty palettes retain native text
  shaping and enforce enhanced text, selection, and control contrast in tests.
- Add a fail-closed specialty-palette validator that reconstructs the complete
  standard Dark state after invalid input, and document the safe declarative
  custom-theme boundary.
- Add an early native Markdown Mode with source-backed formatted editing, H1,
  H2, Bold, Italic, Link, Code, List, and Quote actions, plus five conservative
  source diagnostics. Supported heading and inline delimiters remain hidden in
  the active editor while the file stays ordinary Markdown on disk.
- Bundle the variable Inter typeface under the SIL Open Font License so headings
  and strong emphasis use real font weights consistently across platforms.
- Add working About Noter and truthful update-status dialogs, including the
  `noter update` entry point.
- Add locked source-install helpers for PowerShell and POSIX systems with check
  and custom-root modes.
- Add deterministic native README screenshot generation and validation for an
  identical-document Light Text and Markdown pair plus Dark, Green Screen, and
  Amber Screen Markdown captures. The calm non-sensitive demo proves the
  dual-view behavior while each formatted capture keeps the direct editor and
  formatting controls visible without transient focus pixels.
- Add revision-checked edit transactions with exact UTF-8 source expectations,
  exact inverses, directional selections, operation origin, and adapter-supplied
  monotonic timestamps.
- Add Edit-menu Undo and Redo with Ctrl+Z, Ctrl+Y, Cmd+Z, and Shift+Cmd+Z paths.
  History is shared by Text and Markdown modes and bounded by both transaction
  count and retained source bytes.
- Add explicit edit intent and bounded deterministic Undo coalescing. Adjacent
  typing, Backspace, and forward Delete group independently; paste, replacement,
  formatting, conversion, clock regression, selection movement, and resource
  boundaries end the group.
- Add a responsive non-modal Find and Replace bar with bounded literal queries,
  Unicode case matching, next and previous navigation, wrap reporting, match
  counts, and explicit selection or document Replace All scope. Navigation and
  replacement keep keyboard focus in the non-modal controls while restoring the
  visible document selection.
- Add a pure lifecycle reducer for dirty New, Open, Reload, Quit, and native
  close requests, backed by exhaustive transition tests and a fixed-seed
  reference-model property.
- Add Select All to Text Mode and Markdown Mode plus a validated Go To Line
  dialog for Text Mode that navigates LF, CRLF, CR, and mixed files without
  allocating a line index. Markdown selections can span parsed blocks, retain
  direction and exact line endings, and replace only the selected source bytes.
- Add forward and reverse primary-pointer selection across separately rendered
  Markdown blocks. The native interaction preserves exact source boundaries for
  Unicode and hidden syntax, keeps live cross-block feedback, autoscrolls at a
  bounded edge speed, preserves normal touch completion, and cancels cleanly on
  Escape or input loss without changing document bytes. Cancel collapses to the
  drag origin so the application selection cannot retain an aborted multi-block
  range through lagged fallback.
- Add persistent Text Mode word wrap and editor-only zoom from 50 to 300 percent
  with View-menu controls, standard zoom shortcuts, supported pointer
  magnification over the document surface, and a status indication.

### Added

- Add pure external-change classification and a conflict decision reducer that
  compare the trusted load or save baseline with focus-regain and bounded
  focused-timer inspections. Changed, deleted, special, and unreadable outcomes
  prompt Reload Disk Version, Keep Editing, or Save As. Keep Editing never
  rebaselines the expectation, so ordinary Save still fails closed through the
  durable save protocol when the disk version differs. Ordinary Save is paused
  only while the prompt is visible; Save As remains available.

### Changed

- Commit dirty Markdown drafts before selection restore so toolbar formatting
  and unfinished typing cannot be discarded by Find navigation, pointer zoom,
  or other same-frame restore paths. An active draft whose source range becomes
  invalid is kept visible instead of being silently cleared.
- Bind Escape finish to the active editor serial so a same-frame click into
  another block cannot be retired by the previous block's Escape.
- Sort inverted TextEdit character ranges in the bounded input buffer instead of
  panicking if a widget reports an unordered range.
- Replace the separate H1 and H2 Markdown buttons with one fixed-width
  paragraph-style selector for Paragraph and all six ATX heading levels. The
  selector reports the current style accessibly, marks mixed selections
  honestly, and applies idempotent source-backed line edits without changing
  native or mixed line endings. Parser-verified indentation, tab separators,
  and closing sequences preserve visible content; unsupported block structures
  remain byte-exact. Paragraph promotion additionally requires an exact style
  round trip, so ambiguous leading whitespace and trailing closing-style hashes
  report Unavailable instead of losing source. Directional selections remain
  directional, and compact windows place styles in a bounded submenu instead
  of clipping later actions.
- Make the release rehearsal install the Windows cargo-auditable binary from an
  independently checksum-pinned official archive and verify the extracted and
  installed executable hashes. POSIX targets retain their checksum-aware pinned
  installer and versioned receipt validation. This avoids mistaking Cargo
  1.97's own version response for the subcommand version while keeping every
  downloaded payload fail closed.
- Mark the generated third-party HTML license inventory as generated legal
  material for GitHub language analysis. Noter remains a compiled Rust desktop
  application with no WebView, browser engine, HTML interface, JavaScript
  runtime, or Python product path; release-critical maintainer automation is
  now scheduled for parity-verified Rust `xtask` consolidation.
- State Noter's privacy promise directly: no tracking, usage analytics,
  automatic crash uploads, or background activity logging, with no spyware or
  bloatware capabilities hidden behind opt-out settings.
- Bound macOS mutation-test linker input by disabling test-profile debug
  information and limiting test-profile codegen units to 16 in that job after
  Apple clang 21 crashed while linking a mutant even with serialized Cargo
  builds. A second fail-closed crash with stripped objects is retained as
  negative evidence. The complete workspace test scope and 47-candidate
  inventory remain unchanged, ordinary macOS tests retain their normal debug
  profile, and infrastructure-failure validation remains fail closed.
- Make Windows benchmark-command termination synchronous and bounded. The
  harness uses bounded terminate-and-rescan waves to retain identity-checked
  process handles, stop each captured Job Object member, and wait for its
  process object to signal. A final Job Object termination and active-count
  check run under the same deadline before returning the ordinary output-limit
  or deadline error. A distinct shutdown failure reports descendants that do
  not exit in time instead of claiming cleanup succeeded.
- Refine Markdown Mode into a direct word-editor surface. The continuous canvas
  now uses the available window width with only the ordinary editor inset.
  Paragraph and heading styles use one conventional style selector; Bold,
  Italic, links, inline code, lists, and quotes expose real toggle state. Every
  control applies only to the current selection or selected lines.
  Repeated formatting removes only parser-verified simple syntax; malformed,
  asymmetric, multi-backtick, deeper repeated-star, and empty-caret delimiter
  cases fail closed. Empty selections insert blank delimiters instead of
  invented prose. Link never preloads a fake label or URL, validates the exact
  candidate, and preserves complete code-bearing labels when toggled off. Bold,
  Italic, and Link also provide standard focused-editor keyboard paths. The
  formatting bar is permanent and non-modal, with Escape returning an active
  source range to rendered form instead of exposing a prototype-style Done
  control.
- Let the pointer wheel over the live zoom percentage change document zoom
  through the same bounded, non-editing command path used by its buttons,
  keyboard shortcuts, and menu. Clicking the percentage still resets to 100
  percent, and its accessibility value remains current.
- Give Green Screen and Amber Screen deliberate CRT treatment with native
  monospace type, square controls, bounded static scanlines, a restrained edge
  vignette, and theme-specific glass borders. Standard themes reset every
  specialty style. Preserve the complete default font fallback chain so
  Unicode and emoji bytes remain supported while M5 retains native emoji
  appearance validation.
- Rewrite the demo as a quiet proof of the dual-view workflow, then regenerate
  and visually review the complete five-capture README set after removing the
  centered page-like Markdown measure and modal Done control.
- Canonicalize third-party license generation from bounded cargo-about JSON and
  the legal files packaged in every locked dependency. Component expressions
  remain separate from preserved copyright and notice text, and target-gated
  nested notices are included without relying on host-sensitive associations.
  Conventional non-runtime trees and source-code lookalikes are excluded from
  legal-text discovery, while compact third-party notice names and bundled font
  license sidecars remain covered.
- Replace wide text-labeled Markdown formatting controls with compact visual
  controls, grouped by purpose, while retaining full accessible names,
  descriptions, and text labels in the responsive overflow menu.
- Keep the compact formatting layout through 479 pixels, verify every active
  control remains inside its viewport, and render Italic as a deliberate
  typographic icon instead of an ambiguous slash-like glyph.
- Refuse files above the framework editor's current 8 MiB interactive ceiling
  before constructing a complete widget string. The trust-kernel loader keeps
  its separate 64 MiB storage boundary, and M5 retains the 50 MiB release goal.
- Enforce the same interactive ceiling for Text and Markdown typing, paste,
  Replace, and Replace All, with a final restoration guard at the authoritative
  document boundary.
- Update `event-listener` to 5.4.2 so the locked dependency graph clears the
  applicable RustSec advisory.
- Refocus the root README on the product promise, native Markdown interaction,
  screenshots, source installation, release status, and a small documentation
  map. Move contributor workflow and detailed install, update, uninstall, and
  troubleshooting guidance into dedicated documents.
- Refresh the roadmap, design, baseline, mutation evidence, security review,
  UX direction, privacy title, and architecture review so current evidence,
  document ownership, public package metadata, and remaining work agree.
- Make the POSIX source installer report the validated and installed version in
  the same form as the PowerShell installer.
- Place the responsive Text and Markdown mode control and the current Theme
  menu on the upper-right of the application menu row. The second toolbar now
  appears only in Markdown Mode and separates formatting actions from document
  zoom. Visual, keyboard, and accessibility-tree order follow the same
  left-to-right sequence.
- Keep Edit, View, and Help pointer reachable through a compact More menu at the
  420-pixel minimum viewport width.
- Preserve the current directional source selection across Undo, Redo, editing
  mode switches, and safe fallback from an over-budget Markdown projection.
- Map click-and-drag selection in inactive formatted Markdown back to complete
  source spans, including hidden delimiters, escapes, and parser-decoded
  character references, so formatting commands operate on the text the user
  selected. Synthesized text that cannot be mapped without invention remains
  visibly editable as source.
- Commit focused Markdown input before activating another block, finishing the
  active edit, or applying a requested mode change, including when multiple
  input events arrive in one native frame.
- Verify version output and invalid-argument status, standard output, and
  standard error against the installed release executable in both source
  installers.
- Strengthen Help, About, update-status, and modal-close regression coverage
  with pointer, accessibility-tree, and exhaustive state tests. Add adversarial
  Markdown selection tests for cross-block ranges, invalid UTF-8 boundaries,
  non-first-block offsets, and explicit reference-definition styling.
- Report modified state, one-based logical line and Unicode-scalar column, and
  selection size in the responsive status bar.
- Scale native Markdown headings with editor zoom while leaving menus, dialogs,
  toolbars, and status controls at the configured application size.
- Bound Go To Line text, paste, and IME input before the focused widget can
  normalize or retain an oversized payload.

### Security

- Bind every newly created Windows file to the process user's explicit owner SID
  and protected user-and-SYSTEM DACL, then verify both through the opened handle
  before any document bytes are written. Filesystems that ignore or cannot
  report the requested private policy now fail closed and remove the exact open
  zero-byte file; a cleanup failure preserves both native causes and reports the
  possible private artifact separately.
- Add exact token-buffer length boundary tests after focused mutation testing
  exposed three survivors in Windows process-user SID acquisition. The settled
  clean-detached campaign catches all 20 private-security candidates with no
  unviable, missed, or timed-out result and retains machine-readable provenance.
- Bind the complete release workflow, CI workflow, and WiX authoring source to
  reviewed digests, with an independently isolated release-validator step and
  a separate cross-platform test-job digest. This makes commented-out gates,
  inherited shell bypasses, altered shell execution context,
  reordered publication steps, unpinned action syntax, disabled native tests,
  and dynamic MSI-directory redirection fail closed.
- Read dependency legal files through identity-checked descriptors, reject
  symbolic links, Windows reparse points, hard links, directory replacement,
  and observed concurrent mutation, and run the repository-script suite in the
  native Windows CI job.
- Restrict release workflow write, OIDC, and attestation permissions to the
  final host job; validate dispatch tags before shell use; checksum every
  release-tool bootstrap; upload the SBOM through the correct step output; and
  attest the source archive directly. Keep the per-machine MSI rooted in
  protected Program Files so its optional system PATH entry cannot target a
  user-writable directory.
- Restrict publication to the protected `main` branch and prerelease tags until
  M7 is complete. Publish reviewed, attestation-first notes instead of generated
  install commands or direct pipe-to-shell guidance.
- Declare all four target-specific SBOMs as cargo-dist artifacts so the release
  manifest and published asset set agree exactly. Recheck the remote `main` tip
  immediately before atomic tag creation so a concurrent merge cannot publish
  a stale candidate.
- Preserve positional document paths as native operating-system strings during
  command-line parsing. Non-Unicode Unix paths now reach the file loader, while
  non-Unicode option names and non-path values such as `--theme` and `--view`
  fail with controlled diagnostics instead of panicking during process startup.
- Bound text, paste, IME, Enter, and Tab document mutations before the Text and
  Markdown editor widgets can lay out bytes beyond their source budgets. The UI
  reports every truncation and preserves the prefix that fits on a UTF-8
  boundary.
- Track Markdown line-prefix state during the existing render pass so accepted
  64 KiB lines render in linear time instead of repeatedly scanning every prior
  byte for each character.
- Restrict any race-safe Unix displaced-document recovery artifact to
  owner-only mode through its verified open handle. On macOS, remove and verify
  the absence of extended access-control entries before retaining it.
- Bound PNG validation by regular-file type, repository symlink policy, encoded
  size, decoded size, and exact RGBA dimensions. Markdown link checking now
  verifies one opened file identity, refuses path swaps and symlinks, reads
  through that descriptor with an exact size ceiling, and rejects invalid UTF-8
  so untrusted pull requests cannot turn CI validation into an unbounded read or
  decompression operation.
- Escape control characters in rejected command-line values before writing
  diagnostics to a terminal.
- Ignore Noter's actual private save and backup recovery siblings, together
  with standard local Python test and coverage caches, so failed-save content
  and generated tooling output cannot be committed accidentally.
- Bind every document observation and reopen to a native no-follow handle. Unix
  uses `O_NOFOLLOW`; Windows opens the final entry with
  `FILE_FLAG_OPEN_REPARSE_POINT`, preserves ordinary sharing, and rejects link
  or reparse metadata before reading content.
- Bound document loading and save-target hashing to the explicit 64 MiB v0.1
  limit, including protection against concurrent file growth.
- Bound Unix extended-attribute snapshots to 4,096 entries and 64 MiB of
  aggregate names and values before value allocation, including macOS resource
  forks.
- Create Windows staging and new files with a protected owner-and-system DACL so
  permissive parent ACLs cannot expose staged document bytes.
- Create macOS staging and new files by requesting mode 0600 and a zero-entry,
  no-inherit ACL in the same `openx_np` operation. Native evidence proves the
  resulting file has true ACL absence while an ordinary control file inherits
  the parent ACE. Defensively remove and verify ACL absence through the live
  descriptor before writing any document bytes, and report the random zero-byte
  sibling if security finalization fails.
- Add a pinned RustSec audit gate to CI.
- Add a pinned weekly and manually dispatchable dependency-security workflow so
  advisory and license-policy checks also run when application code is idle.
- Add an explicit cargo-deny policy and pinned CI gate for dependency licenses,
  registry and Git sources, wildcard versions, advisories, and duplicate-version
  visibility.
- Pin the coverage tool used by CI.

### Fixed

- Stop every Save and Save As after an indeterminate commit until the user
  explicitly reconciles each retained outcome. New and Open preserve the block;
  dismissing its notice only hides the notice. Each record retains the selected
  destination, a 1-KiB display label, and a 4-KiB diagnostic, with a 128-KiB
  encoded-path ceiling and capacity reserved before disk mutation. The bounded
  recovery surface identifies each destination, offers an explicit Copy
  Destination Path action, uses a reversible hexadecimal operating-system
  encoding instead of lossy text for non-Unicode paths, and removes one record
  only after a confirmation that performs no write or retry. Reconciliation of
  the last record clears only its now-stale save-block error. At most 16 records
  can be retained, and no recovery surface performs filesystem identity work
  during repaint.
- Check every live Markdown draft against the structural projection budgets
  before semantic targeting or source-style parsing. Expanded and compact
  toolbar actions defer mutation until all command-state queries finish. Every
  synchronized edit is then checked as a complete document before block
  discovery, so a bounded active range cannot conceal an aggregate source,
  block, or parser-event limit. Inline and link formatting also run the bounded
  projection check on their expanded candidate before semantic validation. An
  adversarial same-frame paste receives one plain bounded layout section,
  commits exact source, and falls back to Text Mode with the specific exceeded
  budget.
- Preserve the final directional source selection when Escape closes an active
  Markdown edit in the same frame as input, allowing shared Undo and Redo to
  restore the correct post-edit caret.
- Use contrast-verified palette error colors in Light and Dark instead of a
  fixed pure red, with direct rendering and actual-surface contrast regressions.
- Reset Go To Line input, validation, and focus when a document is replaced or
  the application leaves Text Mode, and disable Reload while the document is
  Untitled.
- Make Windows private-file resource release and security flag invariants
  directly mutation-testable. LocalAlloc strings now carry their owned
  deallocator, and access, sharing, and security-information masks reject
  overlapping bits while retaining the exact Windows flag values.
- Serialize Cargo build jobs in the focused macOS mutation job after Apple
  clang crashed while linking concurrent workspace test binaries. Mutation
  validation remains strict and the job retains its 90-minute outer bound.
- Render conservative line-wide emphasis-spacing mistakes such as `*text *` as
  intended in Markdown Mode while reporting MD037 and preserving exact source
  until an explicit correction. Flush inactive render runs whenever visible
  style changes so trailing punctuation or whitespace cannot erase emphasis.
- Check the more specific Save As shortcut before Save, use Command on macOS
  and Control on Windows and Linux, and derive displayed shortcut text from the
  same command metadata.
- Return command-line status 2 with actionable usage for invalid arguments, and
  accept `--` before a document path that begins with a dash. A closed standard
  output or error pipe no longer panics the Windows release process.
- Restore the last authoritative document text and reset editor-local state if
  an in-memory edit cannot advance the document revision.
- Track dirty state against the last saved serialized-content fingerprint so
  Undo and Redo can return to clean saved bytes while revisions remain monotonic.
- Bound the synchronous early Markdown projection by source bytes, logical
  lines, line length, block count, block span, and parser events. Over-budget
  files remain unchanged in Text Mode, and Markdown diagnostic counts are
  cached by document generation and revision without retaining a diagnostic
  vector.
- Replace the dirty-document close trap with a Save, Discard Changes, and Cancel
  decision for New, Open, Quit, and native window close.
- Carry valid Text Mode selections across parsed blocks into one contiguous
  source-backed Markdown edit region, preserving direction and exact UTF-8
  boundaries. Invalid or out-of-bounds selections keep Text Mode and leave the
  source unchanged.
- Correlate destructive intents, save completions, and native-close
  authorization with the exact document revision. Unsolicited or stale save
  completions can no longer authorize abandonment.
- Retain indeterminate-save recovery guidance through every dirty-document
  decision and Cancel path, and block every Save and Save As until the user has
  explicitly reconciled that state.
- Keep a link destination visible and selected while it is being edited in
  Markdown Mode, then hide the source target again when the caret leaves it.
- Preserve replacement artifacts whose identity or bytes changed during cleanup.
- Delete Windows cleanup candidates through the exact verified open handle so a
  rebound pathname cannot redirect deletion.
- Keep Unix staging owner-only from creation through atomic exchange, finalize
  metadata after commit, and retain artifacts when safe handle-bound cleanup is
  unavailable.
- Preserve Unix destination metadata from an immutable pre-commit snapshot,
  verify the displaced original after atomic exchange, require its stable
  metadata payload to still match the snapshot, and never apply unratified or
  stale metadata to the committed file.
- Serialize the macOS ACL into the immutable metadata snapshot and replay it
  through the destination descriptor, eliminating temporary ACL paths while
  keeping resource forks and other xattrs inside the bounded snapshot. Treat
  macOS `ENOENT` from `acl_get_fd` as a distinct absent state, replay it with the
  native remove-ACL sentinel, and verify true absence. Record the native kernel
  behavior that replaying explicit zero-entry ACL text canonicalizes to absence.
- Surface exact save cleanup and durability warnings instead of a generic
  success warning.
- Process same-frame editor input before file commands and native close checks.
- Enforce the 64 MiB serialized-document ceiling before load, transaction,
  whole-text replacement, single Replace, and Replace All allocation, including
  the UTF-8 BOM boundary.
- Bound Find and replacement text before focused widgets receive text, paste, or
  IME commit events. Keep Find-field Undo local and restore document focus when
  the bar closes.
- Carry the exact pre-dialog Save As target expectation through hard-link
  confirmation so a rebound destination conflicts instead of being overwritten.
- Preserve creation-time identity failures and retained-sibling cleanup guidance
  as distinct typed errors.
- Detect a same-authority Windows staging mutation before replacement or during
  the final handoff, and classify postcommit mismatch as indeterminate.
- Make About Noter open a truthful project dialog and state the exact limits of
  the current Markdown implementation.
- Preserve editor focus and the exact source selection when switching from Text
  Mode to Markdown Mode or invoking a formatting control. A first click or drag
  on formatted content now activates that range in the same frame.

### Engineering

- Limit test-profile debug information while retaining optimized test code, so
  the growing native UI regression suite does not exhaust the MSVC PDB linker
  limit on Windows.
- Remove the redundant secondary root instruction stub so `AGENTS.md` remains
  the single repository working agreement.
- Verify implementation commit `08fd8a5` in exact-commit nine-context run
  [30702655806](https://github.com/blisspixel/noter/actions/runs/30702655806).
  Hosted Linux coverage is 93.02 percent for the whole workspace and 94.36
  percent for the UI-independent trust kernel. Linux, both required Windows
  mutation shards, and macOS classify 970, 939, and 47 candidates respectively
  with no miss, timeout, or recognized infrastructure failure.
- Verify implementation commit `1988337` in protected nine-context run
  [30606904746](https://github.com/blisspixel/noter/actions/runs/30606904746).
  Hosted Linux runs 411 Rust tests at 93.38 percent whole-workspace and 94.44
  percent trust-kernel line coverage. Linux, both required Windows mutation
  shards, and macOS classify 967, 901, and 47 candidates respectively with no
  miss, timeout, or recognized infrastructure failure.
- Exercise the whole-document replacement size decision at a small injected
  boundary, distinguishing the accepted limit from an oversized replacement
  before diffing without multiplying a 64 MiB fixture across mutation runs.
- Split Windows mutation enforcement across both deterministic cargo-mutants
  shards. Both shards remain required, preserving the complete filtered
  candidate set while keeping each job inside the runner time bound.
- Make Undo and Redo shortcut tests select the simulated operating system
  explicitly, so Windows and macOS conventions are verified independently of
  the runner host.
- Add public contribution and development guides and require them in the CI
  documentation inventory. Local documentation validation now checks
  GitHub-style heading fragments and rejects links that escape the repository
  as well as missing paths.
- Route plain-text input, direct formatted Markdown input, and Markdown
  formatting through the same atomic document mutation boundary. Add fixed-seed
  512-case reference-model properties for single replacements, ordered
  multi-edit transactions, and arbitrary Undo and Redo sequences.
- Route Replace and Replace All through the same revision-checked transaction
  authority, calculate bounded results before allocation, and compare literal
  search plus lifecycle command sequences with fixed-seed reference models.
- Measure the current Windows-local source checkpoint at 95.58 percent line
  coverage for the UI-independent trust kernel and 93.49 percent for the
  complete workspace. The 413-test suite includes 100 percent line coverage for
  lifecycle and logical-line navigation, 99.15 percent for transactions, 98.83
  percent for history, and 97.29 percent for literal search.
- Record a complete 256-candidate mutation campaign for the M3 transaction,
  Undo, literal-search, logical-line navigation, and lifecycle core. The settled
  exact-commit run catches all 216 compiling mutations, classifies 40 genuine
  compiler rejections, has no survivor or timeout, and passes the independent
  infrastructure-failure validator.

- Upgrade eframe and egui to 0.35 while retaining restricted features and
  enabling current shaping, hinting, theme-aware font transfer, and subpixel
  placement behavior. Replace the secondary Markdown renderer with one
  pre-layout native projection built directly from `pulldown-cmark`, reducing
  the dependency graph while keeping active and inactive content on the same
  real-weight style mapping.
- Lock the renderer's distinct light-background and dark-background
  coverage-transfer behavior with a regression test.
- Define and enforce repository-wide code-quality and evidence standards.
- Enforce Ruff linting and formatting for repository validation scripts in CI,
  and normalize text files to LF across supported development platforms.
- Keep local automation state and runtime logs in ignored dedicated directories.
- Remove obsolete tracked automation metadata and commented-out build or CI
  plans.
- Expand mutation enforcement through the native platform adapter with a macOS
  job and a current 741-candidate supported-platform union with no gap. Hosted
  run 30213398323 completed the 49-candidate macOS scope without a survivor and
  exposed only Linux decision-coverage gaps plus two shared line-scanner
  timeouts. The settled local correction removes mutable progress arithmetic,
  gives repeated native decisions exact named predicates, and retains all
  supported-platform candidates across Linux, Windows, and macOS. Exact-commit
  run 30221793209 passes the complete matrix: Linux 617 total with 438 caught,
  Windows 557 total with 381 caught, and macOS 49 total with 43 caught. Every
  remaining candidate is a validated compiler rejection; no scope has a miss or
  timeout.
- Give Windows mutation tests a 60-second minimum test-process timeout after a
  prior hosted run timed out one mutant even though its truth-table test had
  already failed. A focused rerun catches all four mutations of that predicate;
  the 90-minute outer job limit remains unchanged.
- Reject linker, compiler, storage, process, and tool-lock infrastructure
  failures that cargo-mutants would otherwise classify as unviable. Normalize
  ANSI-decorated logs and reject clang linker signal crashes after post-run
  review found one hidden in the otherwise green run 30219731527; corrected run
  30221793209 catches that mutant and passes the strengthened validator.
- Close a focused 58-candidate Markdown diagnostics campaign with a composite
  result of 55 caught and three genuine compiler rejections after isolating and
  rerunning one Windows linker-lock failure.
- Close focused mutation campaigns for lifecycle and save-result decisions with
  26 of 26 candidates caught, and for final native Markdown editing and
  rendering with 80 of 88 candidates caught plus eight genuine compiler
  rejections. Both final reports pass infrastructure validation with no miss or
  timeout.
- Close the focused 16-candidate final-entry observation campaign with 12
  candidates caught and four genuine compiler rejections. A first pass exposed
  one handle-kind truth-table gap; the settled rerun has no miss, timeout, or
  infrastructure failure.
- Maintain measured fixed-seed line coverage at 93.73 percent for the trust
  kernel and 89.49 percent for the complete workspace. The Windows-local suite
  contains 208 tests, and CI enforces respective 90 and 80 percent floors.
