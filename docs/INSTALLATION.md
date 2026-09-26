# Installation and Updates

**Reviewed:** 2026-09-26

**Current availability:** The published `0.1.0-beta.1` release is
available for careful evaluation with backups. Its artifacts include platform
archives, a Windows MSI, checksums, SBOMs, and GitHub build provenance.
Windows and macOS artifacts are not yet platform-signed, and no self-updating
release channel exists. Source installation remains supported. See the
[release process](RELEASING.md).

## Prerelease artifacts

The publication location is the
[GitHub release](https://github.com/blisspixel/noter/releases/tag/v0.1.0-beta.1).
Download an asset before executing it and verify its GitHub attestation.
Published SHA-256 sidecars and
the unified checksum list cover the source archive, platform archives, and
Windows MSI, but not installer scripts, the Homebrew formula, SBOMs, or the
distribution manifest. For the strongest alpha path, manually download an
archive and verify both its checksum and attestation before extraction. Inspect
any installer script before use and never pipe a remote installer into a shell.
The Windows MSI is an unsigned per-machine evaluation package. It requires
elevation, installs under Program Files, and offers a system PATH entry; it is
not a supported stable installer.

## Standalone binary install

Precompiled binaries are published for each supported platform on the
[GitHub Releases page](https://github.com/blisspixel/noter/releases). The
installer scripts install one without build tools. Download the script, read
it, then run it; do not pipe a remote script into a shell.

Windows (PowerShell):

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/blisspixel/noter/main/scripts/install.ps1 -OutFile install-noter.ps1
powershell -ExecutionPolicy Bypass -File .\install-noter.ps1
```

macOS or Linux:

```sh
curl -fsSLo install-noter.sh https://raw.githubusercontent.com/blisspixel/noter/main/scripts/install.sh
sh install-noter.sh
```

The binary installer:

1. finds the newest release, prereleases included, or the one named with
   `--version` (`-Version` on Windows);
2. picks the archive for the operating system and CPU architecture (x64 on
   Windows);
3. downloads the matching archive and its SHA-256 sidecar and verifies the
   checksum before extraction;
4. checks that the downloaded binary reports the expected version;
5. copies it beside the destination and renames it into place, into
   `%LOCALAPPDATA%\Programs\Noter\bin` on Windows or `~/.local/bin` on macOS
   and Linux (`--root` / `-InstallRoot` chooses another root);
6. on Windows, adds that directory to the user `PATH`, keeping existing
   `%VARIABLE%` entries unexpanded; on macOS and Linux, prints a reminder when
   the directory is not on `PATH`.

`--uninstall` (`-Uninstall`) removes the binary, and on Windows its `PATH`
entry when the directory is otherwise empty. Documents, settings, and recovery
records are not touched. `--check` (`-Check`) validates the plan without
installing.

The checksum sidecar comes from the same release as the archive, so it proves
the download arrived intact, not who built it. To verify provenance as well,
download the archive by hand and check its
[GitHub attestation](https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations/using-artifact-attestations-to-establish-provenance-for-builds)
with `gh attestation verify <archive> --repo blisspixel/noter` before
extracting it.

On Linux the window needs a few system libraries; see
[Linux window libraries](#linux-window-libraries).

## Source prerequisites

Install the following before using the source installer:

- [Git](https://git-scm.com/)
- [Rust through rustup](https://rustup.rs/)
- Windows, macOS, or Linux on a machine able to build a native Rust application

The repository pins Rust in `rust-toolchain.toml`. Rustup and Cargo may download
that toolchain and locked dependencies from their configured sources during the
first build.

## Source install

Windows PowerShell:

```powershell
git clone https://github.com/blisspixel/noter.git
cd noter
.\scripts\install.ps1
```

macOS or Linux:

```sh
git clone https://github.com/blisspixel/noter.git
cd noter
sh scripts/install.sh
```

The source installer:

1. validates the local locked Cargo workspace;
2. builds the release executable with the repository's pinned toolchain;
3. replaces an older Cargo-installed Noter build at the selected install root;
4. verifies `noter --version`; and
5. verifies the installed command-line error and exit-status contract.

Run from a checkout, the installer builds that source; pass `--binary`
(`-Binary`) to download a release instead. It does not fetch Noter source,
modify shell startup files, or request administrator access. It prints the
exact installed executable path when it finishes.

## Start and verify

Open a new terminal after the first Rust installation, then run:

```sh
noter --version
noter
```

Pass an existing document path to open it directly, for example
`noter README.md` from the repository checkout. Noter never creates that file
for you: a mistyped path fails on the command line instead of opening a window
that looks like a new blank document.

If `noter` is not found, use the exact path printed by the installer or add its
parent `bin` directory to `PATH`. A source install defaults to Cargo's per-user
binary directory, normally `%USERPROFILE%\.cargo\bin` on Windows and
`$HOME/.cargo/bin` on macOS and Linux; a binary install defaults to the
directories listed above.

Noter remains under active development. Keep backups and do not use it as the
only editor for important files until the release evidence in the roadmap is
complete.

### Linux window libraries

On Linux and the BSDs the window loads its keyboard, display, and OpenGL
libraries when it starts. Desktop installations normally have them. On a
minimal system, install:

| Distribution | X11 session | Wayland session |
| --- | --- | --- |
| Debian or Ubuntu | `libxkbcommon0 libxkbcommon-x11-0 libegl1` | `libxkbcommon0 libwayland-client0 libwayland-egl1 libegl1` |
| Fedora | `libxkbcommon libxkbcommon-x11 mesa-libEGL` | `libxkbcommon libwayland-client libwayland-egl mesa-libEGL` |
| Arch | `libxkbcommon libxkbcommon-x11 libglvnd` | `libxkbcommon wayland libglvnd` |

On X11, `libGL` (GLX) can stand in for `libEGL`. The session is Wayland when
`WAYLAND_DISPLAY` or `WAYLAND_SOCKET` is set.

If one is missing, Noter names it with these packages and exits with status 1 instead of opening
the window. The terminal interface (`noter --tui`) needs none of them.

## Command-line contract

```text
noter [OPTIONS] [--] [FILE]
noter update
```

| Invocation | Exit | Behavior |
| --- | --- | --- |
| `noter --version`, `noter -V` | 0 | Prints `noter <version>` and exits |
| `noter --help`, `noter -h`, `noter update --help` | 0 | Prints the usage block and exits |
| `noter` | runs | Opens an untitled document in a window, or in the terminal interface on a headless Linux or BSD terminal |
| `noter FILE` | runs | Opens an existing readable file |
| `noter --tui [FILE]` | runs | Opens the document in interactive Terminal UI (TUI) mode |
| `noter --gui [FILE]` | runs | Forces graphical desktop interface mode |
| `noter update` | runs | Opens the local update status, titled `Update status - Noter`, or prints it where the terminal interface would open |
| Unknown option, invalid or missing option value, second document path | 2 | One line on standard error, then usage |
| A Linux or BSD window library cannot be loaded | 1 | The missing library and the packages that provide it |
| FILE missing, a directory, or unreadable | 2 | `noter: cannot open ...`, then usage |

`--theme system|light|dark|green|amber` and `--view text|markdown` select the
startup theme and view. Their values are accepted in any letter case. `--` ends
option parsing so a document path may begin with `-`.

Without `--gui` or `--tui`, Noter opens a window. On Linux, the BSDs, and other
non-macOS Unix systems, where windows need an X11 or Wayland display server, a
launch with none of `DISPLAY`, `WAYLAND_DISPLAY`, or `WAYLAND_SOCKET` set to a
non-empty value
whose standard input and standard output are both interactive terminals opens
the terminal interface instead, such as over SSH. `noter update` in that
situation prints the update status to standard output and exits. macOS and
Windows always open a window unless `--tui` is given. `--gui` and `--tui`
together are an argument error.

Argument mistakes fail on the command line. Problems with a file's *content*,
such as invalid UTF-8 or a document above the current interactive size limit,
open the window and report there, because those also reach Noter through the
Open dialog and desktop file associations rather than through a terminal.

The window title names the open document and marks unsaved changes with `*`. It
does not name the active view; the Mode control in the upper right of the window
shows and switches Text Mode and Markdown Mode.

An idle Noter window sleeps. It repaints on input, on the bounded external-change
check while focused, and when a dirty document is due to write its private
recovery copy.

## Update a source installation

Review and fast-forward the existing checkout, then rerun its installer.

Windows PowerShell:

```powershell
git pull --ff-only
.\scripts\install.ps1
```

macOS or Linux:

```sh
git pull --ff-only
sh scripts/install.sh
```

`--ff-only` refuses an implicit merge when local history has diverged. The
installer passes `--locked` and `--force` to Cargo, so it honors the committed
lockfile and replaces the existing source-installed executable.

## Installer options

| PowerShell | POSIX shell | Purpose |
| --- | --- | --- |
| `-Binary` | `--binary` | Download a release even when run from a checkout |
| `-FromSource` | `--from-source` | Build the checkout that contains the script |
| `-Source <path>` | `--source <path>` | Build a different local Noter checkout |
| `-Version <version>` | `--version <version>` | Download this release instead of the newest (binary installs only) |
| `-InstallRoot <path>` | `--root <path>` | Install into `<path>/bin` |
| `-Uninstall` | `--uninstall` | Remove the installed binary |
| `-Check` | `--check` | Validate the plan without installing |

Examples:

```powershell
.\scripts\install.ps1 -Check
.\scripts\install.ps1 -Binary -Version 0.1.0-beta.1
```

```sh
sh scripts/install.sh --check
sh scripts/install.sh --binary --version 0.1.0-beta.1
```

An explicit install root takes precedence, then `CARGO_INSTALL_ROOT` when set.
Otherwise a source build uses `CARGO_HOME` or Cargo's standard per-user
directory, and a binary install uses `%LOCALAPPDATA%\Programs\Noter` on
Windows or `~/.local` on macOS and Linux. The scripts pass the resulting
absolute path to Cargo so repository or user configuration cannot silently
redirect the executable. Only an x64 Windows build is published; Windows on
ARM runs it through x64 emulation.

## Uninstall a source build

For an installation in Cargo's default root:

```sh
cargo uninstall noter
```

For a custom root, use the same root supplied during installation. For example:

```powershell
cargo uninstall noter --root "$env:LOCALAPPDATA\Noter"
```

```sh
cargo uninstall noter --root "$HOME/.local"
```

Cargo removes the executable and its install record. It does not remove the Git
checkout or Noter's per-user framework state. The current build stores its
selected theme, Text Mode word-wrap preference, and editor zoom in `app.ron`
under the following directory:

| Platform | State directory |
| --- | --- |
| Windows | `%APPDATA%\Noter\data` |
| macOS | `~/Library/Application Support/Noter` |
| Linux | `$XDG_DATA_HOME/noter`, or `~/.local/share/noter` when unset |

Inspect that directory before deleting it.

Owner-restricted crash-recovery files use a subdirectory of that state root
(`recovery/records` for active instance records and `recovery/quarantine` for
damaged files). Dirty editing sessions persist recovery copies there; Save and
explicit Discard remove the owned record.

Alpha.2 recovery is supported only when the selected state path resolves to a
normally permissioned, local, owner-controlled per-user directory. The
prerelease does not yet verify or bind the enclosing state and recovery
directories. If `%APPDATA%`, `XDG_DATA_HOME`, or the platform application-support
path is group-writable, ACL-shared, redirected, synchronized, network-hosted,
removable, or on a weak filesystem, recovery is unverified. Keep important work
saved and backed up, and do not rely on recovery from that state root.

The current unreleased Windows build begins M4-H1 by validating the fixed NTFS
directory chain, rejecting reparse and cross-volume components, retaining its
directory handles, rejecting state ACLs with unprivileged mutation rights, and
hardening the recovery subtree to a protected inheritable user-and-SYSTEM DACL
before writing recovery bytes. This foundation does not yet route record
operations through directory handles or prove that a fixed local profile is not
synchronized or redirected. Linux and macOS namespace binding also remains
open, so this is not yet the beta recovery contract.

Uninstall and cleanup distinguish:

| Kind | Location | Safe to delete when |
| --- | --- | --- |
| Preferences | `app.ron` in the state directory above | You want default theme, wrap, and zoom |
| Recovery records | `recovery/` under the same state root | You have saved or discarded all unsaved work |

## Troubleshooting

### Cargo is not found

Install Rust through rustup, allow it to configure the Cargo binary directory,
open a new terminal, and rerun the installer.

### The pinned toolchain cannot be downloaded

Confirm that rustup can reach its configured distribution server, then run
`rustup show` in the checkout. The installer does not bypass the pinned
toolchain or substitute an unverified compiler.

### The install succeeds but `noter` is not found

Use the exact executable path printed at the end of installation. Add that
path's parent `bin` directory to `PATH` if the command should be available in
future terminals.

### A custom installation root behaves unexpectedly

Run the installer with `-Check` or `--check`, then pass an explicit absolute
root. Use that same root for future updates and uninstallation.

## Current update actions

`Help > Check for Updates` and `noter update` open the same local status dialog.
The current dialog explains that Noter does not check in the background and can
open the GitHub releases page only after an explicit user action. It does not
query an API, download an artifact, or replace the application.

A session started by `noter update` names its window `Update status - Noter`
until the status is closed, so the command is not mistaken for a blank editor.
Closing the status returns the window to ordinary document titles.

This boundary avoids presenting a source checkout as a secure release channel.
Documentation will not recommend piping a script from a mutable branch into a
shell. A remote one-line installer becomes appropriate only after immutable,
versioned artifacts and authenticated release metadata exist.

## First-release contract

The first supported binary release must provide:

- portable archives for each supported operating-system and architecture pair;
- a PowerShell installer for Windows and a POSIX installer for macOS and Linux;
- an MSI or similarly native Windows package;
- a Homebrew formula for macOS;
- per-user installation without elevation by default;
- one release-manifest and artifact-verification policy shared by the GUI,
  command, and installers;
- safe reinstall, upgrade, rollback, and uninstall behavior; and
- clear disclosure when platform signing is unavailable.

Release bootstrap instructions must use a stable official endpoint, resolve an
immutable versioned artifact, verify authenticated metadata, and leave the
installer inspectable before execution.

## Update safety requirements

- Checks and downloads occur only after an explicit user action unless a later
  opt-in setting is separately designed and approved.
- Requests contain no document data, path, account, stable installation ID, or
  telemetry payload.
- The current and offered versions, release notes, size, and verification state
  are visible before replacement.
- An update cannot proceed while a document has unsaved changes.
- Downloads are bounded, staged privately, authenticated, and installed without
  damaging the working version on failure.
- Windows uses a separate verified updater process because a running executable
  cannot safely replace itself.
- Downgrades require an explicit version and confirmation.

## Release manifest

Every published release requires a machine-readable manifest containing:

- semantic version, channel, and source commit;
- supported operating-system and architecture tuples;
- exact artifact names, lengths, and SHA-256 digests;
- minimum supported operating-system versions;
- release notes and compatibility information; and
- signature and provenance references when available.

Installers fail closed on a missing asset, unsupported platform, malformed
manifest, incomplete download, digest mismatch, or unapproved downgrade. They
never fall back to building a mutable branch.

## Release verification matrix

Release evidence must cover clean install, reinstall, upgrade, downgrade,
rollback, and uninstall on Windows, Intel and Apple Silicon macOS, X11, and
Wayland. It must also cover paths with spaces and non-ASCII characters,
standard-user operation, interrupted and corrupted downloads, trust failures,
retained documents and settings, and recovery after installation failure.
