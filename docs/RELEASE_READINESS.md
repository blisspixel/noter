# Beta.2 Hardening Readiness

**Updated:** 2026-10-13

**Published checkpoint:** focused beta.2 prerelease, approved and published on
2026-10-11 from `9cdaa3b8cef9a85259236302654cbf40c34e7339`.
The [publication record](evidence/beta2-publication-2026-10-11.md) binds the
[release](https://github.com/blisspixel/noter/releases/tag/v0.1.0-beta.2), passing
CI, public assets, provenance, and installed version to that source. This record
retains preparation history and deferred checks. The later scoped publication is
[0.1.0-beta.2.1](evidence/beta2.1-publication-2026-10-13.md).
Source version `0.1.0-beta.2.2` is the next scoped candidate. It is not a
publication record until the release workflow binds its tag. It does not close
issues #44, #50, #51, #52, #53, #54, #59, or #60. First-release
requirements remain in [REQUIREMENTS.md](REQUIREMENTS.md) and sequence in
[ROADMAP.md](ROADMAP.md).

## Scope and issue audit

| Issue | Implementation | Verification evidence |
| --- | --- | --- |
| [#44](https://github.com/blisspixel/noter/issues/44), hidden Wayland window stalls | Disable glow VSync in Wayland sessions; suppress hidden-window periodic inspection | Configuration and visibility regressions pass; native Hyprland retest postponed, issue remains open |
| [#45](https://github.com/blisspixel/noter/issues/45), menu shortcut spacing | Shared minimum gap across File, Edit, View, and Markdown menus | Rendered geometry regression; native visual review at observed host scale; full display-scale matrix deferred to RC |
| [#46](https://github.com/blisspixel/noter/issues/46), CRT strong emphasis | Distinct high-intensity strong text in both CRT themes | Layout/color/contrast tests; regenerated native captures; native theme retest |
| [#47](https://github.com/blisspixel/noter/issues/47), Linux launcher and Open With | Desktop entry, icon, source-root documentation; selected-file argument and exact Exec escaping; install-root ownership on uninstall | Source/binary installer regressions; real GIO selected-file launch; interactive launcher retest deferred to beta.3 |

The review also corrected unsupported README sticky-formatting claims, roadmap
claims about signed archives and selectable GUI/TUI dependencies, stale alpha
playtest instructions, and expired release-note deferrals. A validator regression
rejects recurrence of the stale checkpoint text.

## Research and implementation constraints

- The [Wayland surface frame contract](https://wayland.freedesktop.org/docs/html/apa.html#protocol-spec-wl_surface-request-frame)
  permits withholding callbacks for surfaces not visible. Disabling blocking
  swaps is a mitigation consistent with that contract; configuration tests do
  not establish resolution on Hyprland. Recovery scheduling stays active.
- [egui 0.35 Button](https://docs.rs/egui/0.35.0/egui/widgets/struct.Button.html#method.gap)
  provides the existing spacing seam. No custom menu renderer is needed.
- The [Desktop Entry Exec contract](https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html)
  requires separate string and argument escaping plus an explicit file field.
  Linux entries use `/usr/bin/env --` to allow literal percent signs in the
  editor path without GLib treating the unexpanded executable as nonexistent.
  This executes the editor directly after argument expansion, without a shell.
  Executable paths containing control characters or `=` are refused with an
  install-root diagnostic before launcher writes; selected file names containing
  `=` remain supported.

## Automated evidence

Historical implementation references below are evidence for their exact commits,
not automatic verification of later revisions.

| Revision | Evidence | Result |
| --- | --- | --- |
| `cc32abe` | [main CI 36460432091](https://github.com/blisspixel/noter/actions/runs/36460432091) | Native suites, coverage, and mutation matrix passed after recovery and installer consolidation |
| `0d8e6de` | [main CI 38038248915](https://github.com/blisspixel/noter/actions/runs/38038248915) | Linux desktop/session integration passed |
| `9879408` | [PR CI 38069044066](https://github.com/blisspixel/noter/actions/runs/38069044066) | UI/compositor changes passed all jobs; trust-kernel line coverage 93.05 percent |
| `9cfd712` | [PR CI 38133597002](https://github.com/blisspixel/noter/actions/runs/38133597002) | Cancelled after documentation correction; ordinary suites, coverage, Linux/macOS mutation, and Windows shard 9 passed; remaining Windows shards were interrupted |
| `9cfd712` | [Hosted Release dry run 38133594593](https://github.com/blisspixel/noter/actions/runs/38133594593) | Four target builds and global assembly passed; downloaded 20-file payload passed host-stage validation and archive inspection; hosting was skipped |
| `a73dc38` | [PR CI 38137457512](https://github.com/blisspixel/noter/actions/runs/38137457512) | Cancelled after the consumer-document correction; ordinary native suites, lint, docs, coverage, and macOS mutation passed before cancellation; this is historical evidence only |
| `a73dc38` | [Hosted dry run 38137465542](https://github.com/blisspixel/noter/actions/runs/38137465542) and [inspection receipt](https://github.com/blisspixel/noter/pull/49#issuecomment-6108874848) | Four target builds, global assembly, 20-file host-stage validation, archives, bundled docs/notices, Windows version/MSI properties, and SBOM identities passed; hosting was skipped |
| `05cbb69` | [PR CI 38139971077](https://github.com/blisspixel/noter/actions/runs/38139971077) | Every job passed, including all 14 mutation workers and aggregate gate |
| `05cbb69` | [Hosted dry run 38139972098](https://github.com/blisspixel/noter/actions/runs/38139972098) and [inspection receipt](https://github.com/blisspixel/noter/pull/49#issuecomment-6109188997) | Four builds, global assembly, exact 20-file payload, archives, source, MSI, SBOMs, bundled documents/notices, and Windows version passed; host skipped |
| `9cdaa3b` | [Main-push CI 38144668329](https://github.com/blisspixel/noter/actions/runs/38144668329) | Every required job passed before publication; protected rebase preserved the reviewed candidate's complete tree |
| `9cdaa3b` | [Release 38149804354](https://github.com/blisspixel/noter/actions/runs/38149804354) and [public verification](evidence/beta2-publication-2026-10-11.md) | Published beta.2; exact 21-file public inventory, server digests, checksums, manifest, source/archive/MSI/SBOM inspection, all attestations, and isolated Windows install/reinstall passed |

The release source `9cdaa3b` hosted Linux LCOV receipt contains 41,537/44,372
covered workspace lines (93.61 percent), and 19,794/21,284 trust-kernel lines
(93.00 percent) using the exact CI exclusion expression. Both exceed their
unchanged gates. Downloaded PR and main mutation reports contain zero missed,
timed-out, or surviving mutants, all baselines pass, and the infrastructure
checker reports no recognized failures. Per-shard counts and artifact hashes
are in the [JSON receipt](evidence/beta2-publication-2026-10-11.json).

Local Windows validation of the `9cfd712` source passed formatting,
strict Clippy, rustdoc, advisory audit, dependency policy, 1,057 Rust tests,
228 Python tests (28 platform-specific skips), Ruff, links, screenshots, and
release configuration. The same working tree passes 26 Linux WSL Ubuntu 20.04 installer tests
(two macOS-specific skips), including a real GLib DesktopAppInfo launch through
the generated entry. These are native Linux process observations,
not an interactive Omarchy desktop result. This historical local record is
supplemented by the final exact-commit hosted and public verification above.

The consumer-guide correction adds version-matched installer recipes and a
configuration regression that rejects source/guide version drift, old-version
recipes, and unversioned installer downloads. Its local script gate passed
237 Python tests (28 platform-specific skips), Ruff, links, screenshots, and
release configuration. Rust implementation and publication safeguards are
unchanged. Its exact-head PR CI and the rebased main-push CI both passed.

The first hosted dry run, [38132418647](https://github.com/blisspixel/noter/actions/runs/38132418647),
built all four targets but correctly refused global assembly because the artifact
validator still expected beta.1. Updating its identity to beta.2 makes the same
downloaded local artifacts pass `prepare --stage global`. A new Cargo-version
consistency regression prevents that drift. Independent review approved the
validator's new digest without changing its payload or publication safeguards.
The successful replacement dry runs above confirmed the corrected identity
through global assembly. Actual public upload and attestation verification were
subsequently established by the named publication record.

## Focused native matrix

Two independent final-diff reviews resolved the launcher defects and approved
the installer, version/MSI changes, and release-note workflow. Both reviews
scored correctness, data safety, privacy/security, performance, tests,
maintainability, and documentation 4/5 with the native process tests and stated
limitations as evidence. Workflow/WiX digest ratification followed that review.
All five candidate screenshots were regenerated and visually inspected; image
bytes match the prior reviewed captures, and the input digest binds beta.2's
updated manifests and lockfile. This records source review and visual inspection,
not completion of the deferred native desktop, display-scale, or clean-machine
installation rows. Public artifact verification is recorded separately above.

Use [PLAYTEST.md](PLAYTEST.md) for exact actions and record the full commit,
artifact checksum, platform, desktop, and display scale beside each result.

| Check | Windows | Omarchy/Hyprland | macOS/X11 |
| --- | --- | --- | --- |
| Candidate launch, edit, save, failed-save retention, recovery | Passed on `ed67c26`; [native record](evidence/beta2-windows-2026-10-11.md) | Interactive retest deferred to beta.3 | Automated tests required; interactive matrix deferred to RC |
| Hidden workspace, clean and dirty, recovery scheduling | Not applicable | Postponed; required for #44 verification, not claimed for beta.2 | Native compositor matrix deferred to beta.3 |
| Menus and CRT emphasis at 100 and 150 percent scale | Native menu and both CRT themes passed at observed host scale; five deterministic 100 percent captures reviewed; full scale matrix deferred to RC | Interactive retest deferred to beta.3 | Interactive theme matrix deferred to RC |
| Source/binary launcher, Open With, uninstall ownership | Not applicable | Native desktop retest deferred to beta.3; Linux process regressions pass locally | Linux X11 desktop retest deferred to beta.3 |
| Downloaded archive version, checksum and attestation | Public archive CLI prints beta.2; checksum/provenance and isolated install/reinstall passed | Linux archive checksum/provenance passed; native Omarchy launch still deferred | Both macOS archives passed checksum/provenance, bundled-document, and SBOM checks; native interactive launch and clean-machine matrix deferred to RC |
| Install, reinstall, upgrade, rollback, uninstall | Automated installer fixtures passed; clean-machine packaged matrix deferred to RC | Linux source/binary installer process fixtures passed; interactive desktop matrix deferred to beta.3 | Native macOS installer fixtures passed in CI; complete packaged clean-machine matrix deferred to RC |

The 2026-10-11 follow-up postponed the user's Omarchy retest and directed continued
repository work. This scoped beta carries the implementation with the limitations
above, without claiming Hyprland resolution or a complete interactive platform
matrix. These deferred observations remain later checkpoint gates. Existing
automated safety and release requirements remain mandatory.

## Publication checklist

- Version, lockfile, monotonic MSI sequence, changelog, screenshots, validators,
  and prerelease notes agree on beta.2.
- Two independent reviews cover the final diff and security-sensitive installer
  and release changes; all findings are resolved or explicitly blocked.
- Exact-head PR CI passes before protected-main integration. Exact main-push CI
  passes before publication, including audit, deny, documentation, both coverage
  gates, and every native mutation shard.
- The focused required native rows above pass. Every deferred environment stays
  outside any new support claim. No known critical or high safety defect remains.
- The hosted dry run produces the exact reviewed inventory. Inspect downloaded
  artifacts; preserve candidate identity and hashes in the publication receipt.
- Publish through the existing exact-tip release workflow, then verify the tag,
  assets, checksums, SBOMs, attestations, installed version, and release readback.

## Work after beta.2

1. [Complete M4-H1's adversarial native storage matrix](https://github.com/blisspixel/noter/issues/50) before broadening recovery
   support: redirected and synchronized roots, weak/remote storage, final-window
   swaps, barrier faults, and second-identity access on each applicable platform.
2. [Run M5's bounded feasibility gate](https://github.com/blisspixel/noter/issues/51) on one immutable corpus and commit. Prove
   typography, bidi, IME, accessible editable text, hit testing, and the 50 MiB
   path before changing the current 8 MiB UI limit or renderer.
3. [Build continuous source-backed Markdown](https://github.com/blisspixel/noter/issues/52), sticky formatting, and caret-aware
   delimiter reveal on the accepted editor foundation. Preserve exact bytes,
   shared transactions, Undo, conflict, and recovery behavior.
4. Complete [M6 diagnostics/formatting](https://github.com/blisspixel/noter/issues/54) and [M7 native release evidence](https://github.com/blisspixel/noter/issues/53): signing, clean-machine
   packaging, platform matrix, and RC dogfood. Keep optional spell checking and
   automation rewrites behind these user-visible correctness priorities.
