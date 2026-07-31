# Noter

[![CI](https://github.com/blisspixel/noter/actions/workflows/ci.yml/badge.svg)](https://github.com/blisspixel/noter/actions/workflows/ci.yml)

Noter is a private, cross-platform editor for plain text and Markdown, written
in Rust. It combines the speed and restraint of classic Notepad with a native,
formatted Markdown editing mode.

Tired of opening a local file only to meet an account prompt, a recurring
subscription, opaque "telemetry," advertising, or AI features you never asked
for? Noter is built around the opposite promise: useful modern editing, no
surveillance business model, no proprietary document format, and no feature
pileup between you and your words.

Text Mode is a fast, classic notepad surface that shows the exact file source.
Markdown Mode is modern where it matters: headings look like headings and
emphasis looks like emphasis, yet the formatted document remains directly
editable for supported content and stays ordinary Markdown on disk. A
conservative initial diagnostic set reports common portability problems without
changing the source.

There is no account, subscription, telemetry, advertising, cloud document
format, bundled AI, or remote content fetch while editing. Your files remain
portable `.txt` and `.md` files, ready to move between devices using whatever
storage, sync, or version-control tools you trust.

## Interface

These Light and Dark captures come from the current release-profile source
build. Both show the same local Markdown file in Markdown Mode.

| Light | Dark |
| --- | --- |
| ![Noter editing Markdown in the Light theme](docs/assets/noter-light.png) | ![Noter editing Markdown in the Dark theme](docs/assets/noter-dark.png) |

## Install

Signed binary releases are not published yet. The current installer builds the
locked source checkout with the Rust toolchain pinned by the repository.
Install [Git](https://git-scm.com/) and
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

Noter is under active development. The source build is suitable for evaluation
and careful dogfooding with backups, but should not yet be the only editor used
for important files. The [roadmap](docs/ROADMAP.md) separates implemented work
from the evidence required for the first public-quality release.

## Documentation

- [Documentation index](docs/README.md)
- [Native Markdown Mode](docs/MARKDOWN.md)
- [Privacy contract](docs/PRIVACY.md)
- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md)
- [Changelog](CHANGELOG.md)

## License

Noter is licensed under the [Apache License 2.0](LICENSE).
