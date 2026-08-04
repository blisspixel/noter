# Noter

[![CI](https://github.com/blisspixel/noter/actions/workflows/ci.yml/badge.svg)](https://github.com/blisspixel/noter/actions/workflows/ci.yml)

**A clean editor. Nothing else.**

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

## Why this shape

- **Just the app.** No spyware, no telemetry, no activity logging, no ads, no
  account, no subscription, no cloud document format, no bundled AI. Update
  checks, when you choose them, stay explicit and separate from editing.
- **Local by default.** A compiled desktop app with a native window and GPU
  renderer. No WebView, no browser engine, no JavaScript runtime inside the
  editor, no remote content fetch while you write.
- **One document, full attention.** Classic single-file focus instead of a
  workspace that wants to become a platform.
- **Two exact views of the same source.** Text Mode shows every character.
  Markdown Mode shows the same file as readable structure with supported content
  still editable. Switching views never rewrites your bytes.

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
deterministic demo file. Green Screen and Amber Screen are complete themes, not
filters over a generic dark capture.

| Dark | Green Screen | Amber Screen |
| --- | --- | --- |
| ![Noter editing Markdown in the Dark theme](docs/assets/noter-dark.png) | ![Noter editing Markdown in the Green Screen theme](docs/assets/noter-green-screen.png) | ![Noter editing Markdown in the Amber Screen theme](docs/assets/noter-amber-screen.png) |

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

The current application makes no network request while you work. Even Help >
Check for Updates only opens a local status dialog unless you explicitly open
the releases page in your browser.

## Project status

Today the source build provides exact-source Text Mode, source-backed Markdown
Mode, Undo and Redo, Find and Replace, Go To Line, wrap, zoom, five themes, and
defensive local saves for ordinary UTF-8 `.txt` and `.md` files.

The current crate version is `0.1.0-alpha.1`. Noter is still an engineering
alpha. Private crash recovery is now scheduled and restored in the application
for dirty sessions (see the roadmap for remaining alpha.2 gates). Continuous
Markdown editing, accessibility matrices, remaining filesystem evidence, and
binary distribution remain open. Use it with backups rather than as the only
copy of important work.

The privacy stance above is product law for every release, including alpha. What
is still unfinished is reliability, completeness, and packaging, not a planned
telemetry path. The [roadmap](docs/ROADMAP.md) defines the ordered path through
correctness alpha (`0.1.0-alpha.2`), beta, release candidate, and the first
public-quality `0.1.0`.

## Documentation

- [Documentation index](docs/README.md)
- [Privacy contract](docs/PRIVACY.md)
- [Native Markdown Mode](docs/MARKDOWN.md)
- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md)
- [Changelog](CHANGELOG.md)

## License

Noter is licensed under the [Apache License 2.0](LICENSE).
