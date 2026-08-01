# M1 Reproducible Baseline Evidence

**Measured:** 2026-07-31 local time, 2026-08-01T03:42:22.971077Z

**Status:** Valid M1 development reference. This is not M1 milestone sign-off.

## Evidence identity

The canonical artifact is
[`evidence/m1-baseline-windows-2026-07-31.json`](evidence/m1-baseline-windows-2026-07-31.json).
It contains every raw latency and memory sample, the exact deterministic corpus
manifest, environment and toolchain facts, binary and worker hashes, and the
resolved dependency summary.

| Field | Value |
| --- | --- |
| Artifact SHA-256 | `5da4643bf7f84c2ae37605c35a91c52e6e4f85fb0f06052f8ddfc0161bfd47e8` |
| Schema | 2 |
| Source commit | `580f16409957ecf0a3ff074a24703937231ca05d` |
| Source tree | `405a6d24bdb091fdc905f1a877cfd6cde8c97286` |
| Checkout | Clean detached Git worktree |
| Corpus SHA-256 | `62f09d6fe9972e1ca36d66142fdfca0e1bfdcdeac1697da117b536a6a0815016` |
| Result sets | 22 across 15 cases |
| Samples | 30 measured samples per set |
| Warmup | 5 operations for each warm-in-process set |

## Method

The committed harness generated seven exact UTF-8 files covering empty input,
1 MiB prose, mixed Unicode and line endings, newline-only content, one 1 MiB
line, 50 MiB source-like content, and 50 MiB log-like content. Search cases use
exact early, middle, late, absent, and adversarial markers.

The reference command first recorded the clean commit and tree, created a
detached linked worktree for that commit, then built and executed that
worktree's harness. Process-cold load sets use one measured operation in each
fresh worker. Warm sets use one worker after five explicit warmups. The
operating-system file cache is intentionally uncontrolled and disclosed in the
artifact. Percentiles use nearest-rank selection over all 30 raw samples.

Load timing includes stable-handle inspection, bounded reading, validation, and
Rope construction. Save timing includes the platform persistence barriers. The
harness verifies search offsets and compares saved bytes with the intended
content outside the measured interval so correctness checks do not inflate the
reported operation latency. Windows commands are created suspended, assigned
to a kill-on-close Job Object, and activated only after association. Command
output, samples, warmups, artifacts, deadlines, and temporary paths are bounded.
The later command-runner hardening uses bounded terminate-and-rescan waves to
retain identity-checked handles, stop assigned processes, and wait for their
process objects to signal. A final Job Object termination and active-count
check run under the same fixed deadline before returning an ordinary
output-limit or command-deadline error. That shutdown protocol was not present
at recorded commit `580f164` and does not alter the historical timing artifact.

## Reference environment

| Item | Value |
| --- | --- |
| Operating system | Windows 11, version 10.0.26200 |
| Processor | AMD Ryzen 9 5950X 16-Core Processor, 32 logical processors |
| Physical memory | 63.92 GiB |
| Storage | Local NTFS volume |
| Display refresh | 120 Hz |
| Rust | `rustc 1.97.1 (8bab26f4f 2026-07-14)` |
| Cargo | `cargo 1.97.1 (c980f4866 2026-06-30)` |
| Python | 3.14.5 |
| Rust build profile | `bench` for the worker, `release` for the application |
| Memory metric | Peak working set per measured worker |

## Results

Times are milliseconds. Memory is the maximum observed peak working set for the
set, in MiB. Exact nanosecond and byte samples remain in the JSON artifact.

| Case | State | p50 | p95 | p99 | Peak MiB |
| --- | --- | ---: | ---: | ---: | ---: |
| Load empty | Process cold | 0.24 | 0.30 | 0.37 | 4.66 |
| Load empty | Warm in process | 0.18 | 0.21 | 0.25 | 4.64 |
| Load 1 MiB prose | Process cold | 2.08 | 2.47 | 2.49 | 6.93 |
| Load 1 MiB prose | Warm in process | 1.92 | 2.46 | 2.56 | 8.47 |
| Load 1 MiB mixed Unicode and EOL | Process cold | 2.19 | 2.51 | 3.34 | 6.93 |
| Load 1 MiB mixed Unicode and EOL | Warm in process | 2.08 | 2.52 | 2.63 | 8.22 |
| Load 1 MiB newlines | Process cold | 2.36 | 2.76 | 2.87 | 6.92 |
| Load 1 MiB newlines | Warm in process | 2.14 | 2.47 | 2.70 | 8.48 |
| Load 1 MiB long line | Process cold | 1.94 | 2.28 | 2.29 | 6.92 |
| Load 1 MiB long line | Warm in process | 1.87 | 2.32 | 2.64 | 8.46 |
| Load 50 MiB source | Process cold | 91.34 | 104.96 | 105.05 | 112.61 |
| Load 50 MiB source | Warm in process | 96.76 | 103.06 | 104.31 | 173.07 |
| Load 50 MiB log | Process cold | 94.94 | 101.70 | 101.78 | 112.61 |
| Load 50 MiB log | Warm in process | 94.05 | 111.16 | 116.93 | 171.40 |
| Search early in 50 MiB | Warm in process | 2.34 | 2.61 | 2.74 | 54.78 |
| Search middle in 50 MiB | Warm in process | 2.39 | 2.82 | 3.06 | 54.79 |
| Search late in 50 MiB | Warm in process | 2.42 | 2.87 | 3.12 | 54.79 |
| Search absent in 50 MiB | Warm in process | 2.38 | 2.87 | 3.02 | 54.78 |
| Search adversarial in 50 MiB | Warm in process | 3.80 | 4.49 | 4.75 | 54.79 |
| Serialize 1 MiB prose | Warm in process | 0.65 | 0.76 | 0.89 | 9.68 |
| Save new 1 MiB prose | Warm in process | 12.14 | 13.64 | 13.80 | 12.92 |
| Replace 1 MiB prose | Warm in process | 16.62 | 17.93 | 18.34 | 13.99 |

The release application binary is 9,299,456 bytes, or 8.87 MiB, with SHA-256
`fa264e20a9a9c20e1e55c4c44b692a960c378155b4b12c7d97f97dcde8845b64`.
It remains below the 12 MiB first-release ceiling. The benchmark worker is
1,938,944 bytes with SHA-256
`5e1f785295222228454f5f9ffe7d59dd7dbc284bc946c89ff23caa7868287586`.

The locked graph contains 416 package records. The union resolved for the four
release targets contains 344 packages: 212 for Windows x86-64, 218 for Apple
Silicon macOS, 219 for Intel macOS, and 290 for Linux x86-64. The project has 10
direct runtime dependencies, 2 direct development dependencies, and no direct
build dependency. Twenty-one duplicate-version families remain visible in the
artifact and continue to be reviewed by dependency policy.

## Interpretation and limits

The evidence establishes a reproducible UI-independent trust-kernel baseline
for the named commit and machine. It does not verify GUI launch, painted-frame
latency, interactive 50 MiB editing, IME, accessibility, display scaling, or
end-to-end user input. Those remain M5 gates. It also does not compare operating
systems or filesystems and does not claim that one local run predicts another
machine.

The artifact provenance is `self-reported-local-run` with authentication
`none`. Exact-head CI can validate the committed bytes, schema, calculations,
source identifiers, and harness. It cannot authenticate the historical timing
event. M1 also remains open until required native and weaker-filesystem fixtures
are executed and reported without overstating durability.

Separate local evidence now covers native NTFS, native WSL2 ext4, and a
fail-closed Windows-to-WSL boundary against source bytes later committed
unchanged as `65ac25f`; see
[M1_FILESYSTEM_EVIDENCE.md](M1_FILESYSTEM_EVIDENCE.md). That record does not
cover the remaining macOS, SMB, cloud, removable, weak-filesystem,
second-identity, or crash-persistence fixtures.
The related exact focused Windows private-security validation is retained in a
[machine-readable mutation record](evidence/m1-windows-private-security-mutation-2026-07-31.json).

## Reproduce and validate

Run a new reference under an unused JSON filename from a clean checkout:

```powershell
python scripts/run_m1_baseline.py --output docs/evidence/m1-baseline-windows-YYYY-MM-DD.json --evidence-class reference --samples 30 --warmup 5
```

Validate the committed artifact and its source commit when that commit is
available in the local Git object database:

```powershell
python scripts/run_m1_baseline.py --validate-artifact docs/evidence/m1-baseline-windows-2026-07-31.json
```
