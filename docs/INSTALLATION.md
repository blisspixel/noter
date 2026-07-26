# Installation and Updates

**Status:** Source installation is available. Noter has not published a binary
release, signed package, or self-updating release channel.

## Install the current source build

The source installers validate the locked workspace, build with the pinned Rust
toolchain, replace an existing Cargo installation, and verify the resulting
binary. They do not fetch Noter source or require administrator access. On a
machine without the pinned toolchain or locked crates in its local cache,
rustup and Cargo may download those build inputs from their configured sources.

Windows PowerShell:

```powershell
git clone https://github.com/blisspixel/noter
cd noter
./scripts/install.ps1
```

macOS or Linux:

```bash
git clone https://github.com/blisspixel/noter
cd noter
sh scripts/install.sh
```

Rust must already be installed through [rustup](https://rustup.rs). The
repository pins its toolchain in `rust-toolchain.toml`. Cargo installs to its
normal per-user binary directory unless a different root is supplied.

## Update a source installation

Update the checkout through an explicit Git operation, review the incoming
revision, then rerun the same installer:

```bash
git pull --ff-only
sh scripts/install.sh
```

```powershell
git pull --ff-only
./scripts/install.ps1
```

The scripts pass `--locked` and `--force` to Cargo, so the lockfile is honored
and the existing source-installed binary is replaced.

## Installer options

| PowerShell | POSIX shell | Purpose |
| --- | --- | --- |
| `-Source <path>` | `--source <path>` | Install a different local Noter checkout |
| `-InstallRoot <path>` | `--root <path>` | Use a specific Cargo installation root |
| `-Check` | `--check` | Validate prerequisites and source without installing |

Examples:

```powershell
./scripts/install.ps1 -Check
./scripts/install.ps1 -InstallRoot "$env:LOCALAPPDATA\Noter"
```

```bash
sh scripts/install.sh --check
sh scripts/install.sh --root "$HOME/.local"
```

The explicit install-root option takes precedence. Without it, the scripts use
`CARGO_INSTALL_ROOT` when set, then `CARGO_HOME`, then the standard per-user
Cargo directory. The scripts pass that absolute root to Cargo so a repository
or user Cargo configuration cannot silently redirect the executable elsewhere.

## Current update actions

`Help > Check for Updates` and `noter update` open the same local update-status
dialog. In the current pre-alpha build, that dialog states that no verified
release exists and offers an explicit link to the GitHub releases page. It does
not query an API, download an artifact, or replace the application.

This limitation is intentional. A self-updater is unsafe until release assets,
manifests, checksums, provenance, rollback behavior, and clean-machine tests
exist.

## First-release contract

The first supported binary release must provide:

- portable archives for every supported operating-system and architecture pair;
- a PowerShell installer for Windows and a POSIX installer for macOS and Linux;
- an MSI or similarly native Windows package;
- a Homebrew formula for macOS;
- per-user installation without elevation by default;
- one shared release-manifest and artifact-verification policy for the GUI,
  command, and installers;
- safe reinstall, upgrade, rollback, and uninstall behavior; and
- clear disclosure when platform signing is not yet available.

Release bootstrap instructions must download an installer from an immutable
versioned release asset, verify it against authenticated release metadata, and
leave the script inspectable before execution. Documentation will not recommend
executing a script from a mutable branch.

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

## Verification matrix

Release evidence must cover clean install, reinstall, upgrade, downgrade,
rollback, and uninstall on Windows, Intel and Apple Silicon macOS, X11, and
Wayland. It must also cover paths with spaces and non-ASCII characters,
standard-user operation, interrupted and corrupted downloads, trust failures,
retained documents and settings, and recovery after installation failure.
