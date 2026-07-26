# M1 Security Review

**Reviewed revision:** `3830cdd6e487a35bdd2adeecb3d45bb080ade114`

**Review date:** 2026-07-25

**Follow-up:** 2026-07-26 macOS staging review and remediation

**Coverage:** Partial repository review with full-file inspection of the runtime
document, observation, save, platform, GUI lifecycle, dependency, and CI paths.

## Scope and method

The review treated user-selected bytes and paths, concurrent filesystem actors,
native API results, dependencies, and build inputs as untrusted. It traced the
document loader and durable-save protocol from user intent through the safe core
and native platform boundary. Validation used bounded synthetic files, fault
injection, full-file review receipts, native Windows filesystem fixtures, and
source and compile review of the Unix and macOS paths.

Documentation, tests, generated build output, Git internals, local agent state,
and the application icon were excluded as non-runtime entry points. Test
assertions and architecture contracts were still used as supporting evidence.

## Reportable findings and remediation

### Windows staging files inherited broader directory permissions

Severity was medium. The reviewed Windows path created a same-directory staging
file with default security attributes. A native fixture placed an owner-only
destination in a directory with an inheritable Everyone read entry, observed
that entry on the live staging file, and read the exact synthetic staged bytes
through a second handle.

The remediation moves exclusive private creation into the platform adapter.
Windows passes a protected DACL at `CreateFileW` time, granting full control only
to SYSTEM and the object owner. A native test converts the live descriptor back
to SDDL and verifies the exact protected owner-and-system DACL, writable creation,
exact bytes, exclusive refusal of an existing path, and denial of competing write
handles. Unix continues to create siblings at mode 0600.

### macOS staging files could inherit broader directory permissions

Severity was medium. Mode 0600 restricts POSIX mode bits but does not prevent a
new macOS file from inheriting access-control entries from its parent directory.
The previous Unix creation path therefore could expose staged document bytes
under an inheritable parent ACL even though the mode bits appeared private.

The remediation uses `openx_np` to request mode 0600 and a zero-entry ACL carrying
the global `no_inherit` flag in the same kernel operation. Exact-commit native
run 30211571501 proved an ordinary child inherits the parent ACE while the
protected file immediately reports true ACL absence. It also proved that macOS
canonicalizes explicit zero-entry ACL text to absence instead of retaining an
allocated empty ACL. The adapter defensively applies the remove-ACL sentinel
through the live descriptor, verifies absence and mode 0600, and only then
returns the still-empty file for a write. Any finalization failure closes the
descriptor, preserves the random zero-byte pathname, and produces actionable
cleanup guidance instead of risking a pathname deletion race.

### Document loading had no byte ceiling

Severity was low because exploitation requires a user-assisted local open and
affects only the Noter process. The public load path materialized all bytes before
strict UTF-8 validation. A bounded proof loaded an 80 MiB valid UTF-8 file and
confirmed that the entire file was allocated.

The remediation defines a 64 MiB v0.1 document ceiling, leaving headroom above
the required 50 MiB performance corpus. Both content loading and save-target
fingerprinting reject an announced larger length before allocation or hashing.
They also read through a limit-plus-one wrapper, so concurrent growth cannot
bypass the ceiling. Errors are typed, stage-specific, and path-redacted.

## Reliability issues fixed during the review

- New, Open, Quit, and native close no longer discard a dirty document. They
  remain blocked with a visible explanation until M4 supplies the shared Save,
  Discard, and Cancel state machine.
- Windows backup cleanup now opens without following reparse points, verifies
  identity, fingerprint, and length on the live handle, and deletes that same
  object by handle while denying competing writers. Native fixtures prove a
  rebound path remains untouched and a same-object writer cannot enter the
  verification-to-deletion window.
- Unix existing-file replacement uses an atomic exchange while staging remains
  owner-only. macOS additionally suppresses ACL inheritance at the atomic create
  point and verifies true ACL absence before writing. Required metadata is
  captured into an immutable snapshot and
  revalidated through the open source handle before commit. Because exchange can
  legitimately change the displaced inode's `ctime`, the post-exchange check
  ratifies the new token with stable native identity, link count, content
  fingerprint, and length, but it never treats live post-commit metadata as the
  transfer source. The displaced file's ownership, mode, ACL, and visible
  extended attributes must also equal the pre-commit snapshot before that
  snapshot is applied to the committed open handle. A final-window metadata
  change leaves the committed file private and adds a warning instead of
  restoring stale metadata. Unix-only tests isolate the distinction; the hosted
  Linux and macOS matrix must exercise the complete protocol.
  Portable Unix deletion cannot bind unlink to the verified object, so the
  displaced original and failed-save siblings are retained with explicit
  warnings instead of risking a pathname cleanup race. Each warning names only
  the random sibling basename and gives inspection and removal guidance.
- Unix metadata capture now queries each xattr size before allocating its value,
  limits the snapshot to 4,096 entries and 64 MiB of aggregate names and values,
  and retries size races only three times. This closes a later checker finding
  where a small macOS data fork with a file-sized resource fork could bypass the
  document ceiling, exhaust memory or temporary storage, or stall Save. macOS
  now serializes the ACL into the immutable snapshot and replays it through the
  destination descriptor; resource forks and other xattrs use the bounded
  snapshot. Native ACL absence remains a distinct snapshot state and is replayed
  with macOS's remove-ACL sentinel. Present ACL entries remain serialized, while
  a zero-entry ACL input follows the kernel's canonical absent representation.
- A failed Unix post-commit file barrier now downgrades the result to Best Effort
  and remains distinct from parent-sync and cleanup warnings.
- Save warnings retain every cleanup and durability detail in the GUI. Save and
  Save As also expose the required hard-link confirmation instead of making the
  confirmation API unreachable. Save As carries the exact pre-dialog target
  expectation through confirmation, and a path rebind is covered by an
  automated conflict regression.
- Windows reserves its replacement backup while the staging handle still denies
  competing writers, then immediately revalidates the closed sibling's native
  identity, length, and BLAKE3-256 content before `ReplaceFileW`. Postcommit
  verification closes the remaining integrity-classification window. A fixture
  mutates the sibling after final validation but before the injected native
  replacement call and proves that postcommit verification classifies the
  result as indeterminate rather than silently accepting changed bytes. Processes
  running as the same user remain inside the filesystem authority boundary and
  can also write the final destination directly; their changes are treated as
  external versions, never as silently committed Noter bytes.
- Indeterminate commits and failed precommit or postcommit cleanup now carry a
  typed warning naming the safe random artifact basename and explicit inspect,
  recovery, retry, and removal actions. Displaced artifacts use neutral wording
  because concurrent bytes may not be the prior destination revision.
- A creation-time native identity or platform privacy-finalization failure now
  preserves the primary creation error and a separate cleanup warning when the
  just-created sibling cannot be removed by handle. The warning names only the
  random basename, states that Noter had not written application bytes,
  acknowledges same-authority changes, and gives inspection and removal
  guidance.
- File commands and native close are evaluated only after same-frame editor input
  updates authoritative document state.

## Residual evidence gaps

The review does not claim complete platform proof. M1 still requires:

- native macOS ACL, extended-attribute, resource-fork, flag, and durability
  fixtures;
- Windows post-replacement crash-persistence evidence;
- SMB, cloud-synchronized, removable, and weak-filesystem observations;
- a disposable second Windows identity test for staging-file denial; and
- ancestor-substitution fixtures beyond same-authority writers.

These remain explicit work in [ROADMAP.md](ROADMAP.md) and
[manual-test-matrix.md](manual-test-matrix.md). No release or M1 verification
claim depends on inference from a filesystem name or a local-only test.

## Remediation validation checkpoint

The remediated worktree passes all 172 Windows-local workspace tests, strict
workspace Clippy, rustdoc with warnings denied, documentation-link validation,
and RustSec audit. Fixed-seed measured line coverage is 92.26 percent for the
trust kernel and 87.54 percent for the complete workspace. The first expanded
mutation run exposed nine file-limit boundary survivors. Exact inclusive-limit,
oversized-announcement, constant-value, and overflow tests closed them; the last
completed Windows-applicable campaign classified all 383 mutants as 254 caught
and 129 unviable. The checker-expanded 418-mutant run found four survivors;
focused tests and isolated reruns produce a composite classification of 270
caught, 148 unviable, zero missed, and zero timed out. The expanded native
adapter scope adds a clean local 57-mutant Windows pass with 39 caught and 18
unviable. The descriptor-deallocation repair produced a clean 58-mutant Windows
pass with 40 caught and 18 unviable. The current adapter scope is 66. The settled
three-platform 741-mutant union assigns 617 candidates to Linux, 557 to Windows,
and 49 macOS-specific candidates to macOS with no set-union gap. Hosted run
30213398323 closed the macOS scope and exposed 32 Linux survivors plus two
shared scanner timeouts. The current local correction replaces the mutable
scanner arithmetic, gives repeated Unix decisions exact named predicates, and
tests ownership application through deterministic error injection. A fresh
exact-commit hosted campaign is required before this checkpoint becomes
complete CI evidence.
