# Noter

[![CI](https://github.com/blisspixel/noter/actions/workflows/ci.yml/badge.svg)](https://github.com/blisspixel/noter/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/blisspixel/noter?include_prereleases)](https://github.com/blisspixel/noter/releases)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

**A clean, private, buttery-smooth text and Markdown editor. Nothing else.**

Privacy first. Zero spyware. Zero telemetry. Zero activity logging. No analytics
panel, no usage funnel, no silent crash phone-home, no background history of
what you wrote or which files you opened. Noter is only the tool you asked for:
open a file, write, save. The rest is deliberately not there.

Noter is a local, cross-platform editor for plain text and Markdown. Use it for
notes, drafts, journals, fiction, arguments, code comments, anything you need
to put into words. Write freely. No account, no feed, no product that studies
you while you work. Freedom of creativity and freedom of speech both need
software that does not second-guess, score, or siphon the work.

Your files stay ordinary portable `.txt` and `.md` on disk. Noter does not lock
them into a proprietary cloud format, a subscription vault, or a bundled AI
pipeline. You choose how to store, sync, back up, or publish. Local preferences
such as theme, wrap, and zoom are ordinary settings on your machine, not a
dossier.

## Highlights

- **Private by Design:** A compiled native desktop and terminal application. No
  WebView, no browser engine, no background network access, and zero telemetry.
  An idle window sleeps instead of draining battery on continuous repaints.
- **One File, Three Exact Views:**
  - **Text Mode:** Raw byte fidelity, explicit line endings, and exact source
    inspection.
  - **Markdown Mode:** Directly editable live structure with sticky formatting
    and caret-aware delimiter unfurl.
  - **Terminal TUI Mode:** A fast, lightweight terminal interface ("Like Nano...
    but better") with dual modern and Nano shortcuts, mouse support, and CRT
    themes (`noter --tui`).
- **Defensive Durability:** Atomic file replacement, BLAKE3 content hashing,
  sibling staging, verified permissions, and owner-restricted crash recovery.
- **Responsive and Butter-Smooth:** Sub-16.7ms input latency, 120Hz/ProMotion
  frame pacing, and a virtualized rope engine that handles files up to 50 MiB
  with zero hesitation.
- **One Document, Full Focus:** Single-document ergonomics instead of a
  workspace that wants to become a platform.

The full privacy contract is in [docs/PRIVACY.md](docs/PRIVACY.md).

## Interface

### One file, two views

These captures show the identical local file in Text Mode and Markdown Mode.
The source bytes do not change when the view changes.

| Text Mode | Markdown Mode |
| --- | --- |
| ![Noter showing the exact Markdown source in Text Mode](docs/assets/noter-light-text.png) | ![Noter showing the same file as editable structure in Markdown Mode](docs/assets/noter-light.png) |

### Light, dark, and terminal themes

Each theme uses the same native text shaping, source-backed Markdown editor, and
deterministic demo file. Green Screen and Amber Screen are complete CRT phosphor
themes, not simple filters over a generic dark capture.

| Dark | Green Screen | Amber Screen |
| --- | --- | --- |
| ![Noter editing Markdown in the Dark theme](docs/assets/noter-dark.png) | ![Noter editing Markdown in the Green Screen theme](docs/assets/noter-green-screen.png) | ![Noter editing Markdown in the Amber Screen theme](docs/assets/noter-amber-screen.png) |

## Install

### Quick install (Standalone binaries)

Precompiled binaries for Windows, Linux, and macOS (Intel and Apple Silicon) are
published on the [GitHub Releases page](https://github.com/blisspixel/noter/releases).
You can install Noter in seconds with zero build tools:

Windows (PowerShell):

```powershell
Invoke-RestMethod https://github.com/blisspixel/noter/releases/latest/download/install.ps1 | Invoke-Expression
```

macOS or Linux (POSIX shell):

```sh
curl -fsSL https://github.com/blisspixel/noter/releases/latest/download/install.sh | sh
```

The installer verifies cryptographic SHA-256 checksums, installs into your user
binary directory, and configures `PATH`.

### Source install

You can also build the locked source checkout with the pinned Rust toolchain:

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

Start Noter with `noter [FILE]`, or launch the terminal interface with
`noter --tui [FILE]`. Check version with `noter --version`. Full command-line
and update contracts are documented in [docs/INSTALLATION.md](docs/INSTALLATION.md).

## Project status

The current version is `0.1.0-beta.1`. Durable save,
crash recovery, clipboard parity, conflict detection, caret navigation, and
themes are verified. Production rope virtualization (M5), continuous fluid
Markdown (M6), and cross-platform binary distribution with TUI mode (M7) are
in active progression.

The privacy stance above is product law for every release, including alpha. What
is still unfinished is reliability, completeness, and packaging, not a planned
telemetry path. The [roadmap](docs/ROADMAP.md) defines the ordered path through
correctness alpha, beta, release candidate, and the first public-quality `0.1.0`.

## Documentation

Detailed architecture specifications, functional contracts, and evidence records
are maintained in the `docs/` index:

| Document | Purpose |
| --- | --- |
| [Roadmap](docs/ROADMAP.md) | Release milestones, version train, and exit criteria |
| [Technical Design](docs/DESIGN.md) | System architecture, state reducers, virtualized editor, and TUI |
| [Product Requirements](docs/REQUIREMENTS.md) | Functional contracts, performance budgets, and non-goals |
| [Installation & Updates](docs/INSTALLATION.md) | Binary installers, package management, and updater contracts |
| [Native Markdown Mode](docs/MARKDOWN.md) | Continuous editing, sticky formatting, and CommonMark conformance |
| [Privacy Contract](docs/PRIVACY.md) | Offline law, zero-telemetry guarantee, and local state bounds |
| [Code Quality Standards](docs/CODE-QUALITY-STANDARDS.md) | Merge gates, testing standards, and mutation coverage rules |
| [Changelog](CHANGELOG.md) | User-visible releases and version history |
| [Contributing](CONTRIBUTING.md) | Working agreement, development rules, and pull requests |
| [Security Policy](SECURITY.md) | Vulnerability disclosure and security contacts |

## License

Noter is licensed under the [Apache License 2.0](LICENSE).
