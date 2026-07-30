# Privacy Contract

**Effective for release candidates:** July 2026

**Status:** Product contract. A release is not privacy-verified until dependency
review and runtime network inspection pass on every supported platform.

Noter is designed for local text and Markdown. Document data has no reason to
leave the machine during editing.

## 1. No background network activity

The official Noter application has no telemetry, analytics, advertising,
account, synchronization, remote font, remote image, link preview, automatic
crash upload, or background update check.

Noter does not fetch Markdown assets or execute embedded content. An unexpected
background connection is a critical security defect.

## 2. Explicit external actions

A link opens outside Noter only after the user selects it. The current
`Help > Check for Updates` action and `noter update` command open a local status
dialog; they perform no network request. The dialog can open Noter's GitHub
releases page only after the user selects that link.

A future updater may contact only the documented release channel after an
explicit update action. Its request must contain only the information required
to select a compatible release, such as current version, operating system, and
architecture. It must not contain document content, document paths, search
text, a stable installation identifier, or an analytics payload.

## 3. Files Noter may read

Noter reads document content only when:

- the user selects or explicitly launches that path;
- the user selects a recent-file entry created by an earlier explicit open; or
- Noter validates a versioned recovery record that Noter created.

Noter does not crawl folders, index unrelated files, inspect neighboring
documents, or follow Markdown references to collect content.

## 4. Local state

Preferences, window state, and recent-file paths belong in the platform's
per-user application directory. Recovery records belong in the private
per-user state directory, not a shared temporary directory.

Recovery and durable-save staging can contain complete document bytes. They
therefore require restrictive creation permissions, bounded size and retention,
checksums or identity validation where applicable, and explicit cleanup.
Recovery never silently overwrites the original document.

Unix staging files are created with mode 0600. macOS additionally suppresses ACL
inheritance during creation and verifies the finalized ACL before writing.
Windows creates a protected DACL granting full control only to the owner and
SYSTEM.

When Unix replacement must retain a displaced original, Noter first validates
its identity, content, and metadata through the open object. An exact match is
restricted through that handle to mode 0600 before retention. macOS also removes
the ACL and verifies its absence. If validation or restriction fails, Noter
reports the retained artifact and does not claim that owner-only access was
established.

## 5. Diagnostics

Default diagnostics do not contain:

- document, clipboard, recovery, search, or replacement content;
- full document paths or recent-file lists; or
- user, machine, account, or installation identifiers.

Any future diagnostic export that needs sensitive context must be opt-in,
previewable, local by default, and separately specified before implementation.

## 6. No AI processing

Noter has no AI feature and does not transmit content for training, inference,
classification, moderation, or profiling.

## 7. Verification and reporting

Release evidence includes dependency-capability review, locked advisory checks,
build provenance, and runtime traffic inspection on Windows, macOS, X11, and
Wayland. Report unexpected file access, outgoing traffic, or sensitive log data
as a critical security issue.
