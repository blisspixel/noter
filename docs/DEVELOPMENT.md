# Development Guide

This guide covers the common contributor workflow. The
[code quality standards](CODE-QUALITY-STANDARDS.md) remain the authoritative
merge contract, and [CI](../.github/workflows/ci.yml) is the exact cross-platform
gate.

## Prerequisites

- [Git](https://git-scm.com/)
- [Rust installed through rustup](https://rustup.rs/)
- Python 3.11 or newer for repository validation scripts
- Ruff for the Python validation scripts, using the version pinned in CI

The repository pins Rust in [`rust-toolchain.toml`](../rust-toolchain.toml).
Rustup selects and installs that toolchain when a Cargo command runs in the
checkout.

## Run from a checkout

```sh
git clone https://github.com/blisspixel/noter.git
cd noter
cargo run --locked --release
```

Use the source installer when testing the installed application rather than a
Cargo-launched process:

```powershell
.\scripts\install.ps1 -Check
.\scripts\install.ps1
```

```sh
sh scripts/install.sh --check
sh scripts/install.sh
```

Installer behavior and custom roots are documented in
[INSTALLATION.md](INSTALLATION.md).

## Local validation

Run the focused local gates before proposing a change:

The examples use `python`. Use `python3` when that is the local Python 3 command.

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
ruff check scripts
ruff format --check scripts
python scripts/check_doc_links.py
python scripts/check_readme_assets.py
python scripts/check_release_config.py
python -m unittest discover -s scripts -p "test_*.py"
```

CI also runs rustdoc, dependency policy and advisory checks, enforced coverage,
native tests on Windows, macOS, and Linux, and the declared mutation scopes. A
local pass does not replace exact-commit CI evidence. Commands and thresholds
for the complete gate are maintained in
[CODE-QUALITY-STANDARDS.md](CODE-QUALITY-STANDARDS.md).

When the locked runtime dependency graph changes, install the cargo-about
version pinned in CI and regenerate the tracked notices with:

```sh
python scripts/generate_third_party_licenses.py
```

The generator consumes frozen cargo-about JSON, validates every component and
license mapping, and independently collects bounded legal files from locked
third-party package sources. Explicit license files, recognized legal-document
names, legal directories, and bundled font license sidecars are included;
conventional test, example, and benchmark trees and source-code lookalikes are
not inferred to be notices. Each
candidate must be a single-link regular file below a stable, non-reparse source
directory and is read through an identity-checked descriptor. The generator
preserves the union of selected license terms and packaged notices,
canonicalizes ordering and line endings, and replaces the inventory atomically.
This avoids making a cross-platform notice depend on the host that ran
cargo-about. Commit the regenerated inventory with the dependency change.

## README screenshots

The tracked screenshots are generated from Noter's real native renderer and the
non-sensitive demo file at [`assets/noter-demo.md`](assets/noter-demo.md). After
an intentional UI change, regenerate both Light and Dark captures on Windows:

```powershell
python scripts\update_readme_screenshots.py
python scripts\check_readme_assets.py
```

Review both images at full size before committing them. Confirm text alignment,
focus and selection state, theme contrast, demo content, dimensions, and the
absence of private data. The manual release expectations are in
[manual-test-matrix.md](manual-test-matrix.md).

## Repository hygiene

- Keep application code in `src/` or the appropriate workspace crate.
- Keep user and engineering documentation in `docs/` unless the file is a
  conventional repository root document.
- Keep generated builds, logs, coverage, mutation output, and local working
  notes in their ignored directories.
- Do not commit documents, paths, logs, screenshots, or fixtures containing
  private user data.
- Do not advance a roadmap item without the evidence named by its exit criteria.
