# Noter Engineering Baseline

**Measured:** 2026-07-25

**Source state:** `m0-foundation` at M0 evidence commit `7512534` after dependency
cleanup. These measurements are a development baseline, not release evidence.

## Reference environment

| Item | Value |
| --- | --- |
| Operating system | Windows 11 Pro, build 26200 |
| Processor | AMD Ryzen 9 5950X, 32 logical processors |
| Memory | 63.9 GiB |
| Rust | 1.97.1 |
| Cargo | 1.97.1 |
| Build profile | `release`, LTO, stripped, panic abort |

## Repository health

| Measure | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Pass |
| Strict Clippy, locked, all targets and features | Pass |
| Unit tests | 7 passed, 0 failed |
| Testable core line coverage | 96.43 percent |
| Whole-binary line coverage | 31.11 percent |
| Enforced development threshold | Pass, core at least 80 percent |
| Release binary | 4,748,800 bytes, 4.53 MiB |
| Resolved Cargo packages across all targets | 325 |
| Direct dependencies | 4 runtime, 1 development |
| GitHub Actions matrix | Pass on Windows, macOS, and Linux ([run 30176526028](https://github.com/blisspixel/noter/actions/runs/30176526028)) |

The development coverage command temporarily excludes `src/app.rs` and
`src/main.rs`. Those files are an untested GUI shell today. This is not a release
exception: v0.1 requires at least 80 percent whole-workspace line coverage plus
semantic UI and manual platform tests.

## Known gaps

- At the M0 evidence commit, the document module exceeded the initial percentage
  target but still lacked its I/O adapter, property tests, failure injection, and
  mutation evidence.
- The current GUI shell has one About-window smoke test but still lacks the
  semantic command, state, accessibility, and visual coverage required for v0.1.
- Startup, input latency, open latency, RSS, and long-file measurements do not
  yet have a reproducible harness.
- Duplicate target-specific dependency families remain in the GUI stack and
  require a release audit.

## M1 local adapter checkpoint

This checkpoint measures the current M1 worktree after the production storage
adapter became reachable from the GUI. It does not replace or reinterpret the M0
evidence above, and it is not yet a milestone sign-off.

| Measure | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Pass |
| Strict workspace Clippy, locked, all targets and features | Pass |
| Windows-local workspace tests | 134 passed, 0 failed |
| Testable trust-kernel line coverage | 93.13 percent, 3,904 of 4,192 lines |
| Whole-workspace line coverage | 90.18 percent, 4,408 of 4,888 lines |
| Enforced development threshold | Pass, at least 90 percent |
| Windows-applicable trust-kernel mutation testing | 418 total, 270 caught, 148 unviable, 0 missed, 0 timed out |
| Windows native-adapter mutation testing | 58 total, 40 caught, 18 unviable, 0 missed, 0 timed out |
| Expanded supported-platform mutation union | 639 total: Linux 556, Windows 476, macOS 169, no union gap; exact-commit CI pending |
| Rustdoc with warnings denied | Pass |
| Local Markdown link check | Pass |
| Linux full-crate cross-target Clippy | Pass |
| macOS platform-crate cross-target Clippy | Pass |
| Release binary | 4,953,088 bytes, 4.72 MiB |
| Release SHA-256 | `78e2b19a274ab3b3c306fc9fa9e7de40c3ed6dfc64db029d20db739af7b63be3` |
| Resolved Cargo packages across all targets | 339 |
| RustSec audit | Pass, no known vulnerability reported |
| Native Windows, macOS, and Linux CI | Pass at `c76515c`, [run 30181088267](https://github.com/blisspixel/noter/actions/runs/30181088267) |
| Paired Linux and Windows mutation CI | Pass at `3830cdd`, [run 30184163737](https://github.com/blisspixel/noter/actions/runs/30184163737) |

The 134 tests comprise 104 primary-library unit tests, 12 binary application
tests, one 19-case golden-corpus test, three generated property suites with 512
fixed-seed cases each, and 14 platform-crate tests. Coverage excludes the
still-prototype `src/app.rs` and `src/main.rs` under the same explicit CI rule as
M0 for the trust-kernel gate. The filesystem adapter itself measures 91.13
percent line coverage, and the trust-kernel total remains above the M1 90
percent gate. A separate unfiltered report measures the whole workspace at
90.18 percent. The UI exclusion from the stricter gate is temporary and is
replaced by semantic UI and manual accessibility gates before v0.1.

The binary grew by 204,288 bytes from the M0 baseline. That measured cost now
includes reachable BLAKE3 conflict fingerprints, native metadata transfer, commit
reconciliation, stable-handle loading, revision-aware saves, and the truthful
About dialog. The lock graph
grew by 14 packages from M0: eight test-only property packages, four BLAKE3
packages, one internal workspace member, and one `xattr` package used on Linux
and macOS.

The complete mutation campaign is recorded in
[M1_MUTATION_EVIDENCE.md](M1_MUTATION_EVIDENCE.md). The next evidence update must
attach manual platform metadata fixtures, weaker-filesystem observations, and
reproducible latency and memory measurements. Until then M1 remains In Progress.
