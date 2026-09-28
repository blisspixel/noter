# ADR-0006: Windows local recovery root and legacy review

Status: Accepted

## Context

eframe places `app.ron` under the per-user RoamingAppData known folder. The
original default recovery session used that same state directory. Recovery
records can contain complete unsaved private documents, so they should not be
placed in a folder intended to roam with a user profile. Existing records in
that location cannot be ignored or removed merely because the default changes.

## Decision

New Windows recovery sessions resolve the current user's
[LocalAppData known folder](https://learn.microsoft.com/windows/win32/shell/knownfolderid)
through
[`SHGetKnownFolderPath`](https://learn.microsoft.com/windows/win32/api/shlobj_core/nf-shlobj_core-shgetknownfolderpath)
and write under `Noter/recovery`.
The existing fixed-NTFS, non-reparse, owner-controlled directory and Cloud
Files checks still apply before writing content. Preferences remain in
RoamingAppData.

If the former RoamingAppData recovery directory exists, open it through the
same verified namespace and scan it alongside the local root. Every startup
offer remembers its source root. Restore claims and reads the old record there,
persists its successor in the local root, then removes the claimed old record.
Discard removes only the explicitly offered record in its source root. A
missing legacy recovery directory is left uncreated. A failed lookup or
verification of a possible legacy root disables recovery for the session and
surfaces the failure instead of silently hiding an existing record.

## Consequences

Unresolved legacy records remain in their original location until the user
restores or discards them. A cleanup failure can leave both a local successor
and the old legacy offer. If startup finds a schema-v2 predecessor link with
exactly the next generation, restoring the local successor keeps the older
Roaming copy available for an explicit cleanup action in the current editor
session. Incomparable records, including legacy schema-v1 records, remain
separate offers. After the local document is saved without a durability
warning, a linked but incomparable Roaming offer is presented through the
ordinary Restore / Later / Discard review. A Save with a durability warning
keeps the current document and its recovery copy at risk and delays that review.
This waits until current
work is safe before another Restore can replace it. A failed exact cleanup
retains its offer for retry. The old copy is never deleted from metadata alone.
Each root has its own bounded startup scan, so a dual-root launch can review
up to twice one root's limits while still leaving overflow untouched.
LocalAppData is the Windows nonroaming
known folder, but arbitrary third-party synchronization of that folder cannot
be proven absent;
the verified-root policy still fails closed for supported detectable cases.
