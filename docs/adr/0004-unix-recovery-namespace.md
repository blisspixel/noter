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
   user and no other user can replace its entries: it is writable by others
   only with the sticky bit, which stops them renaming or removing entries they
   do not own. A group number matching the user's number does not establish
   that no other account belongs to it;
   [Linux ACL masks](https://man7.org/linux/man-pages/man5/acl.5.html) can make
   the group mode bits represent a named user's write grant. Refuse non-sticky
   group-writable ancestors even when their group number matches the user.
   On macOS, inspect each held ancestor's ACL within the
   [platform's 128-entry bound](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/kauth.h)
   and refuse any allow entry while accepting deny-only ACLs, including
   those on default home
   folders. Require the state directory itself to be owned by the current
   user; it is Noter's own directory, so a mode that lets others write it is
   tightened to 0700. On macOS, remove its extended ACL and verify absence
   before binding child directories.
3. Create or open the recovery, records, and quarantine directories through the
   held parent without following links. Require the current user as owner and
   the state directory's device; tighten a looser mode to 0700 and, on macOS,
   strip any extended ACL and verify that none remains.
4. Refuse a state directory on a known network, cluster, shared-folder, or
   user-space file system: the NFS, SMB, CIFS, Coda, AFS, NCP, Ceph, 9P, FUSE,
   GFS2, OCFS2, Lustre, OrangeFS, BeeGFS, GPFS, PanFS, IBRIX, ACFS, SNFS, and
   VirtualBox, VMware, and Parallels shared-folder magic numbers on Linux, and
   any mount without `MNT_LOCAL` on macOS. Linux cannot mark a file system as
   local, so an unlisted network file system is not detected. Other Unix
   systems cannot be verified and are refused.
5. Hold the directory descriptors for the session. Create, open, list,
   classify, commit, remove, and sync every record, lease, and quarantine entry
   relative to them. On macOS too, private creation uses `openat` in the held
   directory; the path-based ACL-aware primitive is unnecessary because the
   directory has no ACL for a new file to inherit. Linux and macOS refuse to
   create entries in a removed directory, so a recovery tree removed while
   Noter runs fails persistence visibly instead of writing unreachable
   records. A link count is not a portable signal: APFS keeps a removed
   directory's count above zero while a descriptor holds it.
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

- Recovery is unavailable for a state root with an ancestor another user can
  change, including through non-sticky group writes, a planted link below its
  existing prefix, or a known network file system. The message names the
  directory and the reason, and ordinary saves are unaffected.
- Renaming or replacing any directory above a bound recovery directory after
  startup cannot redirect recovery reads, writes, or removals.
- An ancestor ACL with an allow entry is refused on macOS, even when it grants
  only the current user access. Deny-only ACLs remain accepted so default home
  folders can retain recovery.
- The Windows namespace keeps pathname operations inside its held,
  delete-protected directories and now refuses any path outside them. Moving
  those operations onto handle-relative Windows calls remains M4-H1 work.

## Evidence

- `crates/noter-platform/src/unix_recovery_namespace.rs` tests: creation of a
  private tree, tightening of loose modes and of a state directory others can
  write, macOS removal of an inherited state-directory ACL, rejection of an
  ancestor others can write with the directory named,
  acceptance of a sticky one and refusal of non-sticky group writes, a native
  Linux named-user ACL fixture whose write mask appears as group mode bits,
  native macOS acceptance of a deny-only ancestor ACL and refusal of an allow ACL
  after two deny entries before state creation, refusal of links and
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
