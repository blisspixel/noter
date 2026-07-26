# ADR-0003: Durable platform replacement protocol

**Status:** Accepted

**Implementation verification:** In progress

**Date:** 2026-07-25

## Context

The prototype creates a predictable sibling, syncs it, and calls
`std::fs::rename`. That does not define collision handling, file identity races,
metadata, symlinks, hard links, parent-directory durability, or the distinction
between a pre-commit error and a post-commit durability warning.

Save is Noter's highest-risk operation. The protocol must be injectable, tested
at every boundary, and honest when an operating system cannot prove whether a
failed replacement committed.

## Evidence

- Rust documents that [`std::fs::rename`](https://doc.rust-lang.org/1.97.1/std/fs/fn.rename.html)
  maps to different Unix and Windows operations and does not work across mount
  points. It is a primitive, not a complete save contract.
- Microsoft documents that
  [`ReplaceFileW`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew)
  preserves creation time, short name, object identifier, DACLs, security
  resource attributes, encryption, compression, and named streams not already
  present in the replacement. It also documents failure codes where path
  entries or inherited metadata may already have changed. A false return value
  is therefore not always proof of non-commit.
- Microsoft documents that
  [`MoveFileExW`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)
  can request `MOVEFILE_WRITE_THROUGH`, can refuse an existing destination when
  replacement is not requested, and becomes copy-and-delete across volumes only
  when `MOVEFILE_COPY_ALLOWED` is set.
- Linux documents that [`rename`](https://man7.org/linux/man-pages/man2/rename.2.html)
  atomically replaces an existing destination, leaves an instance of the
  destination in place on failure, replaces a final symlink entry rather than
  its target, and does not change other hard links to the old inode.
- Linux documents that [`fsync`](https://man7.org/linux/man-pages/man2/fsync.2.html)
  synchronizes file data and metadata but requires a separate directory `fsync`
  for the containing directory entry.
- Apple's `fsync` contract and guidance describe `F_FULLFSYNC` as the stronger
  request for storage persistence. Apple's `copyfile` API can copy POSIX
  metadata, ACLs, and extended attributes separately from file data. See the
  [Xcode fsync manual](https://keith.github.io/xcode-man-pages/fsync.2.html),
  [Apple persistence guidance](https://developer.apple.com/documentation/xcode/reducing-disk-writes),
  and [Xcode copyfile manual](https://keith.github.io/xcode-man-pages/copyfile.3.html).
- The current [`atomic-write-file`](https://docs.rs/atomic-write-file/0.3.0/atomic_write_file/)
  crate explicitly replaces a final symlink and does not preserve timestamps,
  ACLs, extended attributes, or SELinux contexts. It does not satisfy this
  contract as Noter's production adapter.

## Decision

### Outcome model

The commit-point adapter returns exactly one of:

1. `Committed`, with a verified observation and achieved durability level.
2. `Conflict`, when the destination no longer matches the snapshot expectation.
3. `NotCommitted`, only when the adapter proves the destination did not commit.
4. `CommitStateUnknown`, when the platform may have changed a path entry despite
   reporting failure.

`CommitStateUnknown` keeps the document dirty, retains recovery, disables blind
retry, and requires path reconciliation before another ordinary Save. A generic
I/O error is never interpreted as non-commit.

### Protocol

1. Capture an immutable revision-tagged snapshot containing exact serialized
   bytes, target, expected identity, fingerprint, BOM, and EOL profile.
2. Inspect the final path without following an unapproved final link. Refuse
   special files and detect an external version before creating a sibling.
3. Create an unpredictable same-directory sibling with create-new semantics.
4. Write all bytes and flush user-space buffers.
5. Apply the ratified metadata policy. Failure is pre-commit and leaves the
   destination unchanged.
6. Sync the sibling's data and metadata.
7. Revalidate destination identity and fingerprint immediately before commit.
8. Use the platform commit primitive for existing or absent destinations.
9. Reconcile documented ambiguous platform results before assigning commit
   state.
10. Sync the containing directory or request the strongest supported equivalent.
11. Remove temporary or backup artifacts explicitly and report cleanup failure.

If replacement commits but a later persistence barrier fails, the result is
`Committed` with a weaker durability level and warning. The UI must not say that
nothing was written. Only an outcome for the current revision may clear dirty
state. Recovery remains until the committed revision reaches the required
durability policy.

### Platform contract

| Platform and case | Commit primitive | Metadata | Durability result |
| --- | --- | --- | --- |
| Windows, existing file | `ReplaceFileW` with a unique backup sibling and no ignore-merge flags; reconcile destination, replacement, and backup on documented partial failures | Native merge preserves the documented DACL, encryption, compression, creation, identifier, and named-stream properties; any merge failure is not ignored | Flush sibling before commit; because `REPLACEFILE_WRITE_THROUGH` is unsupported and no supported parent-directory barrier is documented, report at most `FileSynced` unless platform tests prove more |
| Windows, absent file | `MoveFileExW` with `MOVEFILE_WRITE_THROUGH`, without replace or cross-volume copy flags | New-file policy and inherited parent ACL | Refuse a newly appeared destination; report the barrier strength demonstrated by platform tests |
| Linux, existing file | Same-directory `renameat`; destination identity revalidated immediately before commit | Preserve mode, ACLs, extended attributes, security context, and attainable ownership; abort before commit if required metadata cannot be copied | `fsync` sibling, rename, then `fsync` opened parent directory |
| Linux, absent file | No-replace rename where supported, otherwise a no-overwrite link-and-unlink sequence | Owner-only mode 0600 | Same file and parent barriers as existing replacement |
| macOS, existing file | Same-directory `renameat` through an opened parent | Copy POSIX metadata, ACLs, and extended attributes with the platform metadata APIs; saving intentionally advances modification time | Request `F_FULLFSYNC` for the sibling, rename, then synchronize the parent where supported |
| macOS, absent file | No-replace rename where supported, otherwise a no-overwrite link-and-unlink sequence | Owner-only mode 0600 | Same barriers as existing replacement |
| Network, cloud, removable, or unknown filesystem | Use the platform path only when same-filesystem commit prerequisites hold | Never silently discard known metadata | Return `BestEffort` or `FileSynced` according to demonstrated capability; never advertise full durability from filesystem name alone |

No platform may fall back to deleting the destination first. No cross-volume
copy-and-delete operation is described as atomic replacement.

### Links, read-only files, and special paths

- The conservative v0.1 policy refuses a final symlink or Windows reparse point
  for both Open and Save As. The user may select the resolved target explicitly.
  Following a link remains deferred until its resolved target and link entry can
  both be represented and revalidated across all supported platforms.
- Unix commit operations use an opened containing directory and relative names,
  reducing rename and remount races at the commit boundary.
- A destination with multiple hard links requires explicit confirmation that
  atomic replacement updates only the selected directory entry. Other hard
  links continue to reference the old file.
- Read-only files are not made writable silently. Save remains not committed and
  offers Save As or a separate explicit permission-changing action.
- Directories, devices, pipes, sockets, and unsupported reparse points are never
  ordinary Save targets.

### Residual race

Portable path replacement cannot provide a compare-and-swap against a hostile
external writer on every supported filesystem. Noter narrows the window with
identity plus content fingerprint checks, opened-parent operations, exclusive
creation for absent targets, and immediate pre-commit revalidation. Recovery is
retained through uncertain outcomes. The product does not claim adversarial
transaction isolation that the operating systems do not provide.

### Implementation boundary

The pure `Storage` protocol owns ordering and outcome decisions. Platform code
implements only inspected facts, unique creation, metadata transfer, commit,
reconciliation, synchronization, and cleanup. Safe wrapper crates are preferred.
Any required unsafe operating-system calls must be isolated in a small adapter
crate with a written safety contract and dedicated platform tests; the primary
library remains `unsafe_code = "forbid"`.

## Consequences

- Windows needs backup-aware reconciliation because documented replacement
  failures can have side effects.
- Unix needs explicit metadata transfer and directory synchronization.
- Saving a symlink, hard-linked file, read-only file, or weak filesystem becomes
  visible policy rather than an accidental consequence of `rename`.
- Some successful saves legitimately carry a durability warning.
- The `atomic-write-file` 0.3 crate and plain `std::fs::rename` are rejected as
  complete production adapters, though their implementation techniques remain
  useful evidence.

## Verification required before M1 completes

- Injected failures for every operation before and after the commit point.
- Tests proving original completeness for every proven pre-commit failure.
- Tests proving exact destination bytes for success.
- Windows tests for `ReplaceFileW` success and documented partial-failure states
  on NTFS, including DACLs, alternate streams, compression, and read-only files.
- Linux and macOS tests for mode, ownership limits, ACLs, extended attributes,
  final symlinks, hard links, and parent synchronization.
- New-destination race tests and existing-destination external-writer tests.
- Local, SMB, cloud-synced, removable, and at least one weaker filesystem record.
- Mutation testing of commit-state, conflict, cleanup, and dirty-revision
  decisions.

The pure fault-injected protocol is verified across the CI matrix at commit
`0edc342`, and the private-sibling slice is verified at commit `d44b1ec`.
BLAKE3-256 fingerprints, stable-handle loading, metadata change tokens, and the
complete `FilesystemStorage` adapter are verified at commit `c76515c` in
[GitHub Actions run 30181088267](https://github.com/blisspixel/noter/actions/runs/30181088267).
The adapter copies Linux and macOS metadata, uses native Windows and Unix commit
primitives, reconciles documented partial states, verifies exact committed
identity and bytes, reports cleanup and durability independently, and is
integrated with the sealed revision-aware Document API. The complete local
mutation campaign has zero missed mutants and zero timeouts, as recorded in
[M1 Mutation Evidence](../M1_MUTATION_EVIDENCE.md), and a pinned full-scope CI
gate now enforces it. The manual platform and weak-filesystem matrix plus the
reproducible benchmark baseline remain required before M1 is Verified.
