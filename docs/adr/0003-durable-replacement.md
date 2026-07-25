# ADR-0003: Durable platform replacement protocol

**Status:** Proposed

**Date:** 2026-07-25

## Context

The prototype creates a predictable sibling, syncs it, and calls
`std::fs::rename`. That does not define collision handling, file identity races,
metadata, symlinks, parent-directory durability, or the distinction between a
pre-commit error and a post-commit durability warning.

Save is Noter's highest-risk operation. The protocol must be injectable and
tested at every boundary before adoption.

## Proposed decision

Implement saving behind the `Storage` boundary in
[DESIGN.md](../DESIGN.md#6-durable-file-io):

1. Capture a revision-tagged immutable save snapshot.
2. Revalidate destination identity and fingerprint.
3. Create an unpredictable same-directory sibling with create-new semantics.
4. Stream bytes, flush, and sync the sibling.
5. Apply the ratified destination metadata policy.
6. Revalidate identity immediately before commit.
7. Replace atomically without first deleting the destination.
8. Sync the parent directory where meaningful.
9. Return Committed, Conflict, or Not Committed with exact commit state.

Evaluate Windows `ReplaceFileW` for existing destinations and atomic move for
new destinations. Use same-filesystem rename plus parent sync on Unix. Prefer an
audited crate if it proves the same contract and reduces platform-specific code.

## Questions that block acceptance

1. Which Windows metadata and alternate-stream properties survive each API?
2. Which Unix permission, ownership, ACL, and extended-attribute fields must be
   copied to the sibling?
3. Does Save follow a symlink to its recorded regular-file target, and how is
   target identity revalidated?
4. How does Save As treat an existing symlink?
5. Which platforms support meaningful parent-directory sync?
6. How are cloud, network, removable, and unusual filesystems detected or
   documented?
7. What outcome is shown if replacement commits but parent sync fails?

## Acceptance evidence

- Platform contract table for Windows, macOS, Linux, and weaker filesystems.
- Injected failures for every operation before and after commit.
- Tests proving original completeness for every pre-commit failure.
- Tests proving exact destination bytes for success.
- File-identity race tests with an external writer.
- Metadata and symlink fixtures on all supported operating systems.
- Mutation testing of commit-state and dirty-revision decisions.

Until this evidence exists, `Document::save_atomic` remains explicitly interim
and M1 is not complete.
