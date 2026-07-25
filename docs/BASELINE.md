# Noter Engineering Baseline

**Measured:** 2026-07-25

**Source state:** working tree based on local commit `3e60e1c`, after the first
M0 repairs. The branch was five commits ahead of and one commit behind
`origin/master`. These measurements are a development baseline, not release
evidence for a published commit.

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
| Whole-binary line coverage | 31.03 percent |
| Enforced development threshold | Pass, core at least 80 percent |
| Release binary | 5,441,536 bytes, 5.19 MiB |
| Resolved Cargo packages across all targets | 413 |
| Direct dependencies | 11 runtime, 1 development |
| GitHub Actions matrix | Not yet run for this divergent local tree |

The development coverage command temporarily excludes `src/app.rs` and
`src/main.rs`. Those files are an untested GUI shell today. This is not a release
exception: v0.1 requires at least 80 percent whole-workspace line coverage plus
semantic UI and manual platform tests.

## Known gaps

- The local and remote histories must be reconciled before CI can validate the
  exact worktree on Windows, macOS, and Linux.
- The current document module exceeds the M1 percentage target, but M1 still
  lacks its I/O adapter, property tests, failure injection, and mutation evidence.
- The GUI shell has no automated UI coverage.
- Startup, input latency, open latency, RSS, and long-file measurements do not
  yet have a reproducible harness.
- Duplicate target-specific dependency families remain in the GUI stack and
  require a release audit.

The next baseline replaces this file only after the measurements are scripted
or otherwise reproducible and tied to a green commit.
