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
- The GUI shell has no automated UI coverage.
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
| Workspace tests | 81 passed, 0 failed |
| Testable trust-kernel line coverage | 92.76 percent |
| Enforced development threshold | Pass, at least 90 percent |
| Rustdoc with warnings denied | Pass |
| Local Markdown link check | Pass |
| Linux full-crate cross-target Clippy | Pass |
| macOS platform-crate cross-target Clippy | Pass |
| Release binary | 4,871,680 bytes, 4.65 MiB |
| Release SHA-256 | `23747D13CE4B081D4794B5CE4907381200C07C2F4D021D00A2A6109FF79C2E5C` |
| Resolved Cargo packages across all targets | 339 |
| RustSec audit | Pass, no known vulnerability reported |
| Native Windows, macOS, and Linux CI | Pending for this worktree |

The 81 tests comprise 71 primary-crate unit tests, one 19-case golden-corpus
test, three generated property suites with 512 cases each, and six platform-crate
tests. Coverage excludes the still-prototype `src/app.rs` and `src/main.rs` under
the same explicit CI rule as M0. The new adapter itself measures 89.23 percent
line coverage, and the total remains above the M1 90 percent gate.

The binary grew by 122,880 bytes from the M0 baseline. That measured cost now
includes reachable BLAKE3 conflict fingerprints, native metadata transfer, commit
reconciliation, stable-handle loading, and revision-aware saves. The lock graph
grew by 14 packages from M0: eight test-only property packages, four BLAKE3
packages, one internal workspace member, and one Linux-only `xattr` package.

The next evidence update must attach native CI results, mutation results, manual
platform metadata fixtures, weaker-filesystem observations, and reproducible
latency and memory measurements. Until then M1 remains In Progress.
