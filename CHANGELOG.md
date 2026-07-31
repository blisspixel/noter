# Changelog

All notable project changes are recorded here. Noter has not published a stable
release, so current work remains under Unreleased.

## Unreleased

### Added

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
- Add deterministic native Light and Dark README screenshot generation and
  validation using a polished non-sensitive Markdown demo document. The capture
  keeps formatted content active so the direct editor and formatting controls
  are visible together while suppressing transient focus pixels.
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
- Add Text Mode Select All and a validated Go To Line dialog that navigates LF,
  CRLF, CR, and mixed files without allocating a line index.
- Add persistent Text Mode word wrap and editor-only zoom from 50 to 300 percent
  with View-menu controls, standard zoom shortcuts, supported pointer
  magnification over the document surface, and a status indication.

### Changed

- Canonicalize third-party license generation from bounded cargo-about JSON so
  package order, source paths, line endings, and repeated license records cannot
  make the checked-in notice differ across build hosts.
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
  appears only in Markdown Mode and contains formatting actions only. Visual,
  keyboard, and accessibility-tree order follow the same left-to-right sequence.
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
- Preserve cross-block Text Mode selections when Markdown Mode cannot yet map
  them safely, explain the current one-block editing boundary, and restore
  same-block selections so Bold and the other formatting actions work
  immediately after switching modes.
- Correlate destructive intents, save completions, and native-close
  authorization with the exact document revision. Unsolicited or stale save
  completions can no longer authorize abandonment.
- Retain indeterminate-save recovery guidance through every dirty-document
  decision and Cancel path, and block an ordinary Save retry until the user has
  reconciled that state or chosen Save As.
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
