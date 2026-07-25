# Noter Product and Engineering Research

**Research date:** 2026-07-25

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
5. Ship and dogfood the reliable plain-text editor before adding Markdown
   formatting.

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
- The local branch was five commits ahead of and one commit behind
  `origin/master`, so publishing requires an explicit history reconciliation.

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
- Save through an opened final symlink only after recording and revalidating the
  link and resolved regular-file target. Save As refuses an existing final
  symlink or reparse point.
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

- Markdown work follows the trustworthy plain-text release.
- Inline styling keeps source punctuation visible and never changes bytes.
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

For releases, use a current `cargo-dist` configuration, generate an SBOM and
checksums, pin GitHub Actions by immutable commit SHA, minimize token permissions,
and attach provenance. Current research found cargo-dist 0.32.0, so the existing
0.28.0 planning comment must be refreshed when distribution work starts.

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
