# 0.1.0-alpha.2 Correctness Matrix

**Recorded:** 2026-08-05

**Subject commit:** `85bf83d6a36b2a160a6675a428c9dfe78121d11f`

**Exact-head CI:** [31030331218](https://github.com/blisspixel/noter/actions/runs/31030331218)
(`headSha` matches the subject commit; conclusion success)

**Status:** Partial dogfood gate record. This is **not** a `0.1.0-alpha.2`
label claim and is **not** a full release-candidate matrix.

## Purpose

The full release matrix lives in
[manual-test-matrix.md](manual-test-matrix.md). That template covers IME, real
screen readers, packaging soak, and multi-week dogfood that belong to later
checkpoints. This record scores only the **correctness-alpha** rows needed to
judge whether kill-process recovery, conflict overwrite, clipboard and
navigation, and available trust evidence are strong enough for careful dogfood
with backups.

Rows use:

| Result | Meaning |
| --- | --- |
| Pass | Observed or automated proof named below |
| Partial | Important path proven; a residual interactive or environmental gap remains |
| Blocked | Environment or interactive session unavailable this run |
| N/A | Outside alpha.2 dogfood scope (see reason) |

## Evidence header

| Field | Value |
| --- | --- |
| Noter commit | `85bf83d6a36b2a160a6675a428c9dfe78121d11f` |
| Tree | Clean `main` matching `origin/main` at record time |
| Build profile | Locked workspace debug tests; release source install in M2 record |
| Tester | Automated maintainer session on the development host |
| Date | 2026-08-05 |
| Operating system | Windows 11 Pro, build 26200 |
| Rust / Cargo | 1.97.1 |
| Filesystem | Healthy fixed NTFS (local) |
| Relevant automated evidence | Exact-head CI `31030331218`; [M1_FILESYSTEM_EVIDENCE.md](M1_FILESYSTEM_EVIDENCE.md); [M2_INSTALLED_EVIDENCE.md](M2_INSTALLED_EVIDENCE.md); [M3_EDITING_EVIDENCE.md](M3_EDITING_EVIDENCE.md) |

## Automated baseline at subject commit

Command:

```text
cargo test --locked --workspace --all-features
```

| Suite | Result |
| --- | ---: |
| `noter` lib unit tests | 236 passed |
| `noter` binary unit tests | 304 passed |
| Integration and property tests | 15 passed |
| `noter_platform` unit tests | 26 passed |
| **Total** | **581 passed, 0 failed** |

Hosted exact-head CI on Windows, Ubuntu, and macOS also passed fmt/clippy, docs,
install-from-source steps, coverage gates (Linux), and the supported-platform
mutation union for this commit.

## Alpha.2 critical rows

### Lifecycle and close safety

| ID | Result | Evidence |
| --- | --- | --- |
| LIF-03..LIF-06 dirty New/Open/Close/Quit decisions | Pass | Pure `LifecycleState` exhaustive unit tests and 512-case property (`core::lifecycle`, `lifecycle_properties`) |
| LIF-07 failed Save keeps dirty document | Pass | Save continuation truth table never discards dirty work |
| LIF-10 indeterminate-save block | Pass | App recovery-ledger bounds and save-block unit paths |

Native dialog chrome and window-manager Close chrome remain manual residual for
RC (not scored here as alpha.2 blockers when the pure decision core is green).

### Open, Save, and byte fidelity (available platforms)

| ID | Result | Evidence |
| --- | --- | --- |
| IO-04..IO-09 empty/BOM/EOL round-trips | Pass | Document fixtures and property tests; golden matrix |
| IO-12 metadata policy (Windows NTFS subset) | Partial | [M1_FILESYSTEM_EVIDENCE.md](M1_FILESYSTEM_EVIDENCE.md) native NTFS and WSL2 ext4 |
| IO-13 refuse final reparse/symlink | Pass | File-observation and platform unit truth tables |
| IO-14 cloud/network/removable limits | Blocked | No SMB, cloud write, removable, or weak FS available; gaps named in M1 record |
| IO-15..IO-20 full platform metadata matrix | Partial | Windows private DACL and NTFS replacement covered; macOS native and full Linux metadata fixtures remain open in M1 |

### Recovery

| ID | Result | Evidence |
| --- | --- | --- |
| REC-02 idle-debounce persist then restart offer | Pass | `crash_recovery::tests::dirty_edit_persists_after_idle_debounce` + `startup_scan_offers_valid_record` |
| REC-04 newest valid offer | Pass | Startup scan offer ordering and validation unit tests |
| REC-05 restore opens dirty without writing original | Pass | `restore_active_offer` keeps dirty document; pure recovery never rewrites the user path |
| REC-06 Save and Discard delete owned record | Pass | `save_clean_deletes_owned_record`, `discard_offer_deletes_record` |
| REC-08 corrupt/truncated/checksum quarantine | Pass | `core::recovery` and `core::recovery_store` quarantine campaigns |
| REC-09 distinct instances | Pass | `fresh_identity_keeps_distinct_recovery_instances` |
| REC-10 persist failure visible | Pass | Persist-failure message and app unit paths; pure epoch-matched failure ack tests |
| REC-01 force-kill before idle debounce | Partial | Scheduling proves no persist is required before the 2 s idle / 15 s max policy fires; live GUI force-kill timing not executed this session |
| REC-03 force-kill during replacement | Blocked | Destructive concurrent-write scenario not executed this session |

### External changes

| ID | Result | Evidence |
| --- | --- | --- |
| CON-01..CON-03 classify external change | Pass | Pure conflict classifier unit truth tables |
| CON-04 Reload guarded when dirty | Pass | Lifecycle + conflict integration unit paths |
| CON-05 Keep Editing never rebaselines | Pass | `conflict_state_prompts_once_and_keep_editing_does_not_authorize_reload` |
| CON-07 overwrite second confirmation | Pass | `overwrite_requires_a_second_confirmation` |
| CON-08 conflict during Save | Pass | `core::save` conflict tests refuse overwrite of external version |

### Editing, clipboard, navigation

| ID | Result | Evidence |
| --- | --- | --- |
| EDT-01 word / line-home / document movement | Pass | `core::navigation` unit suite; `keyboard_nav` platform policy unit suite; Text Mode and Markdown active-block adapter integration tests |
| EDT-02 Shift extend | Pass | `extend_selection` pure tests; Markdown `Shift+End` integration test |
| EDT-04 Cut / Paste shared path | Pass | `cut_command_removes_selection_through_the_shared_edit_path`; paste origin and bounds tests |
| EDT-05..EDT-08 undo/coalesce | Pass | History unit tests, long-session bound fixture, typing coalesce vs paste separation |
| EDT-09..EDT-11 find/replace/go-to-line | Pass | Search property and app find-navigation unit tests |
| EDT-03 mouse drag selection | Partial | Markdown cross-block pointer drag unit suite; Text Mode double/triple click still platform-widget residual |
| EDT-12 word wrap | Pass | Text wrap preference unit and UI paths without byte change |

### Privacy (alpha.2 slice)

| ID | Result | Evidence |
| --- | --- | --- |
| SEC-01 no unexpected network | Pass | Architecture and privacy contract: no background network; update is explicit |
| SEC-03 recovery permissions | Partial | Private recovery store owner-restricted siblings; platform private-file unit tests on Windows |
| SEC-05 Markdown remote content | Pass | No remote fetch in Markdown projection path; documented product boundary |

### Rows outside alpha.2 dogfood scope

| Area | Result | Reason |
| --- | --- | --- |
| TXT-01..TXT-04 real IME | N/A | M5 feasibility gate |
| A11Y screen readers (NVDA/VoiceOver/Orca) | N/A | M5 / RC |
| PERF full benchmark re-run | N/A | Existing M1 baseline remains the reference; not re-executed this session |
| REL packaging soak and 14-day dogfood | N/A | RC / 0.1.0 |
| IO cloud write / second identity / power-loss | Blocked | Environment unavailable; does not invent a pass |

## Cross-platform automated posture

| Platform | Automated tests | Mutation | Source install step |
| --- | --- | --- | --- |
| Windows | Pass (hosted + local) | Pass | Pass (CI PowerShell + M2 disposable install) |
| Linux | Pass (hosted) | Pass | Pass (CI POSIX) |
| macOS | Pass (hosted) | Pass | Pass (CI POSIX) |

Interactive keyboard, recovery kill timing, and theme GUI relaunch were not
re-run by a human on each hosted OS this session. Hosted CI proves the locked
suite and mutation gates, not the full manual matrix.

## Dogfood readiness judgment

**Ready for careful local dogfood with backups** on Windows for ordinary
`.txt` / `.md` notes, provided the operator keeps external backups and
understands remaining M1 environment gaps.

**Not ready to claim the `0.1.0-alpha.2` version label** until at least:

1. One interactive Windows force-kill recovery pass covers REC-01 and REC-02
   against a live GUI process (method, timing, and recovery directory recorded).
2. One non-Windows interactive smoke of launch, type, save, and word/Home-End
   navigation is recorded, or is explicitly deferred with a signed product
   decision that hosted CI alone is enough for that slice.
3. This matrix and the recovery path land on one immutable green commit that is
   then version-bumped and tagged together.

## Commands to re-verify

```text
git rev-parse HEAD
cargo test --locked --workspace --all-features
python scripts/check_doc_links.py
python scripts/check_readme_assets.py
```

Exact-head hosted CI must remain green for the labeled commit before any
version tag.
