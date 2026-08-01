#!/usr/bin/env python3
"""Regenerate README screenshots with Noter's real native renderer."""

from __future__ import annotations

import hashlib
import os
import subprocess
import tempfile
from pathlib import Path

from check_readme_assets import (
    EXPECTED_SIZE,
    REPOSITORY_ROOT,
    SCREENSHOTS,
    png_dimensions,
    screenshot_source_digest,
    validate,
)
from readme_screenshot_contract import SCREENSHOT_SPECS, ScreenshotSpec


DEMO_DOCUMENT = REPOSITORY_ROOT / "docs/assets/noter-demo.md"


def render(spec: ScreenshotSpec, output: Path) -> None:
    """Render one deterministic view and theme, requiring a clean exit."""

    command = [
        "cargo",
        "run",
        "--locked",
        "--release",
        "--features",
        "screenshot-qa",
        "--",
        "--theme",
        spec.theme,
        "--view",
        spec.view,
        "--screenshot",
        str(output),
        str(DEMO_DOCUMENT),
    ]
    subprocess.run(command, cwd=REPOSITORY_ROOT, env=os.environ.copy(), check=True)


def validate_generated_screenshot(path: Path) -> None:
    """Require a freshly rendered screenshot before replacing a tracked asset."""

    if not path.is_file():
        raise RuntimeError(f"Noter did not create the requested screenshot: {path}")
    dimensions = png_dimensions(path)
    if dimensions != EXPECTED_SIZE:
        raise RuntimeError(
            f"Noter rendered {dimensions[0]}x{dimensions[1]}, "
            f"expected {EXPECTED_SIZE[0]}x{EXPECTED_SIZE[1]}"
        )
    if path.stat().st_size < 20_000:
        raise RuntimeError(f"Noter rendered an implausibly small screenshot: {path}")


def render_and_promote(spec: ScreenshotSpec, output: Path) -> None:
    """Render to a unique sibling and atomically promote only a valid fresh PNG."""

    with tempfile.NamedTemporaryFile(
        dir=output.parent,
        prefix=f".{output.stem}-",
        suffix=".pending.png",
        delete=False,
    ) as handle:
        staged = Path(handle.name)
    staged.unlink()
    try:
        render(spec, staged)
        validate_generated_screenshot(staged)
        staged.replace(output)
    finally:
        staged.unlink(missing_ok=True)


def main() -> None:
    """Render the capture matrix, validate it, and report reproducible hashes."""

    for spec in SCREENSHOT_SPECS:
        render_and_promote(spec, REPOSITORY_ROOT / spec.path)
    validate(check_hashes=False, check_source_freshness=False)
    print(f"screenshot inputs  sha256:{screenshot_source_digest()}")
    for relative_output in SCREENSHOTS:
        data = (REPOSITORY_ROOT / relative_output).read_bytes()
        print(
            f"{relative_output.as_posix()}  sha256:{hashlib.sha256(data).hexdigest()}"
        )


if __name__ == "__main__":
    main()
