# 0.1.0-alpha.2 Correctness Matrix

**Recorded:** 2026-08-23

**Implementation commit:** `27b9fe7f10dc01a0e4b7004c453ebde510db36d0`

**Exact-head CI:** Required on the final evidence commit before the tag is
created. GitHub branch protection and the Release workflow are the authoritative
external record because a workflow run cannot cite the commit that adds its own
run identifier.

**Status:** Release gate record for the scoped `0.1.0-alpha.2` prerelease. This
is not the full release-candidate or stable-release matrix.

## Purpose

The full release matrix lives in
[manual-test-matrix.md](manual-test-matrix.md). That template covers real IME,
screen readers, packaging soak, and multi-week dogfood that belong to later
checkpoints. This record scores the correctness-alpha rows used to decide
whether recovery, conflict handling, editing bounds, clipboard and navigation,
and the available trust evidence are strong enough for careful prerelease
dogfood with backups.

Rows use:

| Result | Meaning |
| --- | --- |
| Pass | Observed or automated proof named below |
| Partial | Important path proven; a residual interactive or environmental gap remains |
| Blocked | Environment or interactive session unavailable this run |
| N/A | Outside alpha.2 prerelease scope, with the reason recorded |

## Evidence header

| Field | Value |
| --- | --- |
| Noter implementation commit | `27b9fe7f10dc01a0e4b7004c453ebde510db36d0` |
| Tree | Feature implementation frozen at the named commit; this evidence document is its descendant |
| Build profiles | Locked workspace tests and release all-features Windows GUI |
| Tester | Automated maintainer session with live native UI inspection on the development host |
| Date | 2026-08-23 |
| Operating system | Windows 11 Pro, build 26200 |
| Rust / Cargo | 1.97.1 / 1.97.1 |
| Filesystem | Healthy fixed NTFS, local |
| Relevant prior evidence | [M1_FILESYSTEM_EVIDENCE.md](M1_FILESYSTEM_EVIDENCE.md); [M2_INSTALLED_EVIDENCE.md](M2_INSTALLED_EVIDENCE.md); [M3_EDITING_EVIDENCE.md](M3_EDITING_EVIDENCE.md) |

## Automated baseline

Commands and results at the implementation commit:

| Gate | Result |
| --- | --- |
| `cargo test --locked --workspace --all-targets --all-features` | 812 passed, 0 failed |
| Python unittest discovery | 172 passed, 4 Windows-host environment skips |
| Whole-workspace line coverage | Prior exact hosted descendant: 94.18 percent, 34,353 of 36,474 lines; final exact-head rerun required |
| Trust-kernel line coverage | Prior exact hosted descendant: 93.72 percent, 14,457 of 15,425 lines; final exact-head rerun required |
| Formatting and strict Clippy | Pass |
| Rustdoc with warnings denied | Pass |
| RustSec audit | Pass, no advisory warnings |
| Dependency license and source policy | Pass |
| Focused mutation-gap reruns | Pass; the final line-ending, first-ending, recovery-store, and stable live-lease binding campaigns caught every viable mutant with zero missed or timed-out results. The corrected split-CRLF boundary rerun caught 15 and rejected 2 at compile time; the real live-lease fact rerun caught all 5. Platform-specific helper names keep cfg-inactive Unix and Windows candidates on their executable host without narrowing either host's owned scope. Three platform race match guards remain narrowly excluded because portable tests cannot force them without mocking the native install or rename path, while their adjacent success and failure surfaces are covered. |
| Documentation links and release configuration | Pass |
| Light, Dark, and specialty-theme screenshot validation | Pass, regenerated from exact source; all five outputs remained byte-identical to the visually reviewed set; approved input digest `8ce752054adf0fc8a58339c3c7dedb13522f104d6b47c6d4d26e5d8d6f39a22f` |
| `dist plan --tag v0.1.0-alpha.2` | Pass, expected archive, installer, checksum, SBOM, and attestation inventory |

The 812 Rust tests comprise 312 library unit tests, 457 binary unit tests, 15
integration and property tests, and 28 native-platform unit tests.

## Alpha.2 critical rows

### Lifecycle and close safety

| ID | Result | Evidence |
| --- | --- | --- |
| LIF-03..LIF-06 dirty New/Open/Close/Quit decisions | Pass | Pure `LifecycleState` exhaustive unit tests and the 512-case lifecycle property test |
| LIF-07 failed Save keeps dirty document | Pass | Save continuation truth table never discards dirty work |
| LIF-10 indeterminate-save block | Pass | App recovery-ledger bounds and save-block unit paths |

Native dialog chrome and window-manager Close chrome remain manual residuals
for the later full release-candidate matrix.

### Open, Save, and byte fidelity

| ID | Result | Evidence |
| --- | --- | --- |
| IO-04..IO-09 empty/BOM/EOL round-trips | Pass | Document fixtures, properties, and golden matrix |
| IO-12 metadata policy, Windows NTFS subset | Partial | Prior native NTFS and WSL2 ext4 evidence remains current |
| IO-13 refuse final reparse/symlink | Pass | File-observation, native-platform, and command-line no-follow preflight fixtures, including nonblocking Unix FIFO rejection |
| IO-14 cloud/network/removable limits | Blocked | No SMB, cloud write, removable, or weak filesystem was available |
| IO-15..IO-20 platform metadata matrix | Partial | Windows private DACL and NTFS replacement are covered; native macOS and full Linux fixtures remain open |

### Recovery

| ID | Result | Evidence |
| --- | --- | --- |
| REC-01 force-kill before idle debounce | Partial | Scheduler bounds are tested; the live run killed only after a durable record appeared |
| REC-02 force-kill after idle debounce | Pass | Live release GUI run described below restored an exact intentionally empty dirty editor value |
| REC-03 force-kill during recovery replacement | Blocked | Destructive concurrent-write timing was not exercised interactively |
| REC-04 maximal valid offers | Pass | Bounded metadata-only startup scan coalesces authenticated schema-v2 instance revisions and proven causal successors while retaining legacy and incomparable branches |
| REC-05 restore opens dirty without writing original | Pass | Restore revalidates the exact claimed artifact, requires successor backup cleanup and parent durability, and treats later predecessor cleanup failure as non-fatal |
| REC-06 Save and Discard delete only owned records | Pass | FIFO persist fencing, keyed owned cleanup, dual fact-bound lease deletion, exact-handle deletion, path-replacement refusal, live-instance refusal, and distinct-branch regressions |
| REC-07 Cancel preserves recovery | Pass | Recovery-offer Cancel is inert and stale UI outcomes cannot delete newer records |
| REC-08 invalid records quarantine visibly | Pass | Corrupt, truncated, wrong-version, lineage, whole-record checksum, bounded-read, exact quarantine-copy, and quarantine-result campaigns |
| REC-09 distinct instances | Pass | Two independent locked paths survive one pathname rebind; lease failures fail closed, instances cannot claim one another's records, and a pathname/header identity disagreement remains untouched without an offer |
| REC-10 persistence and cleanup failure visible | Pass | Identity, lease, persist, and authorized-deletion failures retain the offer or surface a durable warning |
| REC-11 Undo and Redo recovery freshness | Pass | Persist C, Undo to dirty B, restart, and verify B bytes plus directional selection |
| REC-12 startup resource bounds | Pass | 1,024 raw-entry, 256 eligible-candidate, 128 MiB aggregate-read, 32-offer, 32-quarantine-result, and 16-superseded-handle bounds are surfaced; exact stale-live cleanup guarantees progress across bounded launches |

Live Windows method for REC-02:

1. Built the release all-features GUI and launched it with the test-only
   isolated state-directory override.
2. Opened a source fixture containing `alpha2 recovery probe`, located the real
   Noter window and `Document text editor` control through native Windows UI
   Automation, then invoked Edit > Select All and Edit > Cut. This made the
   authoritative document intentionally empty and dirty without synthetic
   keystrokes or clipboard injection.
3. Waited for one 252-byte recovery record and two independent live-lease files,
   then terminated the process without a graceful close.
4. Restarted against the same isolated state root and observed the Restore
   offer.
5. Selected Restore and verified the editor value was the exact empty dirty
   source, rather than the nonempty disk fixture.
6. Verified a distinct successor recovery record and exactly two held successor
   lease files existed after Restore.
7. Closed through the dirty-document prompt with explicit Discard Changes and
   verified zero recovery records and zero lease files remained.

The isolated root was under
`%LOCALAPPDATA%\Temp\noter-alpha2-e2e-...`. The harness validated that target
under the system temporary directory and removed it after observing zero record
and lease files, so it contains no remaining recovery payload.

### External changes

| ID | Result | Evidence |
| --- | --- | --- |
| CON-01..CON-03 classify external change | Pass | Pure conflict-classifier truth tables |
| CON-04 Reload guarded when dirty | Pass | Lifecycle and conflict integration paths |
| CON-05 Keep Editing never rebaselines | Pass | Exact successful disk evidence, changed-same-class, and uninspectable-state reducer regressions |
| CON-06 retained clean revision protected | Pass | External replacement immediately guards native Close, modified status, lifecycle, and recovery while ordinary Save still conflicts |
| CON-07 overwrite second confirmation | Pass | Explicit confirmation truth table |
| CON-08 conflict during Save | Pass | Save logic refuses overwrite of an externally changed version |

### Editing, clipboard, and navigation

| ID | Result | Evidence |
| --- | --- | --- |
| EDT-01 word, line-home, and document movement | Pass | Navigation core, platform policy, and both editor adapters |
| EDT-02 Shift extend | Pass | Pure selection tests and Markdown adapter integration |
| EDT-04 Cut and Paste shared path | Pass | Shared edit-path and paste-origin bounds tests |
| EDT-05..EDT-08 undo and coalescing | Pass | History tests, long-session bound fixture, and typing/paste separation |
| EDT-09..EDT-11 find, replace, and go-to-line | Pass | Search properties, empty-query Find navigation, ordered input, and app integration tests |
| EDT-03 mouse drag selection | Partial | Markdown cross-block pointer suite passes; Text Mode click behavior remains a native-widget residual |
| EDT-12 word wrap | Pass | Preference and UI paths preserve source bytes |
| View-command focus and bounds | Pass | Keyboard zoom preserves focused controls; bounded menu commands disable honestly at limits |
| Automated editor accessibility semantics | Pass | Text Mode exposes `Document text editor`; active Markdown source exposes `Markdown source editor`; semantic tests cover multiline editable roles, stable names, caret and selection projection, CR/LF/CRLF source mapping, and formatting-control state |
| Interactive-size ceiling | Pass | Typing, paste, IME, inline reopen, automatic Enter transforms, and whole-text replacement reject growth before mutation |
| Automatic Enter boundaries | Pass | Repeated Enter events are preserved, IME cancellation restores the canonical caret, code remains literal, marker-interior carets use ordinary editing, and LF/CRLF following lines remain separate |

### Privacy and security

| ID | Result | Evidence |
| --- | --- | --- |
| SEC-01 no unexpected network | Pass | No background network path; update status only links on explicit user action |
| SEC-03 recovery permissions | Pass | Exact corrected-head Windows, Linux, and macOS suites cover protected Windows DACL creation, Unix owner-only recovery siblings, macOS ACL absence, and fail-closed unsupported states |
| SEC-05 Markdown remote content | Pass | Restricted native model performs no remote fetch |
| Changed-code security review | Pass | Immutable full-range scan `8d960079-7db3-47eb-87f0-2b4fedc83845` reviewed the protected-base range through `cc7c8ca` with complete 28-file coverage. It reported one low-severity terminal-control path and suppressed two Unix recovery pathname races because they require full same-user control of the owner-private state directory, confer no new authority, and fail postcondition ratification. Commit `fd28af8` closes the finding at the complete-diagnostic stderr boundary. Descendant scans `37f04fa4-8385-4e1c-98f1-0151e839db1c` through `fd28af8` and `8e0162d9-f6c1-4140-84f9-19f90a026317` through final implementation `27b9fe7` both have complete coverage and zero findings. |

### Rows outside alpha.2 prerelease scope

| Area | Result | Reason |
| --- | --- | --- |
| TXT-01..TXT-04 real IME | N/A | M5 editor feasibility gate |
| NVDA, VoiceOver, and Orca | N/A | M5 and the full release-candidate matrix |
| PERF full benchmark re-run | N/A | Existing M1 baseline remains the reference; the termination harness deadline regression is unit-tested |
| REL packaging soak and 14-day dogfood | N/A | Full release candidate and 0.1.0 |
| IO cloud write, second identity, and power loss | Blocked | Required environment unavailable |

## Cross-platform posture

| Platform | Local or prior evidence | Required before alpha.2 tag |
| --- | --- | --- |
| Windows | Current local full suite and live recovery pass | Exact-head hosted suite and mutation shard |
| Linux | Prior hosted suite, mutation, and source-install step | Exact-head hosted suite, coverage, and mutation shard |
| macOS | Prior hosted suite, mutation, and source-install step | Exact-head hosted suite and mutation shard |

Interactive non-Windows GUI recovery is explicitly deferred to M5 because the
release host has no Linux or macOS desktop session. This deferral does not waive
the final exact-head Windows, Linux, and macOS CI jobs. A critical or high
security finding cannot be deferred for this prerelease.

## Prerelease judgment

The implementation is ready for a narrowly scoped `0.1.0-alpha.2` prerelease
for careful local dogfood with backups once all required checks succeed on the
final branch head and again on protected `main`. It is not evidence for an RC or
stable label.

The explicit 2026-08-22 alpha checkpoint accepts the named REC-01, REC-03,
non-Windows interactive GUI, filesystem-environment, IME, and accessibility
deferrals. The next engineering gate is M5 editor feasibility because large-file
bounds, IME correctness, accessibility semantics, and scale behavior determine
whether the current editor can safely support the remaining Markdown roadmap.

## Commands to re-verify

```text
git rev-parse HEAD
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --all-features --no-deps
cargo audit --deny warnings
cargo deny --locked check
python -m unittest discover -s scripts -p "test_*.py"
ruff check scripts
ruff format --check scripts
python scripts/check_doc_links.py
python scripts/check_readme_assets.py
python scripts/check_release_config.py
cargo dist plan --tag v0.1.0-alpha.2
cargo llvm-cov --locked --all-targets --all-features --workspace --fail-under-lines 80
cargo llvm-cov --locked --all-targets --all-features --workspace --ignore-filename-regex 'src[/\\](app|bounded_text_input|editor_settings|find_ui|go_to_line_ui|idle_screen|main|markdown_ui|theme)\.rs$' --fail-under-lines 90
```

The release workflow must run first as a dry run on the protected-main head.
Only that same successful main head may create `v0.1.0-alpha.2`.
