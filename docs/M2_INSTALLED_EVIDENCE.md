# M2 Installed-Product Evidence

**Observed:** 2026-08-05

**Exact commit:** `91ed8d7a05e460ef12f357eb112694a1870af0bd`

**Status:** Partial local M2 evidence. This is not M2 milestone sign-off.

## Scope and provenance

This record covers a disposable PowerShell source install of Noter from a clean
`main` checkout at the named commit on one Windows host. No published archive,
MSI, or Homebrew package was exercised. No real user document, account, remote
service, or telemetry was involved.

Exact-head CI run
[31010637393](https://github.com/blisspixel/noter/actions/runs/31010637393) on
the merged PR that produced this commit is green across fmt/clippy, docs, three
platform test jobs, and the supported-platform mutation union.

## Environment

| Item | Value |
| --- | --- |
| Host | Windows 11 Pro, build 26200 |
| Rust | `rustc 1.97.1 (8bab26f4f 2026-07-14)` |
| Cargo | `cargo 1.97.1 (c980f4866 2026-06-30)` |
| Installer | `scripts/install.ps1` |
| Install root | `%TEMP%\noter-alpha2-install` (disposable) |

## Commands

```text
.\scripts\install.ps1 -InstallRoot "$env:TEMP\noter-alpha2-install"
cargo uninstall noter --root "$env:TEMP\noter-alpha2-install"
```

## Observed install result

| Observation | Result |
| --- | --- |
| Install classification | Success |
| Reported version | `0.1.0-alpha.1` |
| Installed path | `%TEMP%\noter-alpha2-install\bin\noter.exe` |
| Executable size | 9,519,104 bytes |
| Executable SHA-256 | `15826dc30e8c793fa6d9a2fa6cef9844bd8638cc9e9916d7b7412783ffa553d5` |
| Uninstall | Removed the executable; path no longer present |

The installer printed the exact installed executable path and the package
version after a locked release build. The disposable root was not on the
process `PATH`; probes used the absolute path. `cargo uninstall` with the same
root removed only the Cargo-installed binary and left no residual executable at
that path.

## Automated shell and theme checks at this commit

| Check | Result |
| --- | --- |
| `theme::tests::persisted_values_round_trip` | Pass |
| `tests::update_command_opens_the_in_app_update_status` | Pass |
| Hosted CI install-from-source (PowerShell on Windows, POSIX on Linux/macOS) | Pass on run `31010637393` |

Theme menu persistence through the eframe storage key is covered by unit tests.
Interactive About dialog rendering and cross-session theme preference after a
GUI quit/relaunch are not claimed by this record.

## Availability and remaining evidence

| Requested evidence | Availability on 2026-08-05 | Disposition |
| --- | --- | --- |
| Disposable Windows PowerShell source install | Available | Executed as described above |
| Hosted Windows / Linux / macOS source-install steps in CI | Available | Pass on exact-head run `31010637393` |
| Theme persistence unit round-trip | Available | Pass |
| Update-status CLI path opens in-app status | Available | Pass |
| Interactive About dialog on installed GUI | Display session required | Not executed in this record |
| Theme preference survives GUI quit and relaunch | Display session required | Not executed in this record |
| Disposable clean-user MSI install/upgrade/uninstall | No release MSI published | Not executed |
| Homebrew and POSIX package install | No published formula or archive used | Not executed |
| Second disposable identity install | No authorized second identity | Not executed |

M2 remains in progress. In particular, this record does not prove installed
GUI semantic automation, cross-platform visual review of an installed binary,
or packaged-installer product identities.
