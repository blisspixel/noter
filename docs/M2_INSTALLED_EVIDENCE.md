# M2 Installed-Product Evidence

**Observed:** 2026-08-05

**Exact commit:** `91ed8d7a05e460ef12f357eb112694a1870af0bd`

**Status:** Partial local M2 evidence. This is not M2 milestone sign-off.

## Scope and provenance

This record covers a disposable PowerShell source install of Noter from a clean
`main` checkout at the named commit on one Windows host. No published archive,
MSI, or Homebrew package was exercised. No real user document, account, remote
service, or telemetry was involved.

Exact-head CI on `main` for this commit is run
[31017380468](https://github.com/blisspixel/noter/actions/runs/31017380468)
(`headSha` `91ed8d7…`). It is green across fmt/clippy, docs, three platform
test jobs, and the supported-platform mutation union.

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

## Windows release-binary command-line output

**Observed:** 2026-08-29

**Exact source:** the console-attachment change on this branch, built locally as
`cargo build --release --locked`.

Release builds set `windows_subsystem = "windows"`, so a shell-launched process
inherits no console and standard output reaches nothing. This record covers the
console attachment that restores the documented command-line contract. It is a
local Windows measurement, not a packaged-installer or cross-platform result.

The probe allocates its own console, launches the release binary through
`ShellExecute` so no handle is inherited, waits for exit, and reads the console
screen buffer back with `ReadConsoleOutputCharacterW`. Reading the buffer rather
than a pipe is what makes the result meaningful: a redirected stream would have
printed with or without the change.

| Item | Value |
| --- | --- |
| Host | Windows 11 Pro, build 26200 |
| Binary | `target/release/noter.exe` |
| Executable size | 9,760,256 bytes |
| Executable SHA-256 | `1d761d8a06c4434f8446017fae69207a0c2454c5780c0bf54bf4351f842cc603` |
| PE subsystem | 2 (Windows GUI) |
| Linked console imports | `AttachConsole`, `GetStdHandle`, `SetStdHandle` |

| Invocation | Console buffer | Exit code |
| --- | --- | --- |
| `noter --version` | `noter 0.1.0-alpha.2` | 0 |
| `noter --help` | `Noter` usage block | 0 |
| `noter --bogus-option` | ``noter: unknown option `--bogus-option` `` | 2 |
| `noter C:\definitely\missing\path.txt` | ``noter: cannot open `C:\definitely\missing\path.txt`: no such file`` | 2 |

A control run of the same probe against a release binary built from the parent
commit `42b357d`, which does not contain the change, produced an empty console
buffer and exit code 0 for `--version`. The difference between the two runs is
the attachment itself, not the probe.

Redirection is unchanged: `noter --version > file` wrote `noter 0.1.0-alpha.2`
to the file and exited 0, so a stream the shell already bound keeps its own
handle instead of being pointed at a console.

This is the installed-binary evidence referenced by the console exclusions in
`.cargo/mutants.toml`. Those exclusions cover the unsafe call sequence only;
the decisions it applies are proved by tests in `noter-platform`.

## Availability and remaining evidence

| Requested evidence | Availability on 2026-08-05 | Disposition |
| --- | --- | --- |
| Disposable Windows PowerShell source install | Available | Executed as described above |
| Hosted Windows / Linux / macOS source-install steps in CI | Available | Pass on exact-head main run `31017380468` |
| Theme persistence unit round-trip | Available | Pass |
| Update-status CLI path opens in-app status | Available | Pass |
| Interactive About dialog on installed GUI | Display session required | Not executed in this record |
| Theme preference survives GUI quit and relaunch | Display session required | Not executed in this record |
| Disposable clean-user MSI install/upgrade/uninstall | No release MSI published | Not executed |
| Homebrew and POSIX package install | No published formula or archive used | Not executed |
| Second disposable identity install | No authorized second identity | Not executed |
| Windows release-binary command-line output | Available | Pass, recorded above on 2026-08-29 |
| Same command-line output on macOS and Linux release builds | Display session required | Not executed in this record |

M2 remains in progress. In particular, this record does not prove installed
GUI semantic automation, cross-platform visual review of an installed binary,
or packaged-installer product identities.
