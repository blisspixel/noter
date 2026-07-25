# Noter Technical Design

**Version:** 0.2

**Reviewed:** 2026-07-25

**Status:** Active architecture contract

This document translates [REQUIREMENTS.md](REQUIREMENTS.md) into an
implementation architecture. [ROADMAP.md](ROADMAP.md) owns sequencing,
[RESEARCH.md](RESEARCH.md) owns external evidence, and architecture decision
records own narrow irreversible choices.

## 1. Current implementation checkpoint

The M0 worktree has:

- a binary crate containing the egui shell;
- a library crate containing the UI-independent document module;
- strict UTF-8 loading with BOM and existing newline-byte preservation;
- an interim same-directory write, sync, and rename save path;
- seven document tests and 96.43 percent line coverage in that module.

It does not yet have the durable replacement adapter, edit transaction model,
dirty lifecycle, recovery, external conflict handling, complete commands,
configuration, accessibility evidence, or release performance evidence.
The interim save path is not the final implementation of NFR-REL-02.

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

1. Read bytes without conversion.
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
    Uniform(LineEnding),
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
    type Temp;

    fn inspect(&self, path: &Path) -> Result<FileObservation, StorageError>;
    fn create_unique_sibling(&self, path: &Path) -> Result<Self::Temp, StorageError>;
    fn write_all(&self, temp: &mut Self::Temp, chunks: &mut dyn ByteChunks)
        -> Result<(), StorageError>;
    fn flush(&self, temp: &mut Self::Temp) -> Result<(), StorageError>;
    fn sync_file(&self, temp: &mut Self::Temp) -> Result<(), StorageError>;
    fn apply_metadata(&self, temp: &mut Self::Temp, source: &FileObservation)
        -> Result<(), StorageError>;
    fn replace(&self, temp: Self::Temp, destination: &Path)
        -> Result<ReplaceReceipt, StorageError>;
    fn sync_parent(&self, destination: &Path) -> Result<Durability, StorageError>;
}
```

The test adapter can fail every operation before and after the replacement
commit point. Production code never infers commit state from a generic I/O
error.

### 6.2 Save protocol

1. Capture an immutable `SaveSnapshot` containing document ID, revision,
   serialized-byte iterator, target, expected file identity, fingerprint, BOM,
   and EOL profile.
2. Revalidate the target. If an external revision is present, return Conflict
   before creating a temporary file.
3. Create an unpredictable sibling with create-new semantics. Never reuse or
   truncate a guessed temporary name.
4. Stream all bytes, flush user-space buffers, and sync the temporary file.
5. Copy required permissions and supported metadata from an existing target.
6. Revalidate target identity immediately before replacement.
7. Replace atomically without deleting the destination first.
8. Sync the parent directory where the platform provides a meaningful operation.
9. Report the exact outcome to the application state.

On Windows, the adapter evaluates `ReplaceFileW` for existing destinations and
an atomic move for new destinations. On Unix, it uses same-filesystem rename
after metadata application and parent-directory sync. An audited crate may
replace custom platform code only if its documented behavior and tests satisfy
this contract.

Save outcomes are explicit:

```rust
enum SaveOutcome {
    Committed {
        revision: Revision,
        durability: Durability,
        observation: FileObservation,
    },
    Conflict(Conflict),
    NotCommitted(StorageError),
}
```

If replacement commits but directory sync fails, the result is committed with a
durability warning. The UI must not tell the user nothing was written. A
successful older snapshot does not clear a newer dirty revision.

### 6.3 Metadata and symlinks

M1 closes these policies through ADR-003 and platform tests:

- preserve ordinary permissions and supported ownership, ACL, alternate stream,
  quarantine, and extended-attribute semantics where the platform permits;
- never replace a symlink directory entry accidentally;
- revalidate a previously resolved regular-file target before commit;
- refuse an ambiguous or changed link with a safe Save As path;
- document weaker durability on network, cloud, removable, and unusual
  filesystems.

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

Markdown operates on immutable revision snapshots after v0.1:

1. Parse the ratified CommonMark dialect off the UI thread.
2. Tag syntax spans and diagnostics with the source revision.
3. Apply only non-mutating styling and diagnostic results that still match.
4. Build explicit fixes as `EditTransaction` values.
5. For Format, produce a diff, parse before and after, reject semantic
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

CI uses the pinned Rust toolchain, locked Cargo graph, immutable action commits,
minimum permissions, formatting, strict Clippy, cross-platform tests, coverage,
and documentation checks. Release work adds license and advisory audits, SBOM,
provenance, checksums, signatures where credentials exist, and cargo-dist
artifacts verified on clean systems.

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

ADR-002 is accepted. ADR-003 remains proposed until platform tests close
metadata and symlink semantics. No custom editor implementation starts
before the M5 feasibility entry criteria are satisfied.
