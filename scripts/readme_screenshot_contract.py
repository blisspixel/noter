"""Deterministic README screenshot capture contract."""

from __future__ import annotations

from pathlib import Path
from typing import NamedTuple


class ScreenshotSpec(NamedTuple):
    """One native renderer configuration and its tracked output."""

    theme: str
    view: str
    path: Path


SCREENSHOT_SPECS = (
    ScreenshotSpec("light", "text", Path("docs/assets/noter-light-text.png")),
    ScreenshotSpec("light", "markdown", Path("docs/assets/noter-light.png")),
    ScreenshotSpec("dark", "markdown", Path("docs/assets/noter-dark.png")),
    ScreenshotSpec("green", "markdown", Path("docs/assets/noter-green-screen.png")),
    ScreenshotSpec("amber", "markdown", Path("docs/assets/noter-amber-screen.png")),
)
SCREENSHOTS = tuple(spec.path for spec in SCREENSHOT_SPECS)
EXPECTED_SIZE = (1200, 760)
