# Noter Technical Design

**Version:** 0.3

**Reviewed:** 2026-08-04

**Status:** Active architecture contract

This document translates [REQUIREMENTS.md](REQUIREMENTS.md) into an
implementation architecture. [ROADMAP.md](ROADMAP.md) owns sequencing,
[RESEARCH.md](RESEARCH.md) owns external evidence, and architecture decision
records own narrow irreversible choices.

## 1. Current implementation checkpoint

The current development checkpoint has:

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
- native no-follow document opens that bind every read and reopen to the final
  entry and reject link or reparse metadata on the returned handle;
- a narrow internal platform crate that preserves the main crate's unsafe-code
  prohibition while wrapping the required native identity, metadata, commit,
  and synchronization operations;
- 128-bit random, exclusive, owner-tracked sibling files with bounded collision
  handling, owner-only Unix mode, atomic macOS ACL-inheritance suppression, an
  explicit-user-owned and handle-verified protected Windows DACL, strongest
  supported file synchronization, and identity-and-content-safe cleanup;
- a production `FilesystemStorage` adapter with metadata transfer, atomic
  existing-file replacement, exclusive new-file installation, documented
  Windows partial-state reconciliation, parent barriers, and exact destination
  verification;
- a stable-handle load path and revision-aware Document Save and Save As that
  preserve dirty state on conflict or failure and adopt a new path only after
  commit;
- strict refusal for final links, read-only destinations, and unconfirmed
  hard-link separation;
- persisted System, Light, Dark, Green Screen, and Amber Screen themes plus a
  source-backed native Markdown slice with formatted direct editing and
  conservative diagnostics;
- one revision-checked edit authority for Text Mode and Markdown Mode, explicit
  operation intent, exact inverse transactions, directional selections,
  content-identity dirty state, and Undo and Redo history bounded to 1,024
  entries and 32 MiB by default;
- bounded deterministic coalescing for adjacent typing, Backspace, and forward
  Delete, with paste, replacement, formatting, and conversion kept as isolated
  transactions;
- a non-modal literal Find and Replace surface with Unicode case matching,
  next, previous, wrap reporting, match counts, explicit selection or document
  replacement scope, and revision-keyed bounded caching;
- Select All in Text Mode and Markdown Mode, directional cross-block selection
  carry, allocation-free mixed-EOL Go To Line in Text Mode, persistent word
  wrap, and document-only keyboard, menu, and pointer zoom bounded from 50 to
  300 percent;
- one pure lifecycle reducer used by dirty New, Open, Reload, Close, and Quit,
  with Save, Discard, and Cancel effects shared by menu and native-close paths
  and correlated to the exact document revision that authorized them;
- current local Rust tests and whole-workspace and trust-kernel coverage above
  their respective 80 and 90 percent gates, with volatile measurements kept in
  the dedicated evidence records rather than duplicated here;
- a 256-candidate exact-commit M3 editing-core mutation campaign with 216
  caught, 40 compiler-unviable, zero missed, zero timed out, and no recognized
  infrastructure failure, recorded in
  [M3_EDITING_EVIDENCE.md](M3_EDITING_EVIDENCE.md);
- a historical 741-candidate supported-platform mutation union with no miss,
  timeout, infrastructure error, or scope gap; and current exact-commit scopes
  of 970 Linux, 939 Windows, and 47 macOS candidates with no miss, timeout, or
  recognized infrastructure failure.

The latest verified implementation checkpoint passes all nine required jobs in
exact-commit run
[30737535516](https://github.com/blisspixel/noter/actions/runs/30737535516)
for commit `76594b89c1967546893cef73569041fa148573a9`. A reproducible local
trust-kernel benchmark baseline now exists; the manual
metadata and weaker-filesystem evidence named by ADR-003, and later M5 GUI and
input benchmarks remain open. The edit foundation still requires complete
navigation and clipboard policy, long-session fixtures, and cross-platform
evidence. Pure recovery scheduling and private recovery storage exist; app
wiring, overwrite-with-second-confirm, accessibility evidence, and release
performance evidence also remain open. M1 through M4 therefore remain In
Progress even where their current implementation slices are substantial.

The local test and coverage measurements above describe the current source
checkpoint, not hosted release evidence. The M1 paragraph identifies the latest
immutable commit whose complete hosted matrix is verified.

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
    saved_revision: Revision,
    content_fingerprint: ContentFingerprint,
    saved_content_fingerprint: ContentFingerprint,
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

Every content mutation, including Undo and Redo, increments `revision`. A
document is dirty when its current serialized-content fingerprint differs from
the last committed fingerprint. `saved_revision` accepts a save result only for
the exact snapshot that initiated it; it is not dirty identity. Path selection
alone does not make content clean.

`DocumentId` and recovery instance IDs use collision-resistant random values.
The implementation may use a narrowly configured UUID crate or an equivalent
OS random source after dependency review.

### 4.2 Positions and selections

Current source transactions and directional selections use validated UTF-8 byte
offsets because the parser and UI adapters map exact source ranges. The
production editor contract still requires a strong `TextPosition` type at its
public navigation boundary instead of mixing byte, character, grapheme, line,
and visual-column indexes.

```rust
struct TextPosition {
    char_index: usize,
    affinity: Affinity,
}

struct Selection {
    anchor: usize,
    active: usize,
}
```

The status bar reports one-based logical line and Unicode-scalar column in v0.1.
Pure caret policy for character, classic word token, logical line, and document
endpoints lives in `core::navigation::move_caret` and returns validated UTF-8
byte offsets. Grapheme-aware visual movement belongs to the editor adapter and
is tested with combining marks, emoji sequences, CJK, and bidirectional samples.

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
    intent: EditIntent,
    observed_at: EditTimestamp,
}

struct TextEdit {
    range: TextRange,
    inserted: String,
    removed: String,
}
```

The transaction validator rejects stale revisions, invalid ranges, overlap, and
non-boundary positions before mutating content. Applying a transaction returns
its exact inverse.

Undo history is bounded by both transaction count and retained byte cost. The
current defaults are 1,024 transactions and 32 MiB across Undo and Redo. A new
branch clears Redo, an edit larger than the byte ceiling clears history that
could no longer apply, and an unexpected revision rejects Undo without changing
the document.

Coalescing is a pure policy over explicit intent, origin, time, adjacency,
selection continuity, and exact edit shape. Adjacent typing, Backspace, and
forward Delete coalesce independently inside an inclusive 750 millisecond
window. A coalesced edit retains at most 16 KiB and can never evade the broader
history byte ceiling. Clock regression, caret movement, origin change, intent
change, non-adjacent ranges, and resource ceilings end the group. Paste,
Replace All, EOL conversion, programmatic replacement, and Markdown formatting
never coalesce with ordinary typing.

Fixed-seed, 512-case `String` reference properties cover arbitrary single
replacements, ordered disjoint multi-edit transactions, edit sequences, and
Unicode typing coalesced into one history step. They compare content and
directional selection after apply and exact inverse, and compare every retained
state after Undo and Redo.

Literal search escapes all regex metacharacters before using the linear-time
regex engine. A shared UI adapter caps each frame's focused text, paste, and IME
payload before widget processing. A bounded text buffer then enforces the exact
UTF-8 byte ceiling at every mutation, including Enter, Tab, and replacements
after navigation changes the selection. Queries and replacements are each
capped at 16 KiB. Match counting retains no document-sized range vector, and
the UI cache is keyed by document revision, query, and case policy.
Unicode-insensitive matching uses the engine's simple case folding and returns
source byte ranges. Replace and Replace All use literal replacement text,
reject invalid UTF-8 scopes, calculate the result length before allocation,
enforce the BOM-aware 64 MiB serialized-document ceiling, and enter the same
transaction history with explicit Replace intent.

Go To Line caps its focused input at 20 UTF-8 bytes before widget processing,
then scans source bytes without allocation and treats LF, CRLF, CR, and mixed
files exactly. An empty document has one addressable line and a final terminator
starts a trailing empty line. Its dialog state is discarded when New or Open
replaces the document and whenever the application leaves Text Mode, so an
unavailable command cannot retain stale input or focus. Select All in either
view and Text Mode Go To Line
restore the exact source selection through the same editor-state boundary used
by Undo and Find. Markdown restoration accepts any in-bounds UTF-8 source
selection, retains its direction, and activates one contiguous source-backed
edit region even when parsed blocks or native line-ending forms differ. Word
wrap changes only Text Mode layout. Keyboard, menu, and supported
pointer magnification over the document surface scale document type, including
native Markdown headings, from 50 to 300 percent without changing menus, status
controls, source bytes, or revision identity. The Markdown document bar also
maps vertical wheel motion over its live percentage to one bounded zoom command
per frame; clicking that value resets to 100 percent. Horizontal-only,
non-finite, and zero motion do nothing. Wrap and zoom preferences accept only
canonical bounded persisted values.

Optional spell checking is an M5 platform adapter, not a network service or a
Markdown rewrite. The preference is explicit and off when no supported local
provider exists. An adapter accepts a BCP 47 language, bounded visible text,
and an exact document revision; it returns ranges and suggestions tagged with
that revision. Stale results are discarded. Suggestions never replace text
without a user action, and no adapter may upload, retain, or train on document
content. Windows and macOS use their native local spell services where
available. A Linux provider must be installed locally, capability-checked, and
covered by the same privacy and unavailable-provider tests before support is
claimed.

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
   truncate a guessed temporary name. If native identity inspection or required
   platform privacy finalization fails after creation, preserve the primary
   error and report a distinct cleanup warning naming the random sibling when
   handle-bound removal is unavailable.
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
    on an exact match, restrict that same open displaced object to owner-only
    access before retaining it, then sync again.
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
restoring stale permissions. After an exact match, the verified displaced object
is restricted through its live descriptor to mode 0600. macOS also removes the
access-control list and verifies its absence. Restriction failure is a committed
cleanup warning, never a false owner-only claim. Portable Unix APIs cannot tie
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

An indeterminate outcome stops every later Save and Save As before destination
inspection or mutation. This global fail-closed boundary avoids treating a
pathname comparison as authority while a parent namespace can be renamed,
replaced, or rebound. New and Open may replace the visible document but never
release retained recovery evidence. Dismissing a notice only hides the notice;
attempting any save resurfaces every active record. Each record exposes a
bounded parent-and-name label and an explicit Copy Destination Path action. A
non-Unicode operating-system path is never copied through lossy Unicode text;
the action is relabeled and copies a reversible hexadecimal byte or UTF-16
representation. Reconcile opens a confirmation that repeats the diagnostic and
path-copy action and instructs the user to inspect the destination and retained
private sibling and preserve the needed version. Confirmation removes exactly
that in-memory record and performs no write, retry, or document mutation.
Removing the last record clears only the stale save-block error and re-enables
the save commands.

Before any save that could become indeterminate, the application reserves the
vector slot and all record-owned text. The selected destination is limited to
128 KiB in its platform encoding, its display label to 1 KiB, and its diagnostic
to 4 KiB. The unknown-outcome path streams details into the reserved diagnostic
without formatting an intermediate string. The ledger retains at most 16
records, never evicts active evidence, and renders inside bounded scroll
regions. Save availability is an in-memory constant-time check, so repaint does
not inspect or canonicalize filesystem paths. Durable restart-spanning records
remain M4 work.

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
Final-entry classification is not separated from a following content open. Unix
opens with `O_NOFOLLOW`; Windows opens the final entry itself with
`FILE_FLAG_OPEN_REPARSE_POINT`. Both paths inspect the returned handle and reject
links, reparse points, directories, and special files before content is read.
The second pathname handle uses the same primitive, so matching identities are
evidence about two no-follow opens rather than two independently followed link
targets.
Read-only files are not made writable implicitly. New Unix files remain
owner-only at mode 0600. Because mode alone does not suppress macOS inherited
ACL entries, macOS requests a zero-entry `no_inherit` ACL and mode 0600 in one
`openx_np` operation. Native execution proves that an ordinary child inherits
the parent ACE while the protected file immediately reports true ACL absence.
macOS canonicalizes the zero-entry creation request rather than retaining an
allocated empty ACL. At runtime, Noter defensively applies the native remove-ACL
sentinel through the live descriptor and verifies absence before any document
bytes are written. Failure
closes the descriptor and reports the possible random zero-byte artifact without
unlinking an unverified pathname. Windows temporary and new files explicitly set
the process user's SID as owner and request a protected DACL granting full
control only to that SID and SYSTEM. Noter verifies the owner and exact DACL
through the created handle before writing. Unsupported or ignored ACL semantics
fail closed, with handle-bound removal of the zero-byte object where supported.
Verified files deny competing write handles while owned, so permissive parent
entries never expose or modify staged document bytes. Existing
Windows replacements still receive the destination metadata merged by
`ReplaceFileW`. Existing Unix replacements remain mode 0600 through the exchange;
the adapter captures required metadata into an immutable snapshot before commit,
verifies the displaced original after exchange, compares its stable metadata
payload with that snapshot, then applies the snapshot to the committed open
handle only on an exact match. It then restricts the verified displaced object
through its open handle to mode 0600 and, on macOS, verified ACL absence. A
metadata-finalization, artifact-restriction, or final-window metadata failure is
a committed warning and leaves the destination at the safest access state
reached, never a false not-committed result or false privacy guarantee.
Extended-attribute capture is limited to 4,096 entries and 64 MiB of aggregate
names and values. Native size queries enforce the limit before value allocation
and retry only within a fixed bound when metadata changes. macOS resource forks
are covered by the same xattr budget. The ACL is serialized into the immutable
snapshot before commit, reconstructed after the exchange, applied through the
destination descriptor, and re-serialized for exact verification. No temporary
ACL pathname is exposed. A source with no extended ACL is represented by the
distinct `Absent` snapshot state and replayed with macOS's native remove-ACL
sentinel. Present ACL entries remain serialized separately. Native evidence also
shows that explicit zero-entry ACL text is canonicalized to absence, so the
design does not claim that an empty ACL remains a separately stored state.
Resource-fork and other xattr values are applied from the bounded immutable
snapshot rather than copied live.

No implementation may claim durable atomic save while these cases are silently
undefined.

## 7. Lifecycle, recovery, and conflicts

### 7.1 One destructive-action state machine

`New`, `Open`, `Reload`, and `Quit` produce a `DestructiveIntent`; native Close
maps to Quit. If the document is dirty, the pure application state emits
`PromptDirty`. Save success continues the original intent, Discard continues
after explicit confirmation, and Cancel returns to editing.

The current `LifecycleState::reduce` implementation has explicit Idle,
Prompting, Saving, and Closing phases. Prompting and Saving retain both the
destructive intent and the exact document revision that produced it. Repeated
requests cannot replace a visible decision, unsolicited or stale save
completions are inert, and a completion for a different revision cannot
authorize abandonment. Dirty or still-interactive save outcomes return to an
explicit decision, a clean save with a blocking warning stops for review, and
only Quit authorizes one native close for the exact saved revision. Exhaustive
transition tests and a fixed-seed 512-case command-sequence property compare
the reducer with a separate reference model. A pure `ConflictState` reducer
classifies focus and timer observations against the trusted save baseline and
offers Reload, Keep Editing, and Save As without silent overwrite.
Overwrite-with-second-confirm is implemented as a pure two-step conflict
decision (`RequestOverwrite` then `ConfirmOverwrite`) that only then rebaselines
the trusted save expectation and saves.

Dialogs cannot directly mutate document state. They emit commands to the same
dispatcher used by keyboard shortcuts and menus. Repeated close events are
idempotent while a decision is open.

### 7.2 Recovery storage

Recovery uses a private subdirectory of the per-user application data root
(never the general temporary directory). Preferences may use eframe storage
(`app.ron`); recovery records do not. The library modules are
`core::recovery` (pure schedule and integrity) and `core::recovery_store`
(durable private files). The binary adapter `crash_recovery` opens
`eframe::storage_dir("Noter")/recovery`, drives the pure scheduler, presents
startup Restore / Discard offers, and surfaces persist failures without writing
user document paths.

Each dirty session owns one versioned record:

```text
magic (NOTERREC)
schema_version
document_id
instance_id
revision
created_at
updated_at
original_path_metadata
bom and encoding tags
selection (UTF-8 body offsets on character boundaries)
content_length
content_checksum (BLAKE3-256)
content_bytes (serialized body including optional UTF-8 BOM)
```

Records stage through exclusive private creation, file sync, and atomic
install or replace. Unix exchange leaves the previous destination on the stage
path; that displaced file is removed after a successful replace so recovery
siblings do not accumulate silently. Windows replacement backups of superseded
recovery content are removed after success.

The pure scheduler uses a 2-second idle debounce and a 15-second maximum
interval while dirty. Persist requests carry a session epoch. Save success and
explicit Discard advance the epoch and request deletion so a late disk
completion cannot reintroduce recovery after clean state. Clock regression
schedules an immediate persist rather than disabling the recovery-point
objective. A recovery write failure is a visible warning and never permits
silent close.

Startup walks the entire records directory. Valid records are offered (at most
32 per launch); surplus valid records remain for a later session. Corrupt or
unsupported records are quarantined; quarantine relocation failures are
reported on the scan entry and leave the damaged file in place rather than
claiming success. Restored content always opens dirty and never writes the
original user path until Save.

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

Noter's product runtime is a compiled Rust desktop binary. eframe supplies the
winit window and event integration, OpenGL rendering through glow, persistence,
and AccessKit bridge; it does not embed a browser engine or WebView. The product
contains no HTML, CSS, JavaScript, or Python execution path. Repository Python
is maintainer-only test, benchmark, legal-inventory, screenshot, and release
automation. It is not invoked by the application or binary installers.

Release-critical repository automation should move to a Rust `xtask` in small,
parity-verified slices when doing so removes a toolchain or materially reduces
total complexity. Language statistics alone do not justify rewriting mature
evidence tooling or weakening its platform behavior.

`AppState::reduce(Command) -> Vec<Effect>` is the testable center. Commands
include New, OpenRequested, OpenCompleted, Edit, SaveRequested, SaveCompleted,
DirtyDecision, RecoveryPersisted, ExternalObservation, and window events.

Effects include ShowOpenDialog, ShowSaveDialog, LoadFile, SaveSnapshot,
PersistRecovery, InspectFile, ShowMessage, and CloseWindow. Effect completions
carry correlation IDs and revisions so duplicate, cancelled, and stale results
are harmless.

Every menu item and shortcut maps to the same `CommandId` table. Enabled state,
label, platform shortcut, and help text derive from that table. A command that
has no behavior is absent. The current alpha adapter applies the same rule
directly to implemented commands; for example, Reload is disabled until the
document owns a filesystem path.

## 9. GUI and editor strategy

### 9.1 M2 correctness adapter

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

Theme preference is System, Light, Dark, Green Screen, or Amber Screen. The two
specialty palettes use the dark rendering path and the same native shaping and
fallback engine while selecting monospace application type, square controls,
and a bounded static CRT glass overlay. The overlay contains at most 1,024
scanlines plus four fixed edge regions and one border; it is noninteractive and
has no timer, document inspection, or network path. Returning to Light, Dark,
or System restores proportional application type and the complete standard
visual state instead of retaining specialty details. System changes are
delivered by platform events where available and checked on focus elsewhere.
Contrast, selection, focus, disabled state, and error state are tested, not
selected only for appearance.

Specialty palettes are complete data values passed through one runtime
validator. Primary, secondary, link, warning, error, selection, active-control,
and outline contrast have explicit thresholds; every color must be opaque. A
rejected extension reconstructs standard Dark visuals instead of applying a
partial or unreadable palette. The future custom-theme loader is declarative
and accepts no scripts, shaders, assets, URLs, commands, or behavior overrides.

## 12. Native Markdown architecture

Markdown Mode and Text Mode share one authoritative Markdown source. Text Mode
shows the exact source. Markdown Mode projects source ranges into native
formatted blocks and maps direct edits back to the smallest practical source
range. Switching modes does not modify bytes.

Rendered characters retain complete UTF-8 source spans. Pointer carets and
drag selections map through those spans, absorbing syntax that is intentionally
hidden from the projection, including supported delimiters, escapes, and
parser-decoded character references. A non-bijective parser event owns its
complete source range only when the displayed characters match a source
substring; otherwise the complete raw source remains visible and directly
editable. This prevents hidden syntax from being split without inventing a
mapping. A mode switch restores the directional source selection before the
destination editor accepts input, and both adapters discard widget-local undo
snapshots after committing to the shared bounded history.

Cross-block primary-pointer selection retains one anchor and one active cursor,
each expressed as absolute source boundaries derived from those same rendered
spans. Native selectable labels own live cross-widget painting. The editor
chooses the nearest rendered block by vertical geometry, performs bounded edge
autoscroll through the containing native scroll area using elapsed frame time,
and activates one contiguous source-backed editor only on release. Forward and
reverse drags use the appropriate leading and trailing source boundaries, so
hidden syntax and multi-byte characters cannot become partial edit positions.
Escape, unreleased pointer loss, or window focus loss clears the transient
selection without changing source bytes. A primary release followed by the
normal touch PointerGone event completes at the retained final interaction
position.

The current M2 slice is deliberately bounded. It parses through
`pulldown-cmark` and builds restricted native egui layout jobs before shaping.
The inactive document and active source-backed editor use the same explicit
body, heading, emphasis, link, and code style mapping. Supported heading and
inline delimiters remain in source while being visually suppressed, and a link
target is revealed only while it is edited. One native paragraph-style
selector exposes Paragraph and all six ATX heading levels; six inline and line
actions remain selection-aware toggles. Style changes are idempotent, preserve
directional selections on no-op, and replace only parser-verified top-level
paragraph or ATX-heading syntax. Indented headings, tab separators, and
optional closing ATX sequences follow the parser's model; code, setext
headings, nested blocks, other unsupported structures, and paragraphs whose
leading whitespace or literal trailing hashes cannot round trip through ATX
syntax report Unavailable and remain byte-exact. Repeating a toggle removes only
a parser-verified simple
construct, while malformed, asymmetric, multi-backtick, and deeper
repeated-star syntax fails closed. Empty-caret commands never remove literal
delimiters. Line commands touch only selected logical lines. Link never invents
label text or a destination, validates the complete candidate through the
parser, and excludes parser-recognized code and inline HTML ranges while
locating the label boundary. Bold, Italic, and Link have focused-editor
keyboard commands. The selector exposes a stable ComboBox name and current
value; toggle buttons expose stable labels and pressed state. The compact
Format control nests paragraph styles instead of placing thirteen actions in
one clipped popup. The responsive top row keeps the primary Mode and Theme
controls opposite the application menus. Markdown uses the same
continuous borderless editor fill as Text Mode, with no page card, border, or
shadow. Content consumes the available canvas with only the ordinary editor
inset instead of a centered page measure. The separate Markdown document bar
keeps whitespace-grouped formatting actions on
the left and a distinct bounded zoom cluster on the right when space permits.
Construction order matches visual order so keyboard focus and
accessibility-tree traversal do not reverse controls. Its reset control exposes
the live percentage as an accessibility value, accepts vertical pointer-wheel
zoom, and preserves control focus without activating a source-backed editor.
Text Mode
exposes every delimiter, and five conservative diagnostics operate directly on
source. A narrowly recoverable line-wide emphasis spacing mistake is projected
with the intended style while MD037 reports that portable Markdown requires
moving the whitespace outside the closing marker. Viewing never changes those
source bytes. This establishes the interaction direction but does not satisfy
M6.

Escape is ordered after active widget input. The editor synchronizes the final
draft, captures the resulting directional source selection, and then removes
the active range. The application records that exact selection in the shared
transaction history, so Undo and Redo restore the post-edit caret instead of a
stale pre-frame selection.

Because the current slice discovers and renders the complete block set
synchronously, it enforces prototype ceilings of 1 MiB of source, 8,192 logical
lines, 64 KiB per line, 512 projected blocks, 64 KiB per block span, and 8,192
parser events. A document that exceeds a Markdown ceiling remains unchanged in
Text Mode when it is within the interface's current 8 MiB file ceiling. The
interface refuses a larger file before constructing its complete widget string,
while the trust-kernel loader retains the independent 64 MiB storage boundary.
Each live active draft is checked against the structural ceilings before
semantic targeting or source-style parsing. Toolbar mutations are deferred
until every visible command state has been derived from the prior bounded
draft. After any draft is synchronized, the complete resulting document is
checked before block discovery. A successful range-local check can therefore
never stand in for the aggregate source, block, or parser-event ceilings. An
over-budget draft receives one plain, bounded layout section for that frame,
commits its exact source through the shared transaction authority, and then
falls back to Text Mode with the specific exceeded budget. This prevents an
adversarial same-frame paste or formatting expansion from reaching the more
expensive formatted layout path first.
Diagnostic counts are cached by document generation and revision. These
ceilings are temporary safety boundaries, not evidence that the final 1 MiB
Markdown latency or 50 MiB text-editing requirements pass.

The M6 architecture completes the model:

1. Parse the ratified CommonMark and selected GFM dialect off the UI thread from
   an immutable revision snapshot.
2. Tag blocks, source ranges, styling, and diagnostics with that revision, and
   discard stale results.
3. Transform parser output into a restricted native document model. Raw HTML is
   inert, remote assets are not loaded, and no webview is used.
4. Express direct formatted edits, source edits, formatting actions, and safe
   fixes as the same minimal `EditTransaction` type used by Text Mode.
5. Preserve unsupported source as opaque editable ranges and reveal source when
   a formatted edit is ambiguous.
6. Give every command an accessible menu path, a documented keyboard path where
   assigned, and exactly one undo step.
7. For whole-document Format, produce a reviewed diff, parse before and after,
   reject unsupported semantic differences, preserve BOM and EOL policy, and
   apply one transaction only after confirmation.

A reading-focused presentation or synchronized split layout may be added later,
but neither defines Markdown support. The primary feature is a directly editable
native Markdown document whose saved representation remains standard source.
Text Mode schedules no Markdown work.

## 13. Performance and concurrency

Noter uses bounded worker threads and message passing, not a general async
runtime, unless later evidence justifies one. Work items carry cancellation and
revision tokens. The current block-focused Markdown slice parses synchronously;
moving that work behind revision-tagged background parsing is an M6 requirement.
At that milestone, the render thread must not wait for disk sync, full-file
search, recovery serialization, or Markdown parsing.

The benchmark corpus contains:

- empty and 1 MiB ordinary prose;
- 50 MiB source-like and log files;
- newline-only input;
- mixed Unicode and mixed EOL;
- one pathological long line;
- early, middle, late, absent, and adversarial search matches.

The framework-backed editor currently mirrors a document as a complete
`String`. A measured 64 MiB open through that widget path peaked at 665.3 MiB on
Windows. Refusing files above 8 MiB before that mirror reduced the same bounded
run to a 196 MiB peak and kept the existing document intact. This local
measurement justifies containment only. M5 still requires either a virtualized,
rope-backed editor or evidence that another bounded design meets the 50 MiB
release corpus without sacrificing IME or accessibility.

Benchmark reports include OS, CPU, memory, storage, display refresh, build
profile, commit, corpus checksum, sample count, warm or cold state, and raw data.
Schema-v2 reference runs execute the recorded clean commit in a detached linked
worktree, use nearest-rank percentiles over at least 30 samples, bind the exact
corpus and build artifacts by SHA-256, and validate canonical bounded JSON
before exclusive promotion. Windows orchestration creates commands suspended,
assigns them to a kill-on-close Job Object, and resumes them only after
association so deadlines and output limits cannot orphan ordinary descendants.
Termination proceeds in bounded waves. Each wave obtains a complete Job Object
process list, retains identity-checked handles, stops the captured members, and
waits for their process objects to signal before rescanning. This closes the
window in which a captured process can create another assigned descendant. A
final Job Object termination and active-count check run under the same fixed
deadline before returning the ordinary output-limit or command-deadline error.
Failure to drain within that deadline is reported separately, and closing the
kill-on-close Job Object remains the final bounded cleanup request.
The first canonical reference and its unauthenticated-local provenance limits
are recorded in [M1_BASELINE_EVIDENCE.md](M1_BASELINE_EVIDENCE.md).

## 14. Errors, diagnostics, and privacy

Internal errors are typed by operation and commit state. User messages answer:

1. What failed?
2. Was the original file changed?
3. Is current work still in memory or recovery?
4. What is the safest next action?

Tracing is off or minimal by default. Logs never include document content,
clipboard content, recovery bytes, or full paths unless a user explicitly
exports and reviews a diagnostic bundle. Editing has no background network
client, remote fonts, remote images, or telemetry endpoint. An explicit update
action may contact only the documented release channel under the constraints in
[PRIVACY.md](PRIVACY.md) and [INSTALLATION.md](INSTALLATION.md).

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
Noter's pinned toolchain. The `proptest` dependency is dual-licensed MIT or
Apache-2.0 and adds nothing to release binaries. It can be removed only if an equivalent shrinking
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
are enabled. The `getrandom` dependency is dual-licensed MIT or Apache-2.0,
declares Rust 1.85, and was already
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
M1 adapter checkpoint to 339 packages. All three are MIT or Apache-family
licensed, have no application network capability, and build under the pinned
toolchain. `xattr` does not publish an MSRV, so CI establishes compatibility.

M3 uses [regex 1.13.1](https://crates.io/crates/regex/1.13.1) only to compile
escaped literal queries, iterate original UTF-8 source ranges, and apply
Unicode-aware simple case matching with linear-time search guarantees. Default
features are disabled; `std`, `perf`, and `unicode-case` are enabled for bounded
compilation, literal acceleration, and the required case policy. Version 1.13.1
is the current registry release from the maintained
[rust-lang/regex](https://github.com/rust-lang/regex) project as reviewed on
2026-07-30. It is MIT or Apache-2.0 licensed, declares Rust 1.65, passes the
repository's advisory and license policy, and builds under Noter's pinned
toolchain.

The selected feature path declares no build script or native code and adds no
application filesystem, process, or network capability. The direct addition
resolves `regex`, `regex-automata`, and `aho-corasick`; `regex-syntax` and
`memchr` were already present, and the locked graph contains no second `regex`
version. The full editing-and-shell slice from exact baseline `d77460c` to the
current local tree increases the all-feature Windows debug binary from
14,124,032 to 16,055,808 bytes and the stripped release binary from 8,296,448
to 9,279,488 bytes. Those conservative increases of 1,931,776 bytes (13.68
percent) and 983,040 bytes (11.85 percent) include every feature and fix in the
slice, so they are upper bounds rather than unsupported attribution to `regex`.
Removal requires an equally bounded literal matcher that returns original byte
ranges, supports the documented Unicode case behavior, and retains linear
worst-case matching.

Those statements describe third-party dependency licenses. Noter itself is
licensed only under Apache-2.0, as declared by both package manifests and the
root [LICENSE](../LICENSE).

The current graph contains 416 cross-target packages after the egui 0.35
upgrade, removal of the redundant secondary Markdown renderer, and addition of
bounded literal search. The current local stripped Windows release is 9,279,488
bytes, or 8.85 MiB, compared with the 4,748,800-byte M0 baseline. The increase
includes native metadata preservation, cryptographic conflict detection,
reconciled commit semantics, current text shaping, the bundled variable
document font, persisted themes, the early native Markdown surface, and the M3
editing controls. Release gates still enforce the 12 MiB ceiling.
The checked-in cargo-deny policy gates licenses, sources, advisories, wildcard
versions, and duplicate-version visibility; a release capability audit remains
required.

CI uses the pinned Rust toolchain, locked Cargo graph, immutable action commits,
minimum permissions, formatting, strict Clippy, cross-platform tests, coverage,
full trust-kernel mutation testing, documentation checks, license policy, and
advisory audits. Release work retains those gates and adds target-specific
SBOMs, provenance, checksums, signatures where credentials exist, and cargo-dist
artifacts verified on clean systems.

## 17. Failure modes and effects

| ID | Failure mode | Severity | Required control | First gate |
| --- | --- | ---: | --- | --- |
| F1 | Partial or truncated save replaces original | 10 | pre-commit fault injection and durable replacement | M1 |
| F2 | Older save clears newer dirty revision | 10 | revision-tagged snapshot and completion tests | M1 |
| F3 | Encoding, BOM, or EOL changes silently | 9 | strict loader, byte goldens, property tests | M1 |
| F4 | Dirty lifecycle discards content | 10 | one pure state machine and model tests | M4 |
| F5 | Recovery is missing, corrupt, or belongs elsewhere | 9 | version, checksum, identity, quarantine, crash tests | M4 |
| F6 | External revision is overwritten | 9 | identity revalidation and explicit conflict decision | M4 |
| F7 | Undo restores the wrong content or selection | 8 | inverse transactions and reference-model tests | M3 |
| F8 | IME commit corrupts or duplicates text | 9 | composition model, real IME matrix, semantic tests | M5 |
| F9 | Screen-reader user cannot inspect or edit text | 8 | accessibility contract and real reader matrix | M5 |
| F10 | Long line or large file freezes the UI | 7 | bounded layout, cancellation, adversarial benchmarks | M5 |
| F11 | Config corruption prevents startup | 5 | versioning, per-field fallback, durable state writes | M4 |
| F12 | Sensitive content enters logs | 8 | redaction tests and diagnostic review | Every gate |
| F13 | A dependency introduces network behavior | 9 | feature audit and runtime traffic inspection | Every gate |
| F14 | Markdown formatter changes meaning | 9 | reviewed diff, parse equivalence, idempotence, undo | M6 |
| F15 | Platform reports failure after changing replacement state | 10 | explicit unknown-commit outcome, backup-aware reconciliation, recovery retention | M1 |
| F16 | Atomic replacement silently drops permissions or extended metadata | 9 | platform metadata fixtures and pre-commit refusal on preservation failure | M1 |
| F17 | Save replaces a symlink entry or surprises other hard links | 9 | link identity revalidation, Save As refusal, hard-link confirmation | M1/M4 |
| F18 | Resource-fork or extended-attribute metadata exhausts memory or temporary storage | 7 | preallocation byte and count limits, bounded retries, pathless serialized macOS ACL snapshot | M1 |

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
