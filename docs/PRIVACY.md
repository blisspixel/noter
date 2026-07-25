# Noter Data Bill of Rights

**Effective for release candidates:** July 2026

**Status:** Release contract. A prerelease is not privacy-verified until its
dependency audit and runtime network inspection pass.

Noter is designed for local plain text. The application has no business model,
feature, or diagnostic need that requires document data to leave the machine.

## 1. Zero application network activity

The official Noter application:

- has no telemetry, analytics, advertising, account, synchronization, update
  check, remote font, remote image, link preview, or automatic crash report;
- does not fetch links or Markdown assets;
- does not contain a user-facing action that transmits document data;
- treats an unexpected outgoing connection as a critical security defect.

Release evidence includes dependency capability review and runtime traffic
inspection on each supported operating system.

## 2. Files Noter may read

Noter reads document content only when:

- the user selects or explicitly launches that path;
- the user selects a recent-file entry created from an earlier explicit open;
- Noter validates a versioned recovery record that Noter itself created.

Noter does not crawl folders, index unrelated files, inspect neighboring
documents, or follow links for preview content.

## 3. Local state

Preferences, window state, and recent-file paths live in the platform's
per-user application configuration or local-data directory. Recovery records
live in the private per-user application state or local-data directory, not the
general temporary directory.

Recovery may contain unsaved document content. It therefore receives restrictive
permissions, checksums, versioning, bounded retention, and explicit cleanup
after successful Save or confirmed Discard. Recovery never silently writes the
original document.

## 4. Diagnostics

Default diagnostics do not contain:

- document or clipboard content;
- recovery bytes;
- search or replacement text;
- full document paths;
- recent-file lists;
- user or machine identifiers.

If a future diagnostic export needs sensitive context, it must be opt-in,
previewable, local by default, and separately specified before implementation.

## 5. No AI use

Noter has no AI feature and does not transmit content for training, inference,
classification, moderation, or profiling.

## 6. Verification and reporting

The source, dependency graph, build provenance, and release checks are intended
to make these claims auditable. Report any unexpected file access, outgoing
connection, or sensitive log entry as a critical security issue.
