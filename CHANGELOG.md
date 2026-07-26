# Changelog

All notable project changes are recorded here. Noter has not published a stable
release, so current work remains under Unreleased.

## Unreleased

### Security

- Bound document loading and save-target hashing to the explicit 64 MiB v0.1
  limit, including protection against concurrent file growth.
- Create Windows staging and new files with a protected owner-and-system DACL so
  permissive parent ACLs cannot expose staged document bytes.
- Add a pinned RustSec audit gate to CI.
- Pin the coverage tool used by CI.

### Fixed

- Preserve dirty work by blocking New, Open, Quit, and native close until the
  complete lifecycle decision flow is implemented.
- Preserve replacement artifacts whose identity or bytes changed during cleanup.
- Delete Windows cleanup candidates through the exact verified open handle so a
  rebound pathname cannot redirect deletion.
- Keep Unix staging owner-only through atomic exchange, finalize metadata after
  commit, and retain artifacts when safe handle-bound cleanup is unavailable.
- Surface exact save cleanup and durability warnings instead of a generic
  success warning.
- Process same-frame editor input before file commands and native close checks.
- Carry the exact pre-dialog Save As target expectation through hard-link
  confirmation so a rebound destination conflicts instead of being overwritten.
- Preserve creation-time identity failures and retained-sibling cleanup guidance
  as distinct typed errors.
- Detect a same-authority Windows staging mutation before replacement or during
  the final handoff, and classify postcommit mismatch as indeterminate.
- Make About Noter open a truthful project dialog and label Markdown assistance
  as unavailable in the current prototype.

### Engineering

- Define and enforce repository-wide code-quality and evidence standards.
- Keep local agent state and runtime logs in ignored dedicated directories.
- Remove obsolete tracked agent metadata and commented-out build or CI plans.
- Expand the paired mutation union to 423 trust-kernel decisions with no
  platform gap; the local 418-mutant Windows classification has zero missed
  mutations and zero timeouts.
- Reject linker, compiler, storage, process, and tool-lock infrastructure
  failures that cargo-mutants would otherwise classify as unviable.
- Raise measured fixed-seed coverage to 93.06 percent for the trust kernel and
  90.09 percent for the complete workspace across 130 Windows-local tests.
