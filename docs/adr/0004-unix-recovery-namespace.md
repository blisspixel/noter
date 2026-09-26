# ADR-0004: Unix recovery namespace binding and retirement

**Status:** Accepted

**Implementation verification:** In progress

**Date:** 2026-09-26

## Context

Crash recovery writes complete document bytes beneath the per-user state
directory. Before this decision, Unix opened that tree by pathname: it created
the recovery directories with `create_dir_all`, checked no owner, mode, or file
system, and removed records, live leases, and quarantined sources with pathname
`unlink`. A directory another user could change, a link planted in the chain,
or a network file system could redirect or expose recovery content, and the
M4-H1 milestone requires every supported platform to reject such a root before
writing recovery bytes or to prove the operations safe against it.

Unix has no call that unlinks the object behind an open descriptor. `unlinkat`
removes a name in a directory, so a check that the name identifies the opened
object and the removal are always two steps. The retirement question is which
guarantee closes that window.

## Decision

Bind the recovery namespace once per session, then keep every operation
relative to held, verified directories:

1. Resolve links in the part of the state path that already exists once, so a
   system link such as macOS `/var` works, then reopen every component from `/`
   with `O_NOFOLLOW | O_DIRECTORY` and create missing components with
   `mkdirat` mode 0700 through the held parent.
2. Accept an ancestor only when it is owned by the superuser or the current
   user and is writable by group or others only with the sticky bit, which stops
   them renaming or removing entries they do not own. Require the state directory
   itself to be owned by the current user and not writable by others.
3. Create or open the recovery, records, and quarantine directories through the
   held parent without following links. Require the current user as owner and
   the state directory's device; tighten a looser mode to 0700 and, on macOS,
   strip any extended ACL and verify that none remains.
4. Refuse a state directory on a network, cluster, or user-space file system:
   known NFS, SMB, CIFS, Coda, AFS, NCP, Ceph, 9P, FUSE, GFS2, OCFS2, Lustre,
   and OrangeFS magic numbers on Linux, and any mount without `MNT_LOCAL` on
   macOS. Other Unix systems cannot be verified and are refused.
5. Hold the directory descriptors for the session. Create, open, list,
   classify, commit, remove, and sync every record, lease, and quarantine entry
   relative to them. Refuse new content in a bound directory whose link count
   has fallen to zero, so a removed recovery tree fails persistence visibly
   instead of writing unreachable records.
6. Retire an entry with `unlinkat` in its held directory, immediately after
   `fstatat` without following links confirms that the name still identifies
   the device and inode of the object Noter holds open.

The residual window in step 6 is closed by the namespace, not by the call. After
binding, the directory holding the entry is owned by the current user, mode
0700, free of ACLs on macOS, and reached only through descriptors whose
ancestors no other user can change. Only processes running as the same user can
alter its entries between the check and the removal, and those processes
already hold full authority over that user's documents and recovery data. They
are outside the threat model, as they are for every other per-user store.

## Alternatives considered

- **Retain and neutralize the opened object** instead of unlinking it: truncate
  and keep the entry for later review. This leaves an unbounded trail of empty
  artifacts for every Save and Discard and still needs a name-based removal
  eventually, so it moves the window instead of closing it.
- **Rename to a unique name, verify, then unlink.** The final unlink is still
  name-based, so this adds a step without removing the window.
- **Keep pathname operations and document the risk.** This leaves records
  redirectable through any ancestor another user controls, which M4-H1 exists
  to prevent.

## Consequences

- A state root that another user can change, that contains a planted link below
  its existing prefix, or that lives on a network file system makes recovery
  unavailable for the session. The message names the reason, and ordinary saves
  are unaffected.
- Renaming or replacing any directory above a bound recovery directory after
  startup cannot redirect recovery reads, writes, or removals.
- Ancestor checks cover owner and mode bits, not ACLs on directories above the
  state directory. Default macOS home folders carry a deny-only ACL, and
  rejecting all ancestor ACLs would disable recovery there; an ancestor ACL that
  grants another user write access is not detected.
- The Windows namespace keeps pathname operations inside its held,
  delete-protected directories and now refuses any path outside them. Moving
  those operations onto handle-relative Windows calls remains M4-H1 work.

## Evidence

- `crates/noter-platform/src/unix_recovery_namespace.rs` tests: creation of a
  private tree, tightening of loose modes, rejection of a state directory or
  ancestor others can write and acceptance of a sticky one, refusal of links and
  non-directories in the recovery tree, one-time resolution of a link in the
  existing prefix, operations that follow the bound directory after an ancestor
  rename, refusal to remove a replaced entry, refusal of new content in a
  removed directory, and file-system classification.
- `src/core/recovery_store.rs` and `src/crash_recovery.rs` tests: the store's
  unchanged behavior through the bound namespace, a replaced records path that
  cannot redirect recovery work, and an unsafe state root refused before any
  recovery directory is created, with its reason shown.
- Native macOS evidence runs in CI; ACL removal on directories has no local
  fixture on Linux.
