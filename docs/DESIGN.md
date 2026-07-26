# Noter Technical Design

**Version:** 0.3

**Reviewed:** 2026-07-25

**Status:** Active architecture contract

This document translates [REQUIREMENTS.md](REQUIREMENTS.md) into an
implementation architecture. [ROADMAP.md](ROADMAP.md) owns sequencing,
[RESEARCH.md](RESEARCH.md) owns external evidence, and architecture decision
records own narrow irreversible choices.

## 1. Current implementation checkpoint

The current M1 worktree has:

- a binary crate containing the egui shell;
- a library crate containing the UI-independent document module;
- strict, 64 MiB-bounded UTF-8 loading with BOM and existing newline-byte
  preservation;
- exact newline-free, uniform, and mixed line-ending profiles with counts,
  deterministic dominant fallback, and edit-point insertion decisions;
- a 19-case external golden corpus and three 512-case generated properties for
  strict byte round-trip, classification, and insertion policy;
- explicit `Encoding`, `Bom`, and checked `Revision` values;
- an injected save protocol with exact conflict, non-commit, commit, uncertain
  commit, durability, and cleanup outcomes;
- BLAKE3-256 content fingerprints from complete byte slices or streaming
  readers, checked against the official reference vectors;
- a bounded stable-file observation that combines an open-handle identity,
  content fingerprint, length, hard-link count, and metadata change token;
- a narrow internal platform crate that preserves the main crate's unsafe-code
  prohibition while wrapping the required native identity, metadata, commit,
  and synchronization operations;
- 128-bit random, exclusive, owner-tracked sibling files with bounded collision
  handling, owner-only Unix mode, a protected Windows DACL, strongest supported
  file synchronization, and identity-and-content-safe cleanup;
- a production `FilesystemStorage` adapter with metadata transfer, atomic
  existing-file replacement, exclusive new-file installation, documented
  Windows partial-state reconciliation, parent barriers, and exact destination
  verification;
- a stable-handle load path and revision-aware Document Save and Save As that
  preserve dirty state on conflict or failure and adopt a new path only after
  commit;
- strict refusal for final links, read-only destinations, and unconfirmed
  hard-link separation;
- 134 Windows-local workspace tests, 93.13 percent line coverage across the
  expanded workspace trust kernel, and 90.18 percent whole-workspace line
  coverage; and
- a 418-mutant Windows core campaign classified as 270 caught and 148 unviable,
  plus a clean 58-mutant Windows native-adapter pass classified as 40 caught
  and 18 unviable.

The historical production-adapter checkpoint passes Windows, macOS, and Linux
CI. The subsequent immutable Unix snapshot repair passes local Windows and
native Linux tests plus cross-target macOS lint, but still needs one
exact-commit hosted platform run. It also requires the manual metadata and
weaker-filesystem evidence named by ADR-003 plus the reproducible benchmark
baseline. Noter does not yet have the edit transaction model, complete dirty
lifecycle, recovery, complete commands, configuration, accessibility evidence,
or release performance evidence. M1 therefore remains In Progress.

## 2. Architectural principles

1. **One authoritative revision:** Content and file metadata have one owner.
   UI caches and worker results carry a revision and cannot silently win over a
   newer state.
2. **Pure decisions, effectful edges:** Lifecycle, conflict, undo, and command
   decisions are pure library code. Filesystem, dialogs, clocks, process IDs, and
   GUI events are adapters.
3. **Commit points are explicit:** Save and recovery distinguish pre-commit
   failure, committed success, and committed success with a durability warning.
4. **No hidden transformation:** Encoding, EOL conversion, formatting, and
   conflict resolution are explicit commands.
5. **Accessibility is architecture:** IME and accessibility semantics are part
   of the editor engine contract, not release polish.
6. **Measured scope:** Performance claims name a corpus, percentile, machine,
   build, and measurement method.
7. **Small trusted core:** The code that can lose or rewrite text remains
   UI-independent, dependency-light, and heavily tested.

## 3. Target module boundaries

```text
src/
  lib.rs
  core/
    document.rs       content, revisions, encoding, EOL policy
    edit.rs           edit transactions, selection, caret
    undo.rs           bounded history and coalescing
    lifecycle.rs      Save / Discard / Cancel state machine
    recovery.rs       recovery records and pure decisions
    conflict.rs       file identity and conflict decisions
  io/
    mod.rs            storage traits and save orchestration
    native.rs         platform replacement implementation
    state.rs          durable config and recovery storage
  app/
    state.rs          UI-independent application state
    command.rs        command dispatch and effect requests
  config.rs
  error.rs

src/main.rs           process bootstrap only
src/app.rs            eframe adapter
src/ui/               menus, bars, dialogs, editor adapter
src/platform/         shortcuts, theme events, native integration
```

The library cannot depend on egui, eframe, rfd, or other GUI types. The binary
may depend on the library. Platform-specific unsafe code, if eventually needed,
is isolated in a small adapter crate or module with a documented safety
contract. The workspace-wide default remains `unsafe_code = "forbid"`.

## 4. Domain model

### 4.1 Identity and revisions

```rust
struct DocumentId(UuidLike);
struct Revision(u64);

struct Document {
    id: DocumentId,
    content: Rope,
    revision: Revision,
    saved_revision: Option<Revision>,
    source: Option<FileSource>,
    encoding: Encoding,
    eol: LineEndingProfile,
}

struct FileSource {
    display_path: PathBuf,
    resolved_path: PathBuf,
    identity: FileIdentity,
    saved_fingerprint: ContentFingerprint,
}
```

Every content mutation increments `revision`. A document is dirty when its
current revision or serialized bytes differ from the last committed snapshot.
Path selection alone does not make content clean.

`DocumentId` and recovery instance IDs use collision-resistant random values.
The implementation may use a narrowly configured UUID crate or an equivalent
OS random source after dependency review.

### 4.2 Positions and selections

Core positions use rope character indexes with validated UTF-8 boundaries.
Public editor commands use a strong `TextPosition` type instead of mixing byte,
character, grapheme, line, and visual-column indexes.

```rust
struct TextPosition {
    char_index: usize,
    affinity: Affinity,
}

struct Selection {
    anchor: TextPosition,
    head: TextPosition,
}
```

The status bar reports one-based logical line and Unicode-scalar column in v0.1.
Grapheme-aware visual movement belongs to the editor adapter and is tested with
combining marks, emoji sequences, CJK, and bidirectional samples.

### 4.3 Encoding

v0.1 supports strict UTF-8 with optional UTF-8 BOM:

1. Read bytes without conversion. Refuse an announced size above 64 MiB and
   enforce the same limit plus one sentinel byte while reading so concurrent
   growth cannot bypass the bound.
2. Detect and remove only the exact UTF-8 BOM prefix.
3. Validate the remaining bytes with strict UTF-8.
4. On failure, keep the source untouched and return a typed error containing a
   safe byte offset, not document contents.
5. Serialize the recorded BOM followed by exact UTF-8 content bytes.

An explicit lossy import, if later added, creates a new untitled dirty document
and requires Save As. It cannot inherit the source path.

### 4.4 Line endings

The current single `LineEnding` field becomes:

```rust
enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

enum LineEndingProfile {
    None { insertion: LineEnding },
    Uniform { ending: LineEnding, count: usize },
    Mixed {
        counts: LineEndingCounts,
        insertion: LineEnding,
    },
}
```

Existing newline bytes remain inside the authoritative content. Loading and
saving an untouched document is therefore byte-identical.

Insertion policy:

- A uniform document inserts its uniform ending.
- A new or newline-free document uses CRLF on Windows and LF elsewhere.
- A mixed document uses the nearest preceding existing ending, then the nearest
  following ending, then its deterministic dominant ending.
- Pasted logical newlines use the insertion policy at the insertion point.
- Existing unrelated lines are never rewritten.

An explicit Convert Line Endings command rewrites the full document in one edit
transaction and is independently undoable. Ties in dominant-ending detection
use first occurrence, making the result deterministic.

## 5. Edit and undo architecture

All mutations are `EditTransaction` values:

```rust
struct EditTransaction {
    base_revision: Revision,
    edits: Vec<TextEdit>,
    selection_before: Selection,
    selection_after: Selection,
    origin: EditOrigin,
    timestamp: MonotonicInstant,
}

enum TextEdit {
    Replace {
        range: TextRange,
        inserted: RopeSliceOwned,
        removed: RopeSliceOwned,
    },
}
```

The transaction validator rejects stale revisions, invalid ranges, overlap, and
non-boundary positions before mutating content. Applying a transaction returns
its exact inverse.

Undo history is bounded by both transaction count and retained byte cost.
Coalescing is a pure policy over origin, time, adjacency, selection, and edit
shape. Paste, Replace All, EOL conversion, recovery acceptance, and Markdown
formatting never coalesce with ordinary typing.

A simple reference model based on `String` is used in property tests. Random
valid edit sequences must produce identical content and selection in the
reference and rope implementations, including after every undo and redo.

## 6. Durable file I/O

### 6.1 Storage boundary

Filesystem operations are injected:

```rust
trait Storage {
    type Temporary;

    fn inspect(&mut self, path: &Path, stage: SaveStage)
        -> Result<TargetState, StorageError>;
    fn create_unique_sibling(&mut self, path: &Path)
        -> Result<Self::Temporary, TemporaryCreationFailure>;
    fn write_all(&mut self, temp: &mut Self::Temporary, bytes: &[u8])
        -> Result<(), StorageError>;
    fn flush(&mut self, temp: &mut Self::Temporary) -> Result<(), StorageError>;
    fn apply_metadata(
        &mut self,
        temp: &mut Self::Temporary,
        destination: &Path,
        source: Option<&FileObservation>,
    )
        -> Result<(), StorageError>;
    fn sync_file(&mut self, temp: &mut Self::Temporary) -> Result<(), StorageError>;
    fn replace(
        &mut self,
        temp: Self::Temporary,
        destination: &Path,
        expected: TargetState,
    ) -> ReplaceOutcome<Self::Temporary>;
    fn sync_parent(&mut self, destination: &Path) -> DurabilityOutcome;
    fn discard(&mut self, temp: Self::Temporary) -> Result<(), StorageError>;
}
```

The test adapter can fail every operation before and after the replacement
commit point. The current matrix proves original-byte preservation at initial
inspection, unique creation, partial write, flush, metadata, file sync,
revalidation, and proven non-committing replacement failures. Production code
never infers commit state from a generic I/O error.

### 6.2 Save protocol

1. Capture an immutable `SaveSnapshot` containing document ID, revision,
   serialized-byte iterator, target, expected file identity, fingerprint, BOM,
   and EOL profile.
2. Revalidate the target. If an external revision is present, return Conflict
   before creating a temporary file.
3. Create an unpredictable sibling with create-new semantics. Never reuse or
   truncate a guessed temporary name. If native identity inspection fails after
   creation, preserve the primary error and report a distinct cleanup warning
   naming the random sibling when handle-bound removal is unavailable.
4. Stream all bytes and flush user-space buffers.
5. Validate the existing target's identity, metadata source, and read-only
   policy without widening private staging access. Refuse a Unix snapshot above
   4,096 extended attributes or 64 MiB of aggregate xattr names and values
   before allocating a value buffer.
6. Sync the temporary file's private data and metadata.
7. Revalidate target identity immediately before replacement.
8. On Windows, reserve the recovery-backup name before closing the staging
   handle, then immediately revalidate the closed sibling's native identity,
   length, and BLAKE3-256 fingerprint. Postcommit verification detects a
   same-authority change in the remaining validation-to-replacement window and
   classifies the result as indeterminate.
9. Replace atomically without deleting the destination first.
10. On Unix existing-file commits, verify the displaced original after exchange,
    compare its stable ownership, mode, ACL, and extended attributes with the
    immutable metadata snapshot ratified before commit, apply the snapshot only
    on an exact match, then sync again.
11. Reconcile platform results whose documented failure may have side effects.
12. Sync the parent directory where the platform provides a meaningful operation.
13. Report the exact outcome and either clean by handle or retain the artifact
    with an explicit warning.

On Windows, the adapter uses `ReplaceFileW` with a random same-volume backup and
no ignore-merge flags for existing destinations. It uses `MoveFileExW` with
only `MOVEFILE_WRITE_THROUGH` for absent destinations, so it cannot replace or
copy across volumes accidentally. On Unix, it opens the sibling parent and uses
an atomic exchange for existing-file replacement. The displaced destination
remains at the temporary path after its identity, fingerprint, length, and link
count are checked. The exchange can legitimately change that inode's `ctime`,
so the post-exchange observation ratifies the new token without treating it as
the source of metadata to apply. The displaced file's stable metadata payload
must still equal the immutable snapshot captured and revalidated before commit.
A mismatch leaves the committed file private and reports a warning instead of
restoring stale permissions. Portable Unix APIs cannot tie
deletion atomically to that verified open object, so the artifact is retained
with a warning that names only the random sibling basename and gives inspection
and removal guidance. Absent-file installation uses `RENAME_NOREPLACE` where
available. Its no-overwrite hard-link fallback also retains the temporary name
with the same actionable warning instead of unlinking by pathname. Windows
cleanup opens the verified object without write sharing, then marks that exact
handle for deletion. Unix synchronizes the opened parent; Windows reports
file-only durability because it exposes no equivalent directory barrier here.

Save outcomes are explicit:

```rust
enum SaveOutcome {
    Committed {
        revision: Revision,
        durability: Durability,
        observation: FileObservation,
        warnings: SaveWarnings,
    },
    Conflict { /* expected, actual, cleanup */ },
    NotCommitted { /* error, cleanup */ },
    CommitStateUnknown { /* reconciliation error, recovery artifact */ },
}
```

If replacement commits but post-commit file or directory sync fails, the result
is committed with every durability warning preserved. A file-barrier failure
downgrades the receipt to Best Effort even when parent synchronization succeeds.
The UI must not tell the user nothing was written. A successful older snapshot
does not clear a newer dirty revision. An uncertain commit keeps dirty state and
recovery and blocks blind retry until paths are reconciled. Its typed recovery
warning names only the random artifact basename and states how to inspect,
recover, retry, and remove it safely.

### 6.3 Metadata and symlinks

ADR-003 resolves these policies, and M1 verifies them through platform tests:

- preserve ordinary permissions and supported ownership, ACL, alternate stream,
  quarantine, and extended-attribute semantics where the platform permits;
- never replace a symlink directory entry accidentally;
- revalidate a previously resolved regular-file target before commit;
- refuse an ambiguous or changed link with a safe Save As path;
- document weaker durability on network, cloud, removable, and unusual
  filesystems.

The conservative v0.1 policy refuses a final symlink or Windows reparse point
for both Open and Save As. This remains stricter than following a link until a
resolved-target identity model and its platform fixtures exist. A file with
multiple hard links requires explicit confirmation that atomic replacement
updates only the selected directory entry. Save and Save As surface that
confirmation in the GUI and explain that other names keep the previous revision.
Save As stores the pre-dialog `TargetExpectation` inside an opaque preparation;
confirmation consumes that exact value, so a rebound selection conflicts rather
than being silently re-inspected and adopted.
Read-only files are not made writable implicitly. New Unix files remain
owner-only at mode 0600. Windows temporary and new files use a protected DACL
granting full control only to the object owner and SYSTEM and deny competing
write handles while owned, so permissive parent entries never expose or modify
staged document bytes. Existing
Windows replacements still receive the destination metadata merged by
`ReplaceFileW`. Existing Unix replacements remain mode 0600 through the exchange;
the adapter captures required metadata into an immutable snapshot before commit,
verifies the displaced original after exchange, compares its stable metadata
payload with that snapshot, then applies the snapshot to the committed open
handle only on an exact match. A metadata-finalization failure or final-window
metadata change is a committed warning and leaves the destination at the safest
access state reached, never a false not-committed result.
Extended-attribute capture is limited to 4,096 entries and 64 MiB of aggregate
names and values. Native size queries enforce the limit before value allocation
and retry only within a fixed bound when metadata changes. macOS resource forks
are covered by the same xattr budget. Its private carrier stores only the ACL;
resource-fork and other xattr values are applied from the bounded immutable
snapshot rather than copied live.

No implementation may claim durable atomic save while these cases are silently
undefined.

## 7. Lifecycle, recovery, and conflicts

### 7.1 One destructive-action state machine

`New`, `Open`, `Reload`, `Close`, and `Quit` produce a
`DestructiveIntent`. If the document is dirty, the pure application state emits
`NeedsDirtyDecision`. Save success continues the original intent, Discard
continues after explicit confirmation, and Cancel returns to editing.

Dialogs cannot directly mutate document state. They emit commands to the same
dispatcher used by keyboard shortcuts and menus. Repeated close events are
idempotent while a decision is open.

### 7.2 Recovery storage

Recovery uses the application state or local-data directory returned by
`directories::ProjectDirs`. Each document has a private versioned record:

```text
magic
schema_version
document_id
instance_id
revision
created_at
updated_at
original_path_metadata
encoding_and_eol_profile
selection
content_length
content_checksum
content_bytes
```

Records are written with the durable state-file protocol and restrictive
permissions. The recovery worker receives immutable revision snapshots. It
acknowledges a persisted revision back to application state, and stale
acknowledgements are ignored.

Scheduling uses a 2-second idle debounce plus a 15-second maximum interval while
dirty. A clean close after Save or explicit Discard removes the owned record. A
recovery write failure is a visible warning and never permits silent close.

Startup scans only Noter's own versioned state directory. Valid orphan records
are offered before a normal untitled document. Unknown versions and checksum
failures are quarantined with a non-destructive explanation.

### 7.3 External changes

`FileObservation` combines platform file identity, length, modified time, and a
content fingerprint. Focus regain and a bounded focused timer inspect metadata.
A changed observation triggers a full confirmation before reload or overwrite.

The conflict UI initially offers:

- Reload Disk Version, guarded by the dirty decision state machine;
- Keep Editing, which does not authorize overwrite;
- Save As;
- Overwrite Disk Version only behind an explicit second confirmation showing
  that the disk version will be replaced.

True concurrent merge is out of scope.

## 8. Application and command architecture

`AppState::reduce(Command) -> Vec<Effect>` is the testable center. Commands
include New, OpenRequested, OpenCompleted, Edit, SaveRequested, SaveCompleted,
DirtyDecision, RecoveryPersisted, ExternalObservation, and window events.

Effects include ShowOpenDialog, ShowSaveDialog, LoadFile, SaveSnapshot,
PersistRecovery, InspectFile, ShowMessage, and CloseWindow. Effect completions
carry correlation IDs and revisions so duplicate, cancelled, and stale results
are harmless.

Every menu item and shortcut maps to the same `CommandId` table. Enabled state,
label, platform shortcut, and help text derive from that table. A command that
has no behavior is absent.

## 9. GUI and editor strategy

### 9.1 M4 correctness adapter

The built-in egui `TextEdit` remains a bounded alpha adapter. Because its
`TextBuffer` contract requires a contiguous string, the adapter maintains a
revision-tagged `String` view cache, derives the changed range, and submits an
`EditTransaction` to the authoritative core.

This path is for workflow correctness, UI automation, and dogfooding on ordinary
files. It is not evidence for the 50 MiB performance contract.

### 9.2 M5 feasibility gate

A one-week vertical slice must demonstrate:

- authoritative rope edits with no full-document copy per frame;
- visible-row layout with bounded caches and long-line limits;
- caret, selection, hit testing, horizontal scrolling, and expected movement;
- IME pre-edit rendering and candidate-window placement;
- accessibility text runs, selection, caret, and editable actions;
- search and styled-source ranges;
- deterministic frame-time instrumentation.

The slice must pass correctness tests, 1 MiB interaction budgets, one real CJK
IME, NVDA or another real screen reader, and show a measured route to 50 MiB.
Failure means retaining the correctness adapter with reduced performance claims
or explicitly evaluating another GUI and text stack. Accessibility or IME
cannot be traded away for throughput.

### 9.3 Production renderer

Only after the gate passes:

- lay out visible logical and wrapped rows plus bounded overscan;
- key caches by document revision, line identity, width, font, and theme;
- bound galley, highlight, undo, search, and worker memory;
- handle pathological single lines without allocating proportional per-frame
  layout state;
- keep file I/O and indexing off the render thread;
- cancel or ignore stale work by revision;
- publish p50, p95, and p99 results on named hardware.

## 10. IME and accessibility

The platform adapter preserves the full IME lifecycle: enabled, pre-edit text,
pre-edit cursor range, commit text, and disabled. Pre-edit does not enter undo or
recovery until commit. Candidate-area coordinates update whenever the caret or
viewport moves.

The editor accessibility node exposes:

- editable text value or bounded text runs;
- caret and selection;
- line and character navigation;
- replace-selection and set-selection actions;
- read-only, modified, and validation state;
- find, recovery, error, and conflict announcements.

Semantic tests use the current egui testing and accessibility tree tooling.
Manual release tests cover NVDA, VoiceOver, Orca, CJK IME, dead keys, emoji,
combining marks, bidirectional samples, keyboard-only use, high contrast, and
125 to 200 percent scaling.

## 11. Configuration, state, and theme

Configuration has a versioned schema with conservative defaults:

```toml
schema_version = 1
theme = "system"
font_size = 15.0
word_wrap = true
show_line_numbers = false
recent_files = []

[window]
width = 900
height = 700
maximized = false
```

Unknown fields are preserved where practical; invalid values are diagnosed and
replaced individually rather than discarding the whole file. Config and recent
files use the durable state-file writer. Recovery content never enters config.

Theme preference is System, Light, or Dark. System changes are delivered by
platform events where available and checked on focus elsewhere. Contrast,
selection, focus, disabled state, and error state are tested, not selected only
for appearance.

## 12. Markdown v0.2

Markdown is source-first and operates on immutable revision snapshots after
v0.1. The user can choose Source, Preview, or synchronized Split Preview.
Source is the only editable authority; view changes and preview rendering never
change document bytes.

1. Parse the ratified CommonMark dialect off the UI thread.
2. Tag syntax spans and diagnostics with the source revision.
3. Apply only non-mutating styling and diagnostic results that still match.
4. Transform parser output into a restricted native document model. Do not
   render arbitrary HTML or host the preview in a webview.
5. Build explicit fixes and selection-aware Bold, Italic, Strikethrough, Inline
   Code, Link, Heading, Quote, List, Task List, and Code Fence commands as
   `EditTransaction` values. Every command has an accessible menu path, a
   documented keyboard path where assigned, and exactly one undo step.
6. Map stable source block ranges to preview blocks. Split Preview scroll
   synchronization uses those mappings, ignores stale revisions, and never
   infers an edit from preview position.
7. For Format, produce a diff, parse before and after, reject semantic
   differences, preserve BOM and EOL policy, and apply one undo transaction only
   after confirmation.

Remote images, link fetching, HTML execution, and implicit formatting have no
implementation path. Markdown disabled schedules no parser work.

## 13. Performance and concurrency

Noter uses bounded worker threads and message passing, not a general async
runtime, unless later evidence justifies one. Work items carry cancellation and
revision tokens. The render thread never waits for disk sync, full-file search,
recovery serialization, or Markdown parsing.

The benchmark corpus contains:

- empty and 1 MiB ordinary prose;
- 50 MiB source-like and log files;
- newline-only input;
- mixed Unicode and mixed EOL;
- one pathological long line;
- early, middle, late, absent, and adversarial search matches.

Benchmark reports include OS, CPU, memory, storage, display refresh, build
profile, commit, corpus checksum, sample count, warm or cold state, and raw data.

## 14. Errors, diagnostics, and privacy

Internal errors are typed by operation and commit state. User messages answer:

1. What failed?
2. Was the original file changed?
3. Is current work still in memory or recovery?
4. What is the safest next action?

Tracing is off or minimal by default. Logs never include document content,
clipboard content, recovery bytes, or full paths unless a user explicitly
exports a diagnostic bundle and previews it. The application has no network
client, updater, remote fonts, remote images, or telemetry endpoint.

## 15. Verification architecture

### 15.1 Automated layers

1. Unit tests for parsing, policies, state transitions, and errors.
2. Golden fixtures for BOM, EOL, Unicode, empty, whitespace, trailing newline,
   invalid UTF-8, long lines, and replacement outcomes.
3. Property tests for byte round-trip, line classification, edit equivalence,
   undo and redo, config round-trip, lifecycle safety, and recovery scheduling.
4. Model tests comparing reducers and edit operations with simple reference
   implementations.
5. Injected I/O failures at create, write, flush, sync, metadata, revalidation,
   replacement, and parent sync.
6. Mutation testing for serialization, replacement decisions, lifecycle, undo,
   and recovery validation.
7. Semantic UI tests for command reachability, enabled state, dialogs, status,
   focus, accessibility values, and stale-effect rejection.
8. Child-process crash tests for save and recovery boundaries.
9. Runtime network inspection and dependency audits.

### 15.2 Coverage

- Development: at least 80 percent line coverage for testable product code.
- Trust kernel: at least 90 percent line coverage.
- v0.1: at least 80 percent whole-workspace line coverage.

Coverage exclusions require a nearby rationale and a replacement method. A high
percentage does not replace property, mutation, fault, semantic UI, or manual
testing.

### 15.3 Manual evidence

Each release candidate copies [manual-test-matrix.md](manual-test-matrix.md),
records commit and environment, and checks every applicable item on Windows,
macOS, X11, and Wayland. IME, screen readers, real dialogs, display scaling,
installers, portable use, and dogfooding cannot be inferred from compilation.

## 16. Dependency and release governance

Each direct dependency records:

- requirement and imported surface;
- exact features and default-feature decision;
- release and maintenance health;
- license and advisory state;
- transitive and duplicate impact;
- build-script, native-code, filesystem, process, and network capability;
- debug and release binary-size delta;
- removal or replacement strategy.

M1 adds `proptest` 1.11 as a development-only dependency for shrinking
counterexamples to byte-round-trip, line-ending, edit, and state-machine
invariants. Default features are disabled and only `std` is enabled, excluding
the unneeded fork, timeout, and bit-set surfaces. Its Rust requirement is below
Noter's pinned toolchain, its license is MIT or Apache-2.0, and it adds nothing
to release binaries. It can be removed only if an equivalent shrinking
property-test harness replaces the required invariant suites.

The addition resolves eight test-only packages, moving the cross-target lock
graph from 325 to 333 packages. The audited feature path is only
`proptest/std`; it does not enter the normal dependency graph. The accompanying
product code changes moved the measured Windows release binary from 4,748,800
to 4,749,312 bytes, remaining 4.53 MiB.

M1 uses the official `blake3` 1.8 implementation for collision-resistant saved
content fingerprints. Noter imports only the 32-byte hash and streaming reader
surface. Default features are disabled and only `std` is enabled; mmap, Rayon,
serde, digest traits, and zeroization integrations remain absent. The crate is
licensed under CC0-1.0, Apache-2.0, or Apache-2.0 with LLVM exception. Its
published metadata does not declare an MSRV, so compatibility is established
by the pinned 1.97.1 build and the operating-system CI matrix rather than an
unsupported assumption.

The crate includes a build script and optimized assembly or C SIMD paths on
supported targets. That native build-time surface is accepted for bounded,
single-threaded streaming performance; it adds no Noter runtime filesystem,
process, or network operation. The `pure` feature is intentionally not used
because upstream documents it as unstable and intended for testing. The
addition resolves four lock-graph packages, moving the graph from 333 to 337.
Removal requires another audited 256-bit cryptographic digest, migrated
fingerprint versioning, and equivalent streaming and reference-vector tests.

Stable Rust 1.97.1 exposes Unix device, inode, and hard-link values directly,
but its full Windows by-handle identity methods remain unstable. The internal
`noter-platform` workspace crate therefore owns narrowly scoped unsafe FFI
boundaries for Windows private creation and descriptor lifecycle, file identity
and change observation, cleanup, replacement, and exclusive movement; Unix
descriptor-based extended-attribute reads; and macOS ACL copy and serialization.
Each boundary has a local safety contract, the main crate still forbids unsafe
code, and the platform crate denies unsafe code outside those explicit
allowances. Windows
prefers the 128-bit `FILE_ID_INFO` identity, detects an all-zero unsupported ID,
and labels the 64-bit fallback as reduced. The fallback is never silently
represented as preferred. This internal member adds one lock entry but no external package,
bringing the graph to 338 packages.

M1 directly uses `getrandom` 0.4 only to fill the 16-byte private sibling-name
nonce from the operating system's preferred random source. No optional features
are enabled. The crate is MIT or Apache-2.0, declares Rust 1.85, and was already
present at version 0.4.3 through the development graph, so direct use adds no
lock entry or duplicate. It has no network capability; its intended capability
is narrowly the operating-system entropy interface. Random-source errors and
partial fills are terminal before file creation. The dependency can be removed
only if an equivalent operating-system random API preserves the 128-bit naming
and injected-failure tests.

The production Unix adapter uses `rustix` 1.1 with only `fs` and `std` for
descriptor-relative rename, no-replace rename, link, unlink, ownership, mode,
and full-sync operations. Linux and macOS use `xattr` 1.6 with default features
disabled to apply, remove, and verify visible extended attributes. Bounded
descriptor-based enumeration and value reads use the already-resolved `libc`
0.2 package. Linux also probes POSIX ACL, SELinux, and capability names
explicitly; macOS uses `libc` for ACL copy and serialization. `rustix` and
`libc` were already in the lockfile; `xattr` adds one package, bringing the
cross-target graph to 339. All three are
MIT or Apache-family licensed, have no application network capability, and build
under the pinned toolchain. `xattr` does not publish an MSRV, so CI establishes
compatibility.

With the adapter reachable from the GUI, the stripped Windows release is
4,953,088 bytes, or 4.72 MiB, compared with the 4,748,800-byte M0 baseline. The
2026-07-25 RustSec audit of all 339 locked packages is clean. This measured delta
is accepted for native metadata preservation, cryptographic conflict detection,
and reconciled commit semantics. Later release gates still enforce the 12 MiB
ceiling and require duplicate, license, source, and capability audits.

CI uses the pinned Rust toolchain, locked Cargo graph, immutable action commits,
minimum permissions, formatting, strict Clippy, cross-platform tests, coverage,
full trust-kernel mutation testing, and documentation checks. Release work adds
license and advisory audits, SBOM, provenance, checksums, signatures where
credentials exist, and cargo-dist artifacts verified on clean systems.

## 17. Failure modes and effects

| ID | Failure mode | Severity | Required control | First gate |
| --- | --- | ---: | --- | --- |
| F1 | Partial or truncated save replaces original | 10 | pre-commit fault injection and durable replacement | M1 |
| F2 | Older save clears newer dirty revision | 10 | revision-tagged snapshot and completion tests | M1 |
| F3 | Encoding, BOM, or EOL changes silently | 9 | strict loader, byte goldens, property tests | M1 |
| F4 | Dirty lifecycle discards content | 10 | one pure state machine and model tests | M3 |
| F5 | Recovery is missing, corrupt, or belongs elsewhere | 9 | version, checksum, identity, quarantine, crash tests | M3 |
| F6 | External revision is overwritten | 9 | identity revalidation and explicit conflict decision | M3 |
| F7 | Undo restores the wrong content or selection | 8 | inverse transactions and reference-model tests | M2 |
| F8 | IME commit corrupts or duplicates text | 9 | composition model, real IME matrix, semantic tests | M4/M5 |
| F9 | Screen-reader user cannot inspect or edit text | 8 | accessibility contract and real reader matrix | M4/M5 |
| F10 | Long line or large file freezes the UI | 7 | bounded layout, cancellation, adversarial benchmarks | M5 |
| F11 | Config corruption prevents startup | 5 | versioning, per-field fallback, durable state writes | M4 |
| F12 | Sensitive content enters logs | 8 | redaction tests and diagnostic review | Every gate |
| F13 | A dependency introduces network behavior | 9 | feature audit and runtime traffic inspection | Every gate |
| F14 | Markdown formatter changes meaning | 9 | diff preview, parse equivalence, idempotence, undo | M7 |
| F15 | Platform reports failure after changing replacement state | 10 | explicit unknown-commit outcome, backup-aware reconciliation, recovery retention | M1 |
| F16 | Atomic replacement silently drops permissions or extended metadata | 9 | platform metadata fixtures and pre-commit refusal on preservation failure | M1 |
| F17 | Save replaces a symlink entry or surprises other hard links | 9 | link identity revalidation, Save As refusal, hard-link confirmation | M1/M3 |
| F18 | Resource-fork or extended-attribute metadata exhausts memory or temporary storage | 7 | preallocation byte and count limits, bounded retries, ACL-only macOS carrier | M1 |

The FMEA is updated whenever a test, incident, dependency, or platform behavior
reveals a new path. Critical and high rows require executable evidence or a
clearly accepted residual risk before release.

## 18. Architecture decisions

- **ADR-001:** egui remains the GUI shell, but the production editor is gated by
  IME, accessibility, and performance evidence.
- **ADR-002:** v0.1 uses strict UTF-8, preserves existing newline bytes, and uses
  an explicit deterministic mixed-EOL insertion policy.
- **ADR-003:** durable replacement uses an injected adapter, platform commit
  semantics, identity revalidation, and explicit durability outcomes.

ADR-002 and ADR-003 are accepted. ADR-003 implementation verification remains
in progress until its named platform matrix is green. No custom editor
implementation starts before the M5 feasibility entry criteria are satisfied.
