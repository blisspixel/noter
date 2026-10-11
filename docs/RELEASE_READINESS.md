# Beta.2 Hardening Readiness

**Updated:** 2026-10-11

**Decision:** focused beta.2 prerelease approved on 2026-10-11. Publication is
pending. The published release remains beta.1 until a verified replacement is
available. First-release requirements remain in [REQUIREMENTS.md](REQUIREMENTS.md)
and sequence remains in [ROADMAP.md](ROADMAP.md).

## Scope and issue audit

| Issue | Implementation | Verification evidence |
| --- | --- | --- |
| [#44](https://github.com/blisspixel/noter/issues/44), hidden Wayland window stalls | Disable glow VSync in Wayland sessions; suppress hidden-window periodic inspection | Configuration and visibility regressions pass; native Hyprland retest postponed, issue remains open |
| [#45](https://github.com/blisspixel/noter/issues/45), menu shortcut spacing | Shared minimum gap across File, Edit, View, and Markdown menus | Rendered geometry regression; visual review at 100 and 150 percent scale |
| [#46](https://github.com/blisspixel/noter/issues/46), CRT strong emphasis | Distinct high-intensity strong text in both CRT themes | Layout/color/contrast tests; regenerated native captures; native theme retest |
| [#47](https://github.com/blisspixel/noter/issues/47), Linux launcher and Open With | Desktop entry, icon, source-root documentation; selected-file argument and exact Exec escaping; install-root ownership on uninstall | Source/binary installer regressions; real GIO selected-file launch; native launcher retest |

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
not automatic verification of the new candidate.

| Revision | Evidence | Result |
| --- | --- | --- |
| `cc32abe` | [main CI 36460432091](https://github.com/blisspixel/noter/actions/runs/36460432091) | Native suites, coverage, and mutation matrix passed after recovery and installer consolidation |
| `0d8e6de` | [main CI 38038248915](https://github.com/blisspixel/noter/actions/runs/38038248915) | Linux desktop/session integration passed |
| `9879408` | [PR CI 38069044066](https://github.com/blisspixel/noter/actions/runs/38069044066) | UI/compositor changes passed all jobs; trust-kernel line coverage 93.05 percent |
| Beta.2 candidate | Exact-head PR and main-push CI | Pending; both 80 percent workspace and 90 percent trust-kernel gates remain mandatory |
| Beta.2 candidate | Hosted Release dry run | Pending; four target archives, MSI, notices, checksums, SBOMs and manifest must agree |

The `ed67c26` hosted Linux LCOV receipt contains 41,529/44,372 covered workspace
lines (93.59 percent), and 19,786/21,284 trust-kernel lines (92.96 percent) using
the exact CI exclusion expression. Both exceed their unchanged gates. Final-head
CI must independently pass; this receipt does not replace that requirement.

Local Windows validation of the beta.2 candidate source passed formatting,
strict Clippy, rustdoc, advisory audit, dependency policy, 1,057 Rust tests,
226 Python tests (28 platform-specific skips), Ruff, links, screenshots, and
release configuration. The same working tree passes 26 Linux WSL Ubuntu 20.04 installer tests
(two macOS-specific skips), including a real GLib DesktopAppInfo launch through
the generated entry. These are native Linux process observations,
not an interactive Omarchy desktop result. Final candidate results replace this
provisional local record after review and version preparation.

The first hosted dry run, [38132418647](https://github.com/blisspixel/noter/actions/runs/38132418647),
built all four targets but correctly refused global assembly because the artifact
validator still expected beta.1. Updating its identity to beta.2 makes the same
downloaded local artifacts pass `prepare --stage global`. A new Cargo-version
consistency regression prevents that drift. Independent review approved the
validator's new digest without changing its payload or publication safeguards.
A complete hosted dry run is still required.

## Focused native matrix

Two independent final-diff reviews resolved the launcher defects and approved
the installer, version/MSI changes, and release-note workflow. Both reviews
scored correctness, data safety, privacy/security, performance, tests,
maintainability, and documentation 4/5 with the native process tests and stated
limitations as evidence. Workflow/WiX digest ratification followed that review.
All five candidate screenshots were regenerated and visually inspected; image
bytes match the prior reviewed captures, and the input digest binds beta.2's
updated manifests and lockfile. This records source review and visual inspection,
not completion of the pending native desktop or publication rows.

Use [PLAYTEST.md](PLAYTEST.md) for exact actions and record the full commit,
artifact checksum, platform, desktop, and display scale beside each result.

| Check | Windows | Omarchy/Hyprland | macOS/X11 |
| --- | --- | --- | --- |
| Candidate launch, edit, save, failed-save retention, recovery | Passed on `ed67c26`; [native record](evidence/beta2-windows-2026-10-11.md) | Interactive retest deferred to beta.3 | Automated tests required; interactive matrix deferred to RC |
| Hidden workspace, clean and dirty, recovery scheduling | Not applicable | Postponed; required for #44 verification, not claimed for beta.2 | Native compositor matrix deferred to beta.3 |
| Menus and CRT emphasis at 100 and 150 percent scale | Native menu and both CRT themes passed at observed host scale; five deterministic 100 percent captures reviewed; full scale matrix deferred to RC | Interactive retest deferred to beta.3 | Interactive theme matrix deferred to RC |
| Source/binary launcher, Open With, uninstall ownership | Not applicable | Native desktop retest deferred to beta.3; Linux process regressions pass locally | Linux X11 desktop retest deferred to beta.3 |
| Downloaded archive version, checksum and attestation | Pending | Pending | Four-target artifact inspection required; clean-machine matrix deferred to RC |
| Install, reinstall, upgrade, rollback, uninstall | Pending source fixture | Automated installer fixtures required | Complete packaged clean-machine matrix deferred to RC |

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
4. Complete M6 diagnostics/formatting and M7 native signing, clean-machine
   packaging, platform matrix, and RC dogfood. Keep optional spell checking and
   automation rewrites behind these user-visible correctness priorities.
