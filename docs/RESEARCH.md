# Noter Product and Engineering Research

**Initial research:** 2026-07-25

**Updated:** 2026-07-27

This document records the evidence behind the roadmap. It is intentionally
separate from the product requirements so that claims can be revised when the
toolchain, operating systems, or editor ecosystem changes.

## Executive conclusion

Noter's opportunity is real, but the winning product is not the editor with the
largest performance claim or the longest feature list. It is the editor whose
behavior is easy to predict and whose reliability claims are backed by visible
evidence.

The immediate priority is therefore the trust kernel, not the custom renderer:

1. Make the repository and documentation honest and green.
2. Prove strict loading, byte fidelity, durable replacement, undo, dirty-state
   lifecycle, and recovery in UI-independent code.
3. Complete the classic single-document workflow on the existing editor control.
4. Run a time-boxed custom-editor feasibility gate for performance, IME, and
   accessibility before committing to a ground-up widget.
5. Stabilize the text transaction, lifecycle, and accessibility foundations
   before expanding the early Markdown slice. The first public-quality release
   includes both Text Mode and native Markdown Mode.

## Repository evidence

The audit started from local commit `3e60e1c` with existing working-tree changes.
At that point:

- The GUI could type text and perform a basic Open, Save, and Save As flow.
- New, Open, and Quit could discard dirty content without confirmation.
- Edit and View menu items were placeholders.
- Invalid UTF-8 was silently converted with replacement characters.
- The app kept both a `String` and a `Rope`, copying the full document at open
  and save boundaries.
- Save used a predictable sibling name, synced the temporary file, then called
  `std::fs::rename`; metadata, directory durability, symlinks, and injected
  failures were not covered.
- There was no editor model, bounded undo, recovery, external-change detection,
  configuration, recent-file implementation, or UI automation.
- Three unit tests passed. Whole-program line coverage was 28.24 percent.
  `src/core/document.rs` line coverage was 85.96 percent, but the safety
  properties named by the roadmap were not all implemented.
- Formatting and strict Clippy failed locally.
- The CI document check still referenced files that had moved into `docs/`.
- The local and public histories had diverged, so publishing required explicit
  reconciliation before consolidating on one `main` branch.

The statement that Phase 1 was complete was therefore not supported by its own
quality gate.

## Market lessons

Focused editors compete on the quality of ordinary operations:

- [CotEditor](https://coteditor.com/) emphasizes fast launch, encoding diagnosis,
  line-ending care, counts, and international text behavior. This supports
  treating encoding, EOL, CJK, RTL, and Unicode behavior as core product work.
- [Notepad++ backup preferences](https://npp-user-manual.org/docs/preferences/#backup)
  separate session snapshots from durable saved-file backups and explicitly warn
  that unsaved state is not a long-term backup. Recovery should be a safety net,
  not a misleading replacement for Save.
- [Sublime Text's changelog](https://www.sublimetext.com/download) repeatedly
  addresses long-line performance, hot-exit data loss, session corruption,
  encodings, IME, DPI, Wayland, and native dialogs. These are mature-editor edge
  cases, not optional polish.

Product decision:

- Default to classic Save, Discard, Cancel behavior for dirty destructive actions.
- Maintain crash recovery independently as a second line of defense.
- Do not add tabs, projects, accounts, cloud state, plugins, AI, or network access.
- Describe the interface as system-integrated and consistent, not native-looking.

## GUI and editor architecture

[egui's own project description](https://github.com/emilk/egui) calls a native
look a non-goal, notes that interfaces can still break between releases, and
recommends laying out only visible content for large scroll regions. It does
provide AccessKit integration and custom painting, so it remains a plausible
foundation if Noter states the trade-off honestly.

The standard
[egui `TextBuffer` contract](https://docs.rs/egui/latest/egui/widgets/text_edit/trait.TextBuffer.html)
requires an `as_str` view. A segmented rope cannot satisfy that without a
contiguous representation. The current dual `String` and `Rope` model is thus a
prototype, not the production architecture.

[Ropey 1.6](https://docs.rs/ropey/latest/ropey/struct.Rope.html) loads through
`from_reader` in O(N), owns segmented UTF-8 text, and does not become a
memory-mapped, lazy file simply because its input is an mmap. The design claim
that a 500 MB file will be memory-mapped "into the Rope" and become editable in
under 16 ms is not technically defensible.

Product decision:

- Retain the built-in `TextEdit` only to finish and validate the trust workflow.
- Build the authoritative edit model independently of egui.
- Time-box a custom-editor spike before full implementation.
- The spike must prove visible-line layout, cursor and selection correctness,
  IME composition, clipboard behavior, AccessKit exposure, and long-line limits.
- Keep a working fallback until the custom editor passes parity tests.

## Performance contract

The phrase "instant" must be backed by a reproducible benchmark corpus and
reference hardware. Opening 500 MB in under one 16 ms frame is not a v0.1 goal.

Initial v0.1 budgets:

| Measure | Required | Stretch |
| --- | ---: | ---: |
| Warm launch to first interactive frame | p95 <= 250 ms | p95 <= 100 ms |
| Open and edit 1 MiB UTF-8 file | p95 <= 150 ms | p95 <= 75 ms |
| First editable frame for 50 MiB file | p95 <= 2.0 s | p95 <= 1.0 s |
| Input to painted frame | p95 <= 16.7 ms, p99 <= 33 ms | p99 <= 16.7 ms |
| Scroll frame time after warmup | p99 <= 16.7 ms | p99 <= 8.3 ms |
| First literal-search match in 50 MiB | p95 <= 800 ms | p95 <= 400 ms |
| Idle RSS on reference Windows machine | <= 120 MiB | <= 80 MiB |
| 50 MiB document RSS | <= 350 MiB | <= 250 MiB |

All numbers must identify OS, CPU, storage, build profile, file corpus, sample
count, cold or warm state, and measurement method. A future 500 MB mode may be a
read-only or incrementally indexed viewer with explicit limitations.

## Save semantics

Atomic visibility and crash durability are related but different properties.

- [`std::fs::rename`](https://doc.rust-lang.org/1.97.1/std/fs/fn.rename.html)
  maps to different Unix and Windows operations. It does not supply metadata,
  conflict, symlink, or durability policy by itself.
- Microsoft documents that
  [`ReplaceFileW`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew)
  preserves several security and filesystem properties, but also documents
  error states where names or inherited streams may already have changed. A
  false return value cannot always mean Not Committed.
- Microsoft documents exclusive same-volume creation and the optional
  `MOVEFILE_WRITE_THROUGH` barrier in
  [`MoveFileExW`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw).
  Cross-volume copy-and-delete is not an atomic-save fallback.
- Linux [`rename`](https://man7.org/linux/man-pages/man2/rename.2.html) provides
  atomic replacement and keeps a destination instance on failure. Linux
  [`fsync`](https://man7.org/linux/man-pages/man2/fsync.2.html) explicitly says
  a separate directory `fsync` is needed to persist the directory entry.
- On macOS, the platform `copyfile` API can copy POSIX metadata, ACLs, and
  extended attributes independently from content. `F_FULLFSYNC` is the stronger
  persistence request. See the [Xcode copyfile manual](https://keith.github.io/xcode-man-pages/copyfile.3.html)
  and [Apple persistence guidance](https://developer.apple.com/documentation/xcode/reducing-disk-writes).
- The evaluated
  [`atomic-write-file` 0.3](https://docs.rs/atomic-write-file/0.3.0/atomic_write_file/)
  implementation has valuable opened-directory and unique-sibling techniques,
  but its own limitations say a final symlink is replaced and ACLs, extended
  attributes, timestamps, and SELinux contexts are not preserved. It is not a
  sufficient production adapter for Noter.

Product decision:

- Use an injected I/O adapter with explicit create, write, flush, metadata,
  file-sync, revalidation, commit, reconciliation, parent-sync, and cleanup
  boundaries.
- Model `Committed`, `Conflict`, `NotCommitted`, and `CommitStateUnknown`.
  Indeterminate commit keeps recovery and cannot be retried blindly.
- Refuse final symlinks and Windows reparse points for both Open and Save As in
  v0.1. Following a link remains deferred until the product has a resolved-target
  identity model and complete platform race fixtures.
- Require explicit confirmation before atomic replacement separates one name
  from other hard links to the same file.
- Preserve required permissions, ACLs, extended attributes, security context,
  encryption, compression, and named streams through platform-native behavior.
  If required metadata cannot be preserved, fail before commit rather than lose
  it silently.
- Never delete the destination first to make a rename succeed.
- A post-commit directory-sync failure is Committed with a durability warning,
  not Not Committed.
- Cloud, network, removable, and unknown filesystems return only the durability
  level actually demonstrated. Filesystem naming alone is not proof.

The complete platform decision and remaining evidence matrix are in
[ADR-0003](adr/0003-durable-replacement.md).

## Recovery semantics

Recovery files must survive an application restart, so the general OS temporary
directory is the wrong contract. The
[XDG Base Directory specification](https://specifications.freedesktop.org/basedir/0.8/)
defines state as data that persists across application restarts, including open
files and undo history. The
[`directories` crate](https://docs.rs/directories/latest/directories/struct.ProjectDirs.html)
exposes the corresponding per-application locations across target platforms.

Product decision:

- Store versioned, per-user recovery records in the application state or local
  data directory, never the general temp directory.
- Use random document and instance IDs, checksums, an explicit schema version,
  original-path metadata, and atomic manifest updates.
- Keep the original file untouched until the user invokes Save.
- Flush recovery asynchronously after edit bursts with a bounded recovery point
  objective. Sync recovery before allowing a frictionless close.
- If recovery persistence fails, show a clear error and retain the classic dirty
  prompt.
- Redact content and paths from diagnostic logs by default.

## IME and accessibility

A custom editor owns text-input correctness. The
[winit IME contract](https://docs.rs/winit/latest/winit/event/enum.Ime.html)
distinguishes pre-edit text from committed text, uses byte-indexed composition
ranges, and requires the candidate window to follow the caret. Keyboard event
tests alone are not enough.

The editor must expose text runs, selection, caret, editable value, and actions
through AccessKit. Automated semantic UI tests should use
[`egui_kittest`](https://docs.rs/egui_kittest/latest/egui_kittest/), which is the
current egui testing library. The roadmap's `egui_mcp` reference was not a
shipping test framework and has been removed.

Release tests must include NVDA on Windows, VoiceOver on macOS, and Orca on Linux,
plus real CJK IME composition, dead keys, emoji, combining marks, RTL samples,
high contrast, keyboard-only operation, and 125 to 200 percent scaling.

## Markdown scope

Markdown formatting is not a small renderer feature. The
[CommonMark specification](https://spec.commonmark.org/current/) contains
context-sensitive block and inline rules. Existing formatters such as
[mdformat](https://mdformat.readthedocs.io/en/stable/users/style.html) protect
users by checking that the parsed document is equivalent before and after
formatting. Existing linters expose many configurable and sometimes conflicting
style rules.

Product decision:

- Markdown implementation follows the trustworthy text prerequisites, but it is
  part of the first public-quality release rather than a later optional release.
- Text Mode always exposes source; Markdown Mode is a directly editable native
  projection of the same source.
- Diagnostics are non-mutating.
- Format is an explicit command that previews a diff, verifies AST equivalence,
  applies one undoable transaction, and preserves the document EOL/BOM policy.
- Smart continuation is an independently toggleable edit command.
- No remote images, HTML execution, link fetching, or hidden transformations.

## Toolchain and release engineering

The [`rust-version` contract](https://doc.rust-lang.org/stable/cargo/reference/rust-version.html)
should name a version actually tested in CI. Linting against an unpinned moving
`stable` toolchain makes a previously green commit fail when new lints appear.
The verified toolchain should be pinned, with a separate advisory latest-stable
job if desired.

Coverage must be enforced, not merely uploaded. `cargo-llvm-cov` supports
`--fail-under-lines`; meaningful coverage should target testable product code and
must be supplemented by property, fault-injection, mutation, UI semantic, and
manual platform tests.

For M1 invariant testing, [`proptest` 1.11](https://docs.rs/proptest/1.11.0/proptest/)
provides generated cases, shrinking, and composable strategies. Its published
features show that `std` can be selected without the default fork, timeout, and
bit-set features, and its Rust requirement is below Noter's pinned 1.97.1
toolchain. The M1 suite therefore uses it only as a narrowly configured
development dependency; it does not enter release artifacts.

For mutation testing, [cargo-mutants 27.1.0](https://mutants.rs/) supports a
checked-in `.cargo/mutants.toml`, workspace tests, Cargo argument forwarding,
and result artifacts. Its [CI guidance](https://mutants.rs/ci.html) recommends a
disposable checkout with `--in-place` and uploading `mutants.out`. The
[`--in-place` contract](https://mutants.rs/in-place.html) explicitly disallows
parallel `--jobs`, so Noter uses four copied-tree jobs for the local reference
run and paired serial in-place jobs in CI. Linux covers the common scope while
Windows covers the full scope, including inactive-on-Linux platform decisions.
The configured source scope is `src/core/*.rs`: mutation testing is a
trust-kernel decision gate, not a substitute for semantic GUI tests. The final
local campaign evaluated 341 mutants, caught 230, rejected 111 as unviable, and
reported no missed mutation or timeout. The first hosted Linux run then exposed
10 Windows-only inactive-branch survivors and drove the platform-aware gate. See
[M1_MUTATION_EVIDENCE.md](M1_MUTATION_EVIDENCE.md).

Rust's built-in benchmark harness remains an
[unstable nightly feature](https://doc.rust-lang.org/unstable-book/library-features/test.html),
which conflicts with Noter's pinned stable toolchain and reproducibility goal.
The M1 benchmark harness should therefore be a small stable binary that uses
[`std::hint::black_box`](https://doc.rust-lang.org/std/hint/fn.black_box.html),
a deterministic generated corpus, warmup rounds, enough measured samples for
percentiles, and machine-readable output. Baselines must record the commit,
toolchain, operating system, CPU, memory, build profile, corpus checksum, sample
count, and raw measurements. A regression gate should compare like-for-like
reference environments and retain artifacts rather than treating one developer
machine as a universal latency oracle.

For external-change detection, the official [`blake3` 1.8.5 Rust
implementation](https://docs.rs/blake3/1.8.5/blake3/) supplies a 32-byte
cryptographic digest and a
[`Hasher::update_reader`](https://docs.rs/blake3/1.8.5/blake3/struct.Hasher.html#method.update_reader)
API that consumes a file without first allocating a second full-content
buffer. Noter enables only `std`; mmap, multithreading, serde, digest-trait, and
zeroization features are disabled. Tests use the upstream
[reference vectors](https://github.com/BLAKE3-team/BLAKE3/blob/master/test_vectors/test_vectors.json)
for zero-byte and one-byte inputs and separately prove reader errors cannot
produce a fingerprint.

The source audit found a build script plus optimized assembly or C SIMD paths,
but no runtime I/O capability is imported by Noter. The upstream `pure` flag is
documented as an unstable testing feature, so relying on it would be a weaker
maintenance contract than accepting and recording the optimized build path.
The dependency adds four lock entries and passed a 2026-07-25 RustSec scan. Once
the production adapter made hashing reachable from the GUI, the current native
I/O and truthful-shell slice measured 4,913,664 bytes, or 4.69 MiB, in the
stripped Windows release.
Hashing latency still requires the reproducible M1 benchmark corpus.

For stable file identity, Rust 1.97.1 exposes Unix `dev`, `ino`, and `nlink`
through [`std::os::unix::fs::MetadataExt`](https://doc.rust-lang.org/1.97.1/std/os/unix/fs/trait.MetadataExt.html).
The analogous full Windows methods are still unstable, so the product crate
cannot use them while keeping a stable toolchain. Microsoft specifies that a
Windows file is identified by the volume serial number plus the 128-bit ID from
[`FILE_ID_INFO`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info).
The older
[`BY_HANDLE_FILE_INFORMATION`](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/ns-fileapi-by_handle_file_information)
also supplies the hard-link count and a 64-bit ID, but Microsoft warns that the
smaller ID is not guaranteed unique on ReFS.

The resulting design keeps all ordinary observation logic in the unsafe-free
product library and isolates three by-handle observation calls in the internal
`noter-platform` crate. It prefers the 128-bit ID, treats a failed query or
all-zero unsupported ID as a labeled reduced fallback, and combines identity
with BLAKE3 content rather than trusting timestamps. The third query reads
Windows `ChangeTime` and `FileAttributes` from
[`FILE_BASIC_INFORMATION`](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/ns-wdm-_file_basic_information),
so metadata-only changes invalidate a saved observation even on volumes that do
not advance `ChangeTime` for an attribute update. Unix uses inode change time
with nanosecond precision. Observation hashes one open handle, checks stable
metadata around the read, and reopens the final path before accepting the result.
Final links and Windows reparse points are classified without following them.

For private sibling names, [`getrandom::fill`](https://docs.rs/getrandom/0.4.3/getrandom/fn.fill.html)
uses the operating system's preferred random source and reports every failure,
including a partial fill. Noter requests 16 bytes per candidate, hex-encodes all
128 bits, and combines that unpredictable name with `create_new`. Randomness is
not treated as exclusivity: deterministic tests force a collision and prove the
existing file is untouched before the adapter retries. Repeated collisions are
bounded, and a random-source error creates no artifact.

The temporary file records identity from its original open handle. Windows
cleanup opens final entries without following reparse points, observes identity
and content on that handle, and marks the same object for deletion. A pathname
rebound after observation therefore cannot redirect deletion to the replacement
entry. Portable Unix unlink cannot express the same object-bound condition, so
Noter retains uncommitted siblings and displaced originals with explicit cleanup
warnings. Unix replacement uses an atomic exchange, and siblings remain mode
0600 until their bytes have committed. An absent Unix destination keeps that
owner-only mode as the intentional v0.1 new-file policy. After exact displaced
identity, content, and metadata validation, Noter restricts that same open object
to mode 0600 before retaining it. macOS also removes the ACL and verifies its
absence. A failure remains an explicit cleanup warning rather than a false
privacy guarantee. On macOS, mode 0600
alone does not suppress inherited ACL entries. Noter therefore passes a
zero-entry ACL with the global `no_inherit` flag and mode 0600 to `openx_np`
atomically. Exact-commit native execution proves that a control file inherits
the parent ACE while the protected file immediately reports true ACL absence.
The kernel canonicalizes the zero-entry request rather than retaining an empty
ACL. Runtime defensively applies the remove-ACL sentinel through the live file
descriptor and verifies absence before writing bytes. Windows staging and new files use a protected DACL granting full control
only to the owner and SYSTEM; broad parent entries are not inherited. Direct
`getrandom` use adds no lock entry because the exact version was already present.

For Linux metadata, the adapter first commits the owner-only sibling through an
atomic exchange. It then uses descriptor-based `fchown` and copies and verifies
the complete visible extended-attribute set before applying `fchmod` last. It also
probes `security.capability`, `security.selinux`, and `system.posix_acl_access`
explicitly because Linux assigns different visibility and permission rules to
the user, security, system, and trusted namespaces. See
[`xattr(7)`](https://man7.org/linux/man-pages/man7/xattr.7.html). If post-commit
metadata cannot be reproduced, the save remains committed, keeps the safest
access state reached, retains the displaced source, and reports an exact warning.
`rustix` 1.1 supplies descriptor-relative atomic exchange, no-replace rename,
hard-link, mode, ownership, and synchronization operations. The Linux and macOS
metadata adapters use `xattr` 1.6, the one new package in the 339-package lock
graph.

On macOS, `acl_get_fd` obtains the source ACL and `acl_to_text` serializes an
existing ACL into the immutable pre-commit snapshot. An `ENOENT` result denotes
ACL absence and remains distinct from every present ACL with entries. Native
execution shows that an explicit zero-entry ACL is canonicalized to absence.
After the private exchange commits, `acl_from_text` reconstructs a present ACL,
or the native `_FILESEC_REMOVE_ACL` sentinel restores absence; `acl_set_fd` applies
either state through the destination descriptor. Noter applies owner and mode
separately and replays bounded extended-attribute values from the same snapshot,
so a successful save advances modification time. See the
[Xcode `acl(3)` manual](https://keith.github.io/xcode-man-pages/acl.3.html).
The sibling receives `F_FULLFSYNC` where supported and falls back to `sync_all`
only when the stronger operation is reported unsupported or invalid.

On Windows, `ReplaceFileW` is called with a random backup and zero ignore flags.
Microsoft documents that it merges DACLs, encryption, compression, and named
streams, and that error 1177 can move the old destination to the backup while
leaving the metadata-merged replacement under its temporary name. The adapter
recognizes only that documented partial state, verifies all identities and bytes,
then completes the intended move or returns Commit State Unknown while retaining
artifacts. New-file installation uses `MoveFileExW` with only
`MOVEFILE_WRITE_THROUGH`; replacement and cross-volume copy flags are absent.

The native adapter adds one lock package overall, bringing the cross-target graph
to 339. The 2026-07-25 RustSec audit is clean, and the adapter passes native
Windows, macOS, and Linux CI. The remaining evidence is manual NTFS, Linux,
macOS, cloud, network, removable, and weaker-filesystem testing plus the
reproducible benchmark corpus; the implementation does not infer durability from
a filesystem label.

For releases, use a current `cargo-dist` configuration, generate an SBOM and
checksums, pin GitHub Actions by immutable commit SHA, minimize token permissions,
and attach provenance. Current research found cargo-dist 0.32.0, so the existing
0.28.0 planning comment must be refreshed when distribution work starts.

## Native Markdown and text-rendering update

An early source-backed formatted Markdown Mode now exists, but it is not the
completed M6 editor. Current primary-source research defines the remaining
direction:

- [CommonMark 0.31.2](https://spec.commonmark.org/0.31.2/) supplies the base
  conformance corpus. The [GFM specification](https://github.github.com/gfm/)
  defines tables, task lists, strikethrough, and autolinks as explicit
  extensions and warns that rendered HTML still requires sanitization.
- [Zed's appearance documentation](https://zed.dev/docs/appearance) separates UI
  and buffer type, exposes deliberate line-height choices, and keeps System,
  Light, and Dark behavior explicit.
- [DirectWrite](https://learn.microsoft.com/en-us/windows/win32/direct2d/direct2d-and-directwrite),
  [Core Text](https://developer.apple.com/documentation/coretext/), and
  [FreeType](https://freetype.org/freetype2/docs/hinting/text-rendering-general.html)
  converge on font metrics, shaping, hinting, subpixel placement, antialiasing,
  gamma behavior, and display density as the relevant quality controls. Generic
  image dithering is not a sound text-editor rendering strategy.
- [egui 0.35 FontTweak](https://docs.rs/egui/latest/egui/struct.FontTweak.html)
  exposes hinting targets and per-font subpixel binning. Its
  [coverage-transfer API](https://docs.rs/epaint/0.35.0/epaint/enum.FontColorTransferFunction.html)
  provides distinct defaults for dark text on a light background and light text
  on a dark background. Noter uses the matching eframe and egui versions, keeps
  global hinting and subpixel binning enabled, and preserves the matching
  coverage transfer for each theme.
- [WCAG 2.2](https://www.w3.org/TR/WCAG22/) requires at least 4.5:1 contrast for
  ordinary text, while its
  [non-text contrast guidance](https://www.w3.org/WAI/WCAG22/understanding/non-text-contrast.html)
  requires 3:1 for visual information needed to identify controls and states.
  Green Screen and Amber Screen enforce 7:1 primary text, 4.5:1 weak and
  selected text, and 3:1 control outlines against their adjacent backgrounds in
  deterministic palette tests.
- Noter bundles the variable Inter typeface for proportional document text and
  requests explicit weight-axis values for body text, headings, and strong
  emphasis. This avoids simulated bold and makes the formatted editor more
  consistent across supported operating systems.
- The current [pulldown-cmark options](https://docs.rs/pulldown-cmark/0.13.4/pulldown_cmark/struct.Options.html)
  keep CommonMark as the default and require extensions to be enabled by named
  flags. The current bounded implementation uses named options only; M6 owns the
  final dialect and conformance evidence.

The resulting product direction is native and source-first. Text Mode schedules
no Markdown work. Markdown Mode renders a restricted local model, never executes
HTML, and never fetches images or references. Direct edits and formatting
actions update standard source. Supported inline delimiters remain in source but
are visually suppressed while their content stays formatted. A direct
`pulldown-cmark` event projection builds the inactive native layout before text
shaping, while the active editor keeps the exact source buffer and reveals a
link destination only while it is edited. The current range-focused
implementation is a vertical slice; M6 replaces its local edit path with shared
revision-tagged transactions, full keyboard and accessibility behavior,
stale-result rejection, and conformance evidence. Whole-document Format remains
a separately reviewed diff with semantic equivalence, idempotence, byte-policy
preservation, and one-step undo.

For visual quality, Noter uses separate document and UI type scales, fixed
control metrics, an 840-point reading measure, theme-specific contrast, and
deterministic Light and Dark captures from the native release renderer. Low-level
rasterization preferences are not exposed as feature clutter until native
platform testing demonstrates a user benefit.

## 2026-07-27 architecture review update

The current implementation review reinforces four constraints already present
in the roadmap:

- Unicode navigation cannot be improvised from bytes, scalar values, or ASCII
  punctuation. [Unicode Standard Annex 29](https://www.unicode.org/reports/tr29/)
  defines extended grapheme and default word boundaries, identifies editor
  selection and cursor movement as direct uses, and requires an implementation
  to declare either the default behavior or a documented profile. Noter must
  pair the selected profile with the corresponding conformance data and explicit
  locale limitations.
- Incremental Markdown work needs an edit-aware syntax representation.
  [Tree-sitter's editing contract](https://tree-sitter.github.io/tree-sitter/using-parsers/3-advanced-parsing.html)
  demonstrates the relevant architecture: update the old tree with the exact
  byte and point edit, then parse against that tree so unchanged structure can be
  shared. This supports the need for `EditTransaction` and revision-tagged
  parsing; it does not by itself select Tree-sitter or a particular Markdown
  grammar. Any parser change still requires dialect, malformed-input,
  dependency, and source-range evidence.
- [The Update Framework security model](https://theupdateframework.io/docs/security/)
  treats arbitrary installation, rollback, fast-forward, freeze, and unbounded
  download behavior as distinct update-system threats. TLS and a single checksum
  do not establish that the offered release is current or that metadata cannot
  be mixed. Noter's updater design must answer these threats before it downloads
  or installs executable content.
- [dist 0.32](https://axodotdev.github.io/cargo-dist/) can generate release
  archives, installers, machine-readable manifests, and release CI, and its
  configuration can install a standalone updater. These are useful distribution
  mechanisms, not a substitute for Noter's update threat model, package-manager
  ownership rules, clean-system tests, or release provenance.

The current formatted Markdown slice still performs synchronous whole-document
block discovery and non-virtualized rendering. Until the M5 and M6 architecture
passes its measured gate, Markdown Mode therefore enforces explicit ceilings
for source bytes, logical lines, line length, projected blocks, block span, and
parser events. An over-budget file remains unchanged and available in Text
Mode. Diagnostic counts are cached by document generation and revision so an
unchanged document is not rescanned merely because another frame is painted.
These controls bound known prototype work without presenting it as the final
editor engine.

## Rejected shortcuts

- Declaring a phase complete because a few happy-path tests pass.
- Using whole-program coverage padding as proof of data safety.
- Replacing the destination by deleting it first.
- Calling recovery data "autosave" when the original file was not saved.
- Storing recovery in a directory the OS may clear at any time.
- Shipping a custom editor before IME and accessibility parity.
- Claiming native appearance from a toolkit that explicitly does not target it.
- Treating Markdown formatting as a regex cleanup pass.
- Making a 500 MB benchmark the product identity of a focused notepad.
