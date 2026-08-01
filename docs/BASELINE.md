# Noter Engineering Baseline

**M0 measured:** 2026-07-26

**Latest verified implementation checkpoint:** 2026-07-31

**M0 source state:** Evidence commit `7512534` after dependency cleanup. These
measurements are a development baseline, not release evidence.

## Latest exact implementation checkpoint

Commit `bfdeb55` is the latest verified implementation checkpoint. Protected
main-branch run
[30612842346](https://github.com/blisspixel/noter/actions/runs/30612842346)
completed all nine required jobs successfully for exact commit
`bfdeb55fb5b903421dd2db6aa093b76e4130ac55`.

| Measure | Result |
| --- | --- |
| Hosted Linux Rust tests | 411 passed, 0 failed |
| Whole-workspace line coverage | 93.38 percent, 14,736 of 15,781 lines |
| Trust-kernel line coverage | 94.44 percent, 7,387 of 7,822 lines |
| Linux mutation scope | 967 total, 718 caught, 249 unviable, 0 missed, 0 timed out |
| Windows mutation scope | 901 total across two required shards, 656 caught, 245 unviable, 0 missed, 0 timed out |
| macOS mutation scope | 47 total, 41 caught, 6 unviable, 0 missed, 0 timed out |
| Mutation infrastructure validation | Pass in all four mutation jobs; no recognized compiler, linker, process, storage, or tool failure hidden as unviable |
| Source installers | Pass on Windows, macOS, and Linux using custom roots containing spaces |
| Formatting, Clippy, rustdoc, dependency audit and policy | Pass |
| Documentation, validation-script lint, and validation-script tests | Pass |

Platform mutation scopes overlap and are not added into a synthetic unique
total. The historical union below remains the evidence for the earlier declared
scope; the current counts show that the required per-platform gates continued to
evolve with implementation.

## Reference environment

| Item | Value |
| --- | --- |
| Operating system | Windows 11 Pro, build 26200 |
| Processor | AMD Ryzen 9 5950X, 32 logical processors |
| Memory | 63.9 GiB |
| Rust | 1.97.1 |
| Cargo | 1.97.1 |
| Build profile | `release`, LTO, stripped, panic abort |

## 2026-07-31 reproducible M1 trust-kernel reference

Commit `580f164` adds the reproducible M1 harness. Its canonical Windows run
uses a clean detached worktree, an exact seven-file corpus, 30 raw samples per
set, five warmups for warm sets, and 22 result sets across stable-handle load,
literal search, serialization, exclusive save, and atomic replacement.

| Measure | Result |
| --- | --- |
| Reference artifact | [M1_BASELINE_EVIDENCE.md](M1_BASELINE_EVIDENCE.md) |
| Artifact SHA-256 | `5da4643bf7f84c2ae37605c35a91c52e6e4f85fb0f06052f8ddfc0161bfd47e8` |
| 50 MiB source load p95 | 104.96 ms process cold, 103.06 ms warm in process |
| 50 MiB log load p95 | 101.70 ms process cold, 111.16 ms warm in process |
| 50 MiB literal search p95 | 2.61 to 4.49 ms across early, middle, late, absent, and adversarial cases |
| 1 MiB save p95 | 13.64 ms exclusive new file, 17.93 ms atomic replacement |
| Maximum observed worker peak working set | 173.07 MiB |
| Release binary | 9,299,456 bytes, 8.87 MiB |
| Four-target resolved package union | 344 packages from 416 locked records |

The complete raw samples, corpus manifest, environment, binary hashes,
dependency counts, method, and limitations are retained in the canonical JSON.
This is a self-reported local trust-kernel baseline, not authenticated telemetry
or M5 GUI and input evidence. The required M1 filesystem fixture set remains
open.

## 2026-07-31 filesystem evidence checkpoint

Source bytes later committed unchanged as `65ac25f` have local synthetic fixture
evidence for native NTFS, native WSL2 ext4, and the Windows-to-WSL UNC boundary.
The record includes exact byte checksums, NTFS DACL, stream and compression
observations, ext4 mode and extended-attribute observations, warning and
durability classifications, and a fail-closed cross-filesystem privacy
regression. See
[M1_FILESYSTEM_EVIDENCE.md](M1_FILESYSTEM_EVIDENCE.md).

Exact clean-detached follow-up validation at `994e0a3` passes 425 Windows
workspace tests with 93.49 percent whole-workspace, 95.23 percent trust-kernel,
and 92.14 percent platform-adapter line coverage. Its focused Windows private-
security mutation campaign catches all 20 candidates with no unviable, missed,
or timed-out result after the initial campaign exposed three token-length
boundary survivors. Commands, candidates, outcomes, and artifact hashes are in
the [machine-readable mutation record](evidence/m1-windows-private-security-mutation-2026-07-31.json).

This checkpoint narrows the manual M1 gap but does not close it. Native macOS,
SMB, cloud-synchronized, removable, weak-filesystem, second-identity, and crash
persistence evidence remains unavailable or unsafe to obtain on this host.

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
- Semantic command and state tests now cover the implemented GUI surface, but
  installed-product accessibility and cross-platform visual evidence remain
  required for v0.1.
- GUI startup, painted-frame input latency, IME, accessibility, and interactive
  long-file measurements still require the M5 harness. M1 trust-kernel load,
  search, save, memory, binary-size, and dependency measurements now have a
  reproducible reference.
- Duplicate target-specific dependency families remain in the GUI stack and
  require a release audit.

## 2026-07-27 engineering checkpoint

This checkpoint measures the development implementation after the M1
storage adapter, M2 shell work, and early M6 Markdown slice became reachable
from the GUI. It does not replace or reinterpret the M0 evidence above, and it
is not yet a milestone sign-off.

| Measure | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Pass |
| Strict workspace Clippy, locked, all targets and features | Pass |
| Windows-local workspace tests | 208 passed, 0 failed |
| Testable trust-kernel line coverage | 93.73 percent, 4,498 of 4,799 lines |
| Whole-workspace line coverage | 89.49 percent, 6,657 of 7,439 lines |
| Enforced development thresholds | Pass, trust kernel at least 90 percent and whole workspace at least 80 percent |
| Historical focused Windows trust-kernel mutation testing | 418 total, 270 caught, 148 unviable, 0 missed, 0 timed out |
| Historical focused Windows native-adapter mutation testing | 58 total, 40 caught, 18 unviable, 0 missed, 0 timed out |
| Current Windows native-adapter scope | 66 candidates included in the exact full Windows run; no scope gap |
| Focused Markdown diagnostics mutation testing | 58 total, 55 caught, 3 unviable, 0 missed, 0 timed out; one linker-lock result caught in an isolated rerun |
| Focused lifecycle and save-result mutation testing | 26 total, 26 caught, 0 unviable, 0 missed, 0 timed out |
| Focused native Markdown editing and rendering mutation testing | 88 total, 80 caught, 8 compiler rejections, 0 missed, 0 timed out |
| Focused final-entry observation mutation testing | 16 total, 12 caught, 4 compiler rejections, 0 missed, 0 timed out |
| Current supported-platform mutation union | 741 total with no union gap; Linux 617 total, 438 caught, 179 unviable; Windows 557 total, 381 caught, 176 unviable; macOS 49 total, 43 caught, 6 unviable; 0 missed and 0 timed out in every scope |
| Rustdoc with warnings denied | Pass |
| Local Markdown link check | Pass |
| Linux full-crate cross-target Clippy | Pass |
| macOS platform-crate cross-target Clippy | Pass |
| Release binary | 8,075,776 bytes, 7.70 MiB |
| Release checksum | Not published until reproducible, signed release artifacts exist |
| Resolved Cargo packages across all targets | 413 |
| RustSec audit | Pass, no known vulnerability reported |
| Cargo dependency policy | Pass for advisories, licenses, sources, and bans; duplicate versions remain visible as warnings |
| Exact-commit Windows, macOS, and Linux CI | Pass at `97371d8`, including all three mutation scopes and strengthened infrastructure validation, [run 30221793209](https://github.com/blisspixel/noter/actions/runs/30221793209) |

The 208 tests comprise 118 primary-library unit tests, 68 binary application and
UI tests, one 19-case golden-corpus test, three generated property suites with
512 fixed-seed cases each, and 18 platform-crate tests. The stricter trust-kernel
report excludes the immediate-mode `src/app.rs`, `src/main.rs`, and
`src/markdown_ui.rs` adapters. Those files remain inside the separate 80 percent
whole-workspace gate. The filesystem adapter measures 91.44 percent line
coverage, the Markdown UI adapter 92.39 percent, the native platform adapter
93.97 percent, and the full workspace 89.49 percent. Semantic UI automation and
manual accessibility gates remain required before v0.1.

The binary is 3,326,976 bytes larger than the M0 baseline. That measured cost
includes reachable BLAKE3 conflict fingerprints, native metadata transfer,
commit reconciliation, stable-handle loading, revision-aware saves, current
text shaping and rasterization, the bundled variable document font, persisted
themes, and the early native Markdown surface. It remains below the 12 MiB
first-release ceiling. The full locked
cross-target graph now contains 413 packages and still requires the release
duplicate, license, source, and capability audit.

### Focused UI mutation commands

The lifecycle result was produced from the settled `src/app.rs` scope with:

```powershell
$mutationRoot = Join-Path ([System.IO.Path]::GetTempPath()) 'noter-mutation-evidence'
$env:CARGO_TARGET_DIR = Join-Path $mutationRoot 'target'
$env:CARGO_MUTANTS_OUTPUT = Join-Path $mutationRoot 'lifecycle'
$env:CARGO_INCREMENTAL='0'
cargo mutants --in-place --no-config --workspace --all-features --colors never --minimum-test-timeout 60 -f src/app.rs --re 'NoterApp::(request_open|request_new_document|request_close|protect_native_close|begin_pending_abandon|cancel_pending_abandon|discard_pending_abandon|save_pending_abandon|continue_pending_abandon_if_clean|execute_abandon_action|restore_save_recovery_message|do_save|do_save_as_to|handle_save_result)\b'
```

The final Markdown result used cargo-mutants' isolated source copy:

```powershell
$mutationRoot = Join-Path ([System.IO.Path]::GetTempPath()) 'noter-mutation-evidence'
$env:CARGO_TARGET_DIR = Join-Path $mutationRoot 'target'
$env:CARGO_MUTANTS_OUTPUT = Join-Path $mutationRoot 'markdown'
$env:CARGO_INCREMENTAL='0'
cargo mutants --no-config --workspace --all-features --colors never --minimum-test-timeout 60 -f src/markdown_ui.rs --re '(markdown_edit_layout|markdown_render_layout|markdown_source_styles|semantic_target_at_selection|apply_markdown_tag|tag_hides_source_markup|reveal_event_text|markdown_text_format|formatted_block_marker|is_quote_marker|is_block_quote|insert_link)'
```

Both reports pass `scripts/check_mutation_infrastructure.py`. The eight Markdown
unviable results are compiler rejections, not linker, storage, process, timeout,
or tool failures.

The complete mutation campaign is recorded in
[M1_MUTATION_EVIDENCE.md](M1_MUTATION_EVIDENCE.md). The next evidence update must
attach manual platform metadata fixtures, weaker-filesystem observations, and
reproducible latency and memory measurements. Until then M1 remains In Progress.
