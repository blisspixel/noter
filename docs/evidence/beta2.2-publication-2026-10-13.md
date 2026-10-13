# Beta.2.2 Publication Verification

**Date:** 2026-10-13

**Published:** 2026-10-13 18:05:15 UTC. Public prerelease and artifact readback passed.

## Immutable release identity

- Release: [v0.1.0-beta.2.2](https://github.com/blisspixel/noter/releases/tag/v0.1.0-beta.2.2).
- Source and tag commit: `e835efb9a7efd2f3d417d6e8558d90c59bed8259`.
- Retained-sibling bound: [PR #65](https://github.com/blisspixel/noter/pull/65),
  squash-merged as `79a4cfc`.
- Version identity: [PR #66](https://github.com/blisspixel/noter/pull/66),
  squash-merged as `e835efb`.
- Publication: [Release run 38442664870](https://github.com/blisspixel/noter/actions/runs/38442664870).
- Machine-readable asset record: [JSON receipt](beta2.2-publication-2026-10-13.json).

This is a scoped prerelease. It is not beta.3, not the continuous Markdown
workflow, and not the release candidate. The tag stays on `e835efb` after this
later documentation commit.

## Automated gates

| Gate | Exact revision | Result |
| --- | --- | --- |
| [PR CI 38405544469](https://github.com/blisspixel/noter/actions/runs/38405544469) | `95adb04` | Docs, formatting, three operating-system test jobs, and all mutation shards passed |
| [Main-push CI 38428347904](https://github.com/blisspixel/noter/actions/runs/38428347904) | `e835efb` | Every required job passed before publication |
| [Dry run 38440084516](https://github.com/blisspixel/noter/actions/runs/38440084516) | `e835efb` | Four target builds, including the Windows MSI, and global assembly passed; host skipped |
| [Publication 38442664870](https://github.com/blisspixel/noter/actions/runs/38442664870) | `e835efb` | Plan, four builds, global assembly, and host passed |

The parent commit `79a4cfc` had its own main-push CI. Windows mutation shards
1 and 5 hit the 90-minute ceiling on the first attempt, with no missed mutant
recorded, and passed on rerun before this tag was cut. This publication does
not close issues
[#44](https://github.com/blisspixel/noter/issues/44),
[#50](https://github.com/blisspixel/noter/issues/50),
[#51](https://github.com/blisspixel/noter/issues/51),
[#52](https://github.com/blisspixel/noter/issues/52),
[#53](https://github.com/blisspixel/noter/issues/53),
[#54](https://github.com/blisspixel/noter/issues/54),
[#59](https://github.com/blisspixel/noter/issues/59), or
[#60](https://github.com/blisspixel/noter/issues/60).

## Public package verification

Downloaded all 21 public assets. Every name, size, and GitHub SHA-256 digest
matched the local file. All six checksum sidecars and `sha256.sum` matched
those files. `dist-manifest.json` announces `v0.1.0-beta.2.2` and application
version `0.1.0-beta.2.2`.

All 21 GitHub attestations passed on the first attempt with the release
workflow, source digest `e835efb9a7efd2f3d417d6e8558d90c59bed8259`,
`refs/heads/main`, and hosted-runner policy enforced. Attestation is build
provenance. Windows and macOS artifacts are not platform-signed.

The source archive and the four platform archives contain the README,
changelog, license, third-party notices, and Inter font license. The source
`Cargo.toml` package version is `0.1.0-beta.2.2`. The Windows archive
executable reports `noter 0.1.0-beta.2.2` and has SHA-256
`cd31d479c9e768d689a9abc787da0f4fbbdb41042d6a4ca7bca6a6324aac1e00`.
Read-only MSI inspection shows ProductVersion `0.0.6`, display version
`0.1.0-beta.2.2`, product name Noter, and upgrade code
`04EFB570-9D04-49E1-B908-F5CEBD979E1B`. Four CycloneDX 1.5 SBOMs identify
`0.1.0-beta.2.2`, with 200 Apple Silicon macOS, 201 Intel macOS, 192 Windows,
and 270 Linux dependency components.

The published PowerShell installer adds its bin directory to the user PATH.
This check did not run that installer. It executed `noter --version` from the
verified Windows archive instead. This is not a clean-machine install,
reinstall, or uninstall matrix.

```sh
gh release download v0.1.0-beta.2.2 --dir public-beta2.2
gh attestation verify PATH_TO_ASSET --repo blisspixel/noter --signer-workflow blisspixel/noter/.github/workflows/release.yml --source-digest e835efb9a7efd2f3d417d6e8558d90c59bed8259 --source-ref refs/heads/main --deny-self-hosted-runners --format json
```

## Remaining gates

Issues #44 and #50 through #54, plus #59 and #60, stay open. The native
Hyprland retest, the rest of the recovery matrix, the measured editor gate,
continuous Markdown, the M6 quality engine, platform signing, clean-machine
packaging, and the 14-day RC dogfood are not verified by this tag.
