# Changelog

All notable project changes are recorded here. Noter has not published a stable
release, so current work remains under Unreleased.

## Unreleased

### Added

- Add persisted System, Light, and Dark themes with a unified aligned editor
  toolbar and deliberate document typography.
- Add an early native Markdown Mode with source-backed block editing, H1, H2,
  Bold, Italic, Link, Code, List, and Quote actions, plus four conservative
  source diagnostics.
- Add working About Noter and truthful update-status dialogs, including the
  `noter update` entry point.
- Add locked source-install helpers for PowerShell and POSIX systems with check
  and custom-root modes.
- Add deterministic native Light and Dark README screenshot generation and
  validation using a polished non-sensitive Markdown demo document.

### Security

- Bound document loading and save-target hashing to the explicit 64 MiB v0.1
  limit, including protection against concurrent file growth.
- Bound Unix extended-attribute snapshots to 4,096 entries and 64 MiB of
  aggregate names and values before value allocation, including macOS resource
  forks.
- Create Windows staging and new files with a protected owner-and-system DACL so
  permissive parent ACLs cannot expose staged document bytes.
- Create macOS staging and new files with mode 0600 and a zero-entry,
  no-inherit bootstrap ACL in the same `openx_np` operation. Remove and verify
  that ACL through the live descriptor before writing any document bytes, and
  report the random zero-byte sibling if security finalization fails.
- Add a pinned RustSec audit gate to CI.
- Add an explicit cargo-deny policy and pinned CI gate for dependency licenses,
  registry and Git sources, wildcard versions, advisories, and duplicate-version
  visibility.
- Pin the coverage tool used by CI.

### Fixed

- Preserve dirty work by blocking New, Open, Quit, and native close until the
  complete lifecycle decision flow is implemented.
- Preserve replacement artifacts whose identity or bytes changed during cleanup.
- Delete Windows cleanup candidates through the exact verified open handle so a
  rebound pathname cannot redirect deletion.
- Keep Unix staging owner-only from creation through atomic exchange, finalize
  metadata after commit, and retain artifacts when safe handle-bound cleanup is
  unavailable.
- Preserve Unix destination metadata from an immutable pre-commit snapshot,
  verify the displaced original after atomic exchange, require its stable
  metadata payload to still match the snapshot, and never apply unratified or
  stale metadata to the committed file.
- Serialize the macOS ACL into the immutable metadata snapshot and replay it
  through the destination descriptor, eliminating temporary ACL paths while
  keeping resource forks and other xattrs inside the bounded snapshot. Treat
  macOS `ENOENT` from `acl_get_fd` as a distinct absent state, replay it with the
  native remove-ACL sentinel, and verify true absence instead of collapsing it
  into an allocated empty ACL.
- Surface exact save cleanup and durability warnings instead of a generic
  success warning.
- Process same-frame editor input before file commands and native close checks.
- Carry the exact pre-dialog Save As target expectation through hard-link
  confirmation so a rebound destination conflicts instead of being overwritten.
- Preserve creation-time identity failures and retained-sibling cleanup guidance
  as distinct typed errors.
- Detect a same-authority Windows staging mutation before replacement or during
  the final handoff, and classify postcommit mismatch as indeterminate.
- Make About Noter open a truthful project dialog and state the exact limits of
  the current Markdown implementation.

### Engineering

- Upgrade eframe and egui to 0.35 and egui_commonmark to 0.24, retaining
  restricted features while enabling current shaping, hinting, theme-aware font
  transfer, and subpixel placement behavior.
- Define and enforce repository-wide code-quality and evidence standards.
- Enforce Ruff linting and formatting for repository validation scripts in CI,
  and normalize text files to LF across supported development platforms.
- Keep local agent state and runtime logs in ignored dedicated directories.
- Remove obsolete tracked agent metadata and commented-out build or CI plans.
- Expand mutation enforcement through the native platform adapter with a macOS
  job and a current 747-decision supported-platform union with no gap. The last
  completed 58-mutant Windows adapter pass catches all 40 viable mutations,
  including descriptor deallocation, with 18 genuine compiler rejections and no
  miss, timeout, or infrastructure failure; the hardened creation path expands
  the current adapter enumeration to 66 and requires a fresh campaign.
- Reject linker, compiler, storage, process, and tool-lock infrastructure
  failures that cargo-mutants would otherwise classify as unviable.
- Close a focused 58-candidate Markdown diagnostics campaign with a composite
  result of 55 caught and three genuine compiler rejections after isolating and
  rerunning one Windows linker-lock failure.
- Raise measured fixed-seed line coverage to 93.53 percent for the trust kernel
  and 87.94 percent for the complete workspace across 170 Windows-local tests,
  and enforce respective 90 and 80 percent floors in CI.
