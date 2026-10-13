# Beta.2.1 Publication Verification

**Date:** 2026-10-13

**Published:** 2026-10-13 08:03:57 UTC. Public prerelease and artifact readback passed.

## Immutable release identity

- Release: [v0.1.0-beta.2.1](https://github.com/blisspixel/noter/releases/tag/v0.1.0-beta.2.1).
- Source and tag commit: `4753953bd1748089752b7a173230e4909a5a723c`.
- Product follow-up: [PR #62](https://github.com/blisspixel/noter/pull/62),
  squash-merged as `c0f58e2`.
- Installer sequence fix: [PR #63](https://github.com/blisspixel/noter/pull/63),
  squash-merged as `4753953`. Its tree matches reviewed `9f15629`.
- Publication: [Release run 38365404126](https://github.com/blisspixel/noter/actions/runs/38365404126).
- Machine-readable asset record: [JSON receipt](beta2.1-publication-2026-10-13.json).

This is a scoped prerelease. It is not beta.3, not the continuous Markdown
workflow, and not the release candidate. The tag stays on `4753953` after this
later documentation commit.

## Automated gates

| Gate | Exact revision | Result |
| --- | --- | --- |
| [PR CI 38342618798](https://github.com/blisspixel/noter/actions/runs/38342618798) | `9f15629` | Docs, formatting, three operating-system test jobs, and all mutation shards passed. Windows shard 5 was canceled at the 90-minute ceiling on the first attempt, with every reached mutant caught, and passed on rerun |
| [Main-push CI 38356789437](https://github.com/blisspixel/noter/actions/runs/38356789437) | `4753953` | Every required job passed before publication |
| [Dry run 38352612688](https://github.com/blisspixel/noter/actions/runs/38352612688) | `9f15629` | Four target builds, including the Windows MSI, and global assembly passed; host skipped |
| [Publication 38365404126](https://github.com/blisspixel/noter/actions/runs/38365404126) | `4753953` | Plan, four builds, global assembly, and host passed |

Two independent reviews approved the follow-up at `94603b8`. A separate review
approved the cargo-wix numeric override at `9f15629`. Those reviews did not
close issues
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
those files. `dist-manifest.json` announces `v0.1.0-beta.2.1` and application
version `0.1.0-beta.2.1`.

All 21 GitHub attestations passed on the first attempt with the release
workflow, source digest `4753953bd1748089752b7a173230e4909a5a723c`,
`refs/heads/main`, and hosted-runner policy enforced. Attestation is build
provenance. Windows and macOS artifacts are not platform-signed.

The source archive and the four platform archives contain the README,
changelog, license, third-party notices, and Inter font license. The source
`Cargo.toml` package version is `0.1.0-beta.2.1`. The Windows archive
executable reports `noter 0.1.0-beta.2.1` and has SHA-256
`a2e70b552d08a15a752f43b3a8f54ee8d02278b5f137a7fb9061f85c0cb977d8`.
Read-only MSI inspection shows ProductVersion `0.0.5`, display version
`0.1.0-beta.2.1`, product name Noter, and upgrade code
`04EFB570-9D04-49E1-B908-F5CEBD979E1B`. Four CycloneDX 1.5 SBOMs identify
`0.1.0-beta.2.1`, with 200 Apple Silicon macOS, 201 Intel macOS, 192 Windows,
and 270 Linux dependency components.

The published PowerShell installer adds its bin directory to the user PATH.
This check did not run that installer. It executed `noter --version` from the
verified Windows archive instead. This is not a clean-machine install,
reinstall, or uninstall matrix.

```sh
gh release download v0.1.0-beta.2.1 --dir public-beta2.1
gh attestation verify PATH_TO_ASSET --repo blisspixel/noter --signer-workflow blisspixel/noter/.github/workflows/release.yml --source-digest 4753953bd1748089752b7a173230e4909a5a723c --source-ref refs/heads/main --deny-self-hosted-runners --format json
```

## Remaining gates

Issues #44 and #50 through #54, plus #59 and #60, stay open. The native
Hyprland retest, the rest of the recovery matrix, the measured editor gate,
continuous Markdown, the M6 quality engine, platform signing, clean-machine
packaging, and the 14-day RC dogfood are not verified by this tag.
