# ADR-0005: Windows predecessor-preserving recovery replacement

Status: Accepted

## Context

An existing crash recovery record must be replaced without losing its older
snapshot if a rename, directory flush, or process fails. `ReplaceFileW` uses
pathnames for both the new stage and destination. Binding and inspecting those
files before the call does not make the final replacement object-bound. A native
NTFS probe found that a held destination denying delete sharing blocks a
competing rename but also blocks an attempted POSIX-style replacement. Allowing
delete sharing permits the replacement and the competing rename.

## Decision

For an existing record, create and sync the new stage privately relative to
the held records directory. Inspect its identity, length, and content, then
reopen it for deletion while denying delete sharing. Open and verify the
predecessor under the same conditions. Rename the held predecessor exclusively
to its reserved backup name and sync the held records directory. Rename the
held stage exclusively to the now-vacant canonical name and sync again.
Recheck the exact new and old artifacts. Delete the opened predecessor only
after both checks and sync the directory once more.

A failed operation returns an error and leaves whichever stage, backup, and
canonical record artifacts exist. Startup scanning validates and coalesces
them by recovery lineage. A competing canonical entry blocks the exclusive
stage rename and remains untouched. A later persist does not reuse an occupied
stage or backup slot. The protocol does not claim an atomic instant at which
the canonical name always exists. It preserves recoverable bytes across each
reported failure and syncs the predecessor backup before vacating its name.

## Consequences

Replacement uses up to three directory barriers and may report an error after
the new record is installed. Callers retain dirty state on every error. The
reserved backup and stage slots bound retained artifacts for an instance and
can require startup review before another persist. The native fixture covers
exclusive rename and competition; injected barrier tests cover recovery-store
failure states. The broader native fault and race matrix, redirected-root
classification, and exact-head CI remain M4-H1 exit work.
