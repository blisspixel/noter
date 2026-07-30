# Noter

[![CI](https://github.com/blisspixel/noter/actions/workflows/ci.yml/badge.svg)](https://github.com/blisspixel/noter/actions/workflows/ci.yml)

Noter is a focused, private editor for plain text and Markdown, written in Rust
for Windows, macOS, and Linux.

Text Mode is a classic notepad surface that shows the exact file source.
Markdown Mode presents that same source as formatted, directly editable content,
so headings look like headings and emphasis looks like emphasis while the file
remains ordinary Markdown on disk. Noter uses no proprietary document format,
requires no account, collects no telemetry, and fetches no remote content while
editing. A conservative initial diagnostic set reports common Markdown
portability problems without changing the source.

## Interface

These Light and Dark captures come from Noter's native release renderer. Both
show the same local Markdown file in Markdown Mode.

| Light | Dark |
| --- | --- |
| ![Noter editing Markdown in the Light theme](docs/assets/noter-light.png) | ![Noter editing Markdown in the Dark theme](docs/assets/noter-dark.png) |

## Install

Noter is pre-alpha and does not yet have a signed binary release. The current
installer builds the locked source checkout with the Rust toolchain pinned by
the repository. Install [Git](https://git-scm.com/) and
[Rust through rustup](https://rustup.rs/), then run the commands for your
platform.

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

Start Noter with `noter`, or verify the installation with `noter --version`.
The [installation guide](docs/INSTALLATION.md) covers updates, custom install
locations, uninstallation, troubleshooting, and the future binary-release
contract.

## Project status

Noter is under active development and should not yet be the only editor used
for important files. The [roadmap](docs/ROADMAP.md) records what is implemented,
what remains, and the evidence required for the first public-quality release.

## Documentation

- [Documentation index](docs/README.md)
- [Native Markdown Mode](docs/MARKDOWN.md)
- [Privacy contract](docs/PRIVACY.md)
- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md)
- [Changelog](CHANGELOG.md)

## License

Noter is licensed under the [Apache License 2.0](LICENSE).
