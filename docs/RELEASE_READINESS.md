# Beta.2 Hardening Readiness

**Updated:** 2026-10-11

**Decision:** focused beta.2 prerelease approved on 2026-10-11. Publication is
pending. The published release remains beta.1 until a verified replacement is
available. First-release requirements remain in [REQUIREMENTS.md](REQUIREMENTS.md)
and sequence remains in [ROADMAP.md](ROADMAP.md).

## Scope and issue audit

| Issue | Implementation | Required evidence |
| --- | --- | --- |
| [#44](https://github.com/blisspixel/noter/issues/44), hidden Wayland window stalls | Disable glow VSync in Wayland sessions; suppress hidden-window periodic inspection | Configuration and visibility regressions; native Hyprland workspace retest |
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

Local Windows validation of the beta.2 candidate source passed formatting,
strict Clippy, rustdoc, advisory audit, dependency policy, 1,057 Rust tests,
226 Python tests (28 platform-specific skips), Ruff, links, screenshots, and
release configuration. The same working tree passes 26 Linux WSL Ubuntu 20.04 installer tests
(two macOS-specific skips), including a real GLib DesktopAppInfo launch through
the generated entry. These are native Linux process observations,
not an interactive Omarchy desktop result. Final candidate results replace this
provisional local record after review and version preparation.

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
| Candidate launch, edit, save, failed-save retention, recovery | Pending | Pending | Automated tests required; interactive matrix deferred to RC |
| Hidden workspace, clean and dirty, recovery scheduling | Not applicable | Pending, required for #44 verification | Native compositor matrix deferred to beta.3 |
| Menus and CRT emphasis at 100 and 150 percent scale | Pending captures/review | Pending | Interactive theme matrix deferred to RC |
| Source/binary launcher, Open With, uninstall ownership | Not applicable | Pending native desktop; Linux process regressions pass locally | Linux X11 desktop retest deferred to beta.3 |
| Downloaded archive version, checksum and attestation | Pending | Pending | Four-target artifact inspection required; clean-machine matrix deferred to RC |
| Install, reinstall, upgrade, rollback, uninstall | Pending source fixture | Automated installer fixtures required | Complete packaged clean-machine matrix deferred to RC |

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

1. Complete M4-H1's adversarial native storage matrix before broadening recovery
   support: redirected and synchronized roots, weak/remote storage, final-window
   swaps, barrier faults, and second-identity access on each applicable platform.
2. Run M5's bounded feasibility gate on one immutable corpus and commit. Prove
   typography, bidi, IME, accessible editable text, hit testing, and the 50 MiB
   path before changing the current 8 MiB UI limit or renderer.
3. Build continuous source-backed Markdown, sticky formatting, and caret-aware
   delimiter reveal on the accepted editor foundation. Preserve exact bytes,
   shared transactions, Undo, conflict, and recovery behavior.
4. Complete M6 diagnostics/formatting and M7 native signing, clean-machine
   packaging, platform matrix, and RC dogfood. Keep optional spell checking and
   automation rewrites behind these user-visible correctness priorities.
