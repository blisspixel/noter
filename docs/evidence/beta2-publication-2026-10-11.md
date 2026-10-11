# Beta.2 Publication Verification

**Date:** 2026-10-11

**Published:** 2026-10-11 15:13:23 UTC. Public prerelease and artifact readback passed.

## Immutable release identity

- Release: [v0.1.0-beta.2](https://github.com/blisspixel/noter/releases/tag/v0.1.0-beta.2).
- Source and tag commit: `9cdaa3b8cef9a85259236302654cbf40c34e7339`.
- Protected integration: [PR #49](https://github.com/blisspixel/noter/pull/49),
  rebased after exact-head CI and two independent reviews.
- Publication: [Release run 38149804354](https://github.com/blisspixel/noter/actions/runs/38149804354).
- Machine-readable asset and verification record: [JSON receipt](beta2-publication-2026-10-11.json).

The protected rebase preserved the complete tree reviewed at
`05cbb69b217cef7b171dbe94ef3038b735c9d53e`. `git diff --exit-code` between that
candidate and the release source passed. Main remained frozen during publication.
The immutable tag identifies the released source even after later documentation
commits update main.

## Automated and artifact gates

| Gate | Exact revision | Result |
| --- | --- | --- |
| [PR CI 38139971077](https://github.com/blisspixel/noter/actions/runs/38139971077) | `05cbb69` | All native suites, lint, documentation, audit/policy, coverage, 14 mutation workers, and aggregate gate passed |
| [Main-push CI 38144668329](https://github.com/blisspixel/noter/actions/runs/38144668329) | `9cdaa3b` | Every required job passed before publication |
| [Dry run 38139972098](https://github.com/blisspixel/noter/actions/runs/38139972098) | `05cbb69` | Four target builds and global assembly passed; host skipped; downloaded 20-file payload inspected |
| [Publication 38149804354](https://github.com/blisspixel/noter/actions/runs/38149804354) and public readback | `9cdaa3b` | Four builds, global assembly, exact-main/tag checks, draft upload validation, attestation, publication, and public verification passed |

Main's Linux LCOV reports 41,537/44,372 covered workspace lines (93.61 percent)
and 19,794/21,284 trust-kernel lines (93.00 percent), using the exact unchanged
CI exclusions. Both required floors pass.

Downloaded PR and main mutation reports passed baseline and infrastructure
checks. Every mutant outcome was caught or unviable, with zero missed,
timed-out, or surviving mutants. The JSON records each shard's counts. Platform
scopes overlap; their sum is not a unique repository-wide mutation count.

Two independent final reviews, including fresh-context review, scored all seven
required categories 4/5 and approved the exact candidate. Focused self-review
after automated gates found no blocker. The
[final review receipt](https://github.com/blisspixel/noter/pull/49#issuecomment-6109699570)
and [main preflight receipt](https://github.com/blisspixel/noter/pull/49#issuecomment-6110359772)
record those decisions and their limits.

## Public package verification

Downloaded all 21 public assets: the reviewed 20-file payload plus the canonical
distribution manifest. Every name, size, uploaded state, and server-side SHA-256
digest matched the local file. All six checksum sidecars and the exact unified
checksum list passed, as did the version-bound distribution manifest.

All 21 GitHub attestations passed with the release workflow, exact source digest,
`refs/heads/main`, and hosted-runner policy enforced. One initial parallel CLI
verification failed; a direct retry and the complete sequential rerun passed
with identical policy and unchanged files. No check was bypassed.

All four archives contain the immutable source's README, changelog, license,
third-party notices, and font license. The source archive matches all 147 tracked
Git blobs. The Windows archive executable reports `noter 0.1.0-beta.2`.
Read-only MSI inspection confirms ProductVersion `0.0.4`, beta.2 display version,
and the unchanged upgrade identity. Four CycloneDX 1.5 SBOMs identify beta.2,
with 200 Apple Silicon macOS, 201 Intel macOS, 192 Windows, and 270 Linux
dependency components.

After provenance verification, the published PowerShell installer completed a
fresh install and reinstall into an isolated unmanaged directory on this
Windows host. Both installed executables reported beta.2 and matched the
verified archive binary's SHA-256:
`c3d3cbf1158763517ead0afe78539717658b22fbfe628f6642e13d8751c2bd0d`.
`NOTER_UNMANAGED_INSTALL` and `NOTER_NO_MODIFY_PATH=1` contained the fixture;
the user PATH stayed unchanged. This is not the clean-machine MSI matrix.

The public verification used:

```sh
gh release download v0.1.0-beta.2 --dir public-beta2
gh attestation verify PATH_TO_ASSET --repo blisspixel/noter --signer-workflow blisspixel/noter/.github/workflows/release.yml --source-digest 9cdaa3b8cef9a85259236302654cbf40c34e7339 --source-ref refs/heads/main --deny-self-hosted-runners --format json
```

Repeat the attestation command for every downloaded asset. The JSON receipt
records all 21 SHA-256 values, sizes, package identities, coverage, mutation
counts, and installation observations. SHA-256 sidecars cover only their named
source archive, platform archives, and MSI; the other files rely on provenance.

## Native scope and remaining gates

The [Windows source-build record](beta2-windows-2026-10-11.md) covers edit/save,
Undo/Redo, failed-save and conflict retention, recovery, menu spacing, and both
CRT themes on `ed67c26` at the observed host scale. Five deterministic captures
were regenerated and visually reviewed at 100 percent. These observations do
not establish a complete native 100/150 percent display-scale matrix or a
clean-machine packaged installation matrix.

Linux process fixtures exercise source/binary installation, exact Desktop Entry
escaping, GIO selected-file delivery, and uninstall ownership. The user's
Omarchy/Hyprland retest remains postponed, so
[#44](https://github.com/blisspixel/noter/issues/44) stays open and compositor
resolution is unverified. Visible playtest fixtures use English; automated
Unicode, wide-character, and control-character cases remain required.

Windows and macOS artifacts are not platform-signed. GitHub attestation proves
build provenance and is separate from platform signing. Recovery support stays
limited to documented local owner-controlled roots. Keep backups during
prerelease use.

The full recovery/compositor matrix, measured editor feasibility gate, and
continuous Markdown remain beta.3 work in
[#50](https://github.com/blisspixel/noter/issues/50),
[#51](https://github.com/blisspixel/noter/issues/51), and
[#52](https://github.com/blisspixel/noter/issues/52). M6 quality tools, signing,
clean-machine packaging, full platform/scale evidence, and required RC dogfood
remain tracked in [#54](https://github.com/blisspixel/noter/issues/54) and
[#53](https://github.com/blisspixel/noter/issues/53). This scoped release does
not mark those exit criteria Verified.
