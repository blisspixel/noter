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
  validation using a polished non-sensitive Markdown demo document. The capture
  keeps one source-backed block active so the formatting controls and underlying
  Markdown are visible together.

### Security

- Bound document loading and save-target hashing to the explicit 64 MiB v0.1
  limit, including protection against concurrent file growth.
- Bound Unix extended-attribute snapshots to 4,096 entries and 64 MiB of
  aggregate names and values before value allocation, including macOS resource
  forks.
- Create Windows staging and new files with a protected owner-and-system DACL so
  permissive parent ACLs cannot expose staged document bytes.
- Create macOS staging and new files by requesting mode 0600 and a zero-entry,
  no-inherit ACL in the same `openx_np` operation. Native evidence proves the
  resulting file has true ACL absence while an ordinary control file inherits
  the parent ACE. Defensively remove and verify ACL absence through the live
  descriptor before writing any document bytes, and report the random zero-byte
  sibling if security finalization fails.
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
  native remove-ACL sentinel, and verify true absence. Record the native kernel
  behavior that replaying explicit zero-entry ACL text canonicalizes to absence.
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
- Lock the renderer's distinct light-background and dark-background
  coverage-transfer behavior with a regression test.
- Define and enforce repository-wide code-quality and evidence standards.
- Enforce Ruff linting and formatting for repository validation scripts in CI,
  and normalize text files to LF across supported development platforms.
- Keep local automation state and runtime logs in ignored dedicated directories.
- Remove obsolete tracked automation metadata and commented-out build or CI
  plans.
- Expand mutation enforcement through the native platform adapter with a macOS
  job and a current 741-candidate supported-platform union with no gap. Hosted
  run 30213398323 completed the 49-candidate macOS scope without a survivor and
  exposed only Linux decision-coverage gaps plus two shared line-scanner
  timeouts. The settled local correction removes mutable progress arithmetic,
  gives repeated native decisions exact named predicates, and retains all
  supported-platform candidates across Linux, Windows, and macOS. Exact-commit
  run 30221793209 passes the complete matrix: Linux 617 total with 438 caught,
  Windows 557 total with 381 caught, and macOS 49 total with 43 caught. Every
  remaining candidate is a validated compiler rejection; no scope has a miss or
  timeout.
- Give Windows mutation tests a 60-second minimum test-process timeout after a
  prior hosted run timed out one mutant even though its truth-table test had
  already failed. A focused rerun catches all four mutations of that predicate;
  the 90-minute outer job limit remains unchanged.
- Reject linker, compiler, storage, process, and tool-lock infrastructure
  failures that cargo-mutants would otherwise classify as unviable. Normalize
  ANSI-decorated logs and reject clang linker signal crashes after post-run
  review found one hidden in the otherwise green run 30219731527; corrected run
  30221793209 catches that mutant and passes the strengthened validator.
- Close a focused 58-candidate Markdown diagnostics campaign with a composite
  result of 55 caught and three genuine compiler rejections after isolating and
  rerunning one Windows linker-lock failure.
- Maintain measured fixed-seed line coverage at 92.26 percent for the trust
  kernel and 87.54 percent for the complete workspace. The Windows-local suite
  contains 172 tests, and CI enforces respective 90 and 80 percent floors.
