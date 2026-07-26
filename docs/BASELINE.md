# Noter Engineering Baseline

**Measured:** 2026-07-26

**Source state:** M0 evidence commit `7512534` after dependency cleanup. These
measurements are a development baseline, not release evidence.

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

## Current worktree checkpoint

This checkpoint measures the current pre-alpha worktree after the M1 storage
adapter, M2 shell work, and early M6 Markdown slice became reachable from the
GUI. It does not replace or reinterpret the M0 evidence above, and it is not yet
a milestone sign-off.

| Measure | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Pass |
| Strict workspace Clippy, locked, all targets and features | Pass |
| Windows-local workspace tests | 170 passed, 0 failed |
| Testable trust-kernel line coverage | 93.53 percent, 4,410 of 4,715 lines |
| Whole-workspace line coverage | 87.94 percent, 5,580 of 6,345 lines |
| Enforced development thresholds | Pass, trust kernel at least 90 percent and whole workspace at least 80 percent |
| Last completed Windows-applicable trust-kernel mutation testing | 418 total, 270 caught, 148 unviable, 0 missed, 0 timed out |
| Last completed Windows native-adapter mutation testing | 58 total, 40 caught, 18 unviable, 0 missed, 0 timed out |
| Current Windows native-adapter scope | 66 enumerated; full rerun pending |
| Focused Markdown diagnostics mutation testing | 58 total, 55 caught, 3 unviable, 0 missed, 0 timed out; one linker-lock result caught in an isolated rerun |
| Current supported-platform mutation union | 751 total: Linux 638, Windows 559, macOS 49, no union gap; exact-commit CI pending |
| Rustdoc with warnings denied | Pass |
| Local Markdown link check | Pass |
| Linux full-crate cross-target Clippy | Pass |
| macOS platform-crate cross-target Clippy | Pass |
| Release binary | 7,344,128 bytes, 7.00 MiB |
| Release SHA-256 | `040d7cf83f8a27ff02c7c06fdd9fed2d76505ee42fe026a9541fc92002eed5cb` |
| Resolved Cargo packages across all targets | 418 |
| RustSec audit | Pass, no known vulnerability reported |
| Cargo dependency policy | Pass for advisories, licenses, sources, and bans; duplicate versions remain visible as warnings |
| Native Windows, macOS, and Linux CI | Pass at `c76515c`, [run 30181088267](https://github.com/blisspixel/noter/actions/runs/30181088267) |
| Paired Linux and Windows mutation CI | Pass at `3830cdd`, [run 30184163737](https://github.com/blisspixel/noter/actions/runs/30184163737) |

The 170 tests comprise 117 primary-library unit tests, 34 binary application and
UI tests, one 19-case golden-corpus test, three generated property suites with
512 fixed-seed cases each, and 15 platform-crate tests. The stricter trust-kernel
report excludes the immediate-mode `src/app.rs`, `src/main.rs`, and
`src/markdown_ui.rs` adapters. Those files remain inside the separate 80 percent
whole-workspace gate. The filesystem adapter measures 91.44 percent line
coverage, the Markdown UI adapter 86.72 percent, and the full workspace 87.94
percent. Semantic UI automation and manual accessibility gates remain required
before v0.1.

The binary is 2,595,328 bytes larger than the M0 baseline. That measured cost
includes reachable BLAKE3 conflict fingerprints, native metadata transfer,
commit reconciliation, stable-handle loading, revision-aware saves, current
text shaping and rasterization, persisted themes, and the early native Markdown
surface. It remains below the 12 MiB first-release ceiling. The full locked
cross-target graph now contains 418 packages and still requires the release
duplicate, license, source, and capability audit.

The complete mutation campaign is recorded in
[M1_MUTATION_EVIDENCE.md](M1_MUTATION_EVIDENCE.md). The next evidence update must
attach manual platform metadata fixtures, weaker-filesystem observations, and
reproducible latency and memory measurements. Until then M1 remains In Progress.
