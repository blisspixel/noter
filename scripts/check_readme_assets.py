#!/usr/bin/env python3
"""Validate tracked README screenshots without requiring a graphical session."""

from __future__ import annotations

import hashlib
import os
import stat
import struct
import zlib
from pathlib import Path

from readme_screenshot_contract import EXPECTED_SIZE, SCREENSHOTS


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
MAX_PNG_BYTES = 2 * 1024 * 1024
MAX_DECODED_BYTES = (1 + EXPECTED_SIZE[0] * 4) * EXPECTED_SIZE[1]
MAX_SCREENSHOT_SOURCE_BYTES = 4 * 1024 * 1024
MAX_SCREENSHOT_SOURCE_TOTAL_BYTES = 16 * 1024 * 1024
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
SCREENSHOT_SOURCE_FILES = (
    Path("Cargo.lock"),
    Path("Cargo.toml"),
    Path("assets/fonts/InterVariable.ttf"),
    Path("docs/assets/noter-demo.md"),
    Path("rust-toolchain.toml"),
    Path("scripts/readme_screenshot_contract.py"),
    Path("scripts/update_readme_screenshots.py"),
)
SCREENSHOT_SOURCE_GLOBS = (
    "crates/**/Cargo.toml",
    "crates/**/*.rs",
    "src/**/*.rs",
)
EXPECTED_SCREENSHOT_SOURCE_SHA256 = (
    "14852d5a46e6947d2527f6aa7b8858892ace9a08d0c91e9f6a29a7ae19743b08"
)
EXPECTED_SHA256 = {
    Path(
        "docs/assets/noter-light-text.png"
    ): "5c7bcaff24fe2b7fb0e609b72191f7b8356fe14c85462866c38f83210a74fd6c",
    Path(
        "docs/assets/noter-light.png"
    ): "3fd33e033a5919569336b45d0121b48af223d67cbab64770786d487db562c12f",
    Path(
        "docs/assets/noter-dark.png"
    ): "c0e1eb06e5b30e4d2704f4dbddd9a0317b0b2dfad988f105fb84d95af29d8784",
    Path(
        "docs/assets/noter-green-screen.png"
    ): "553b1f5ccebe2528c140a3fe25fb0b85a7805551984dc7380bade76b995fbe03",
    Path(
        "docs/assets/noter-amber-screen.png"
    ): "39225b0b50ea03f6f90b09d83a4db15b1340c9a8c44210bcab650166a4bccabc",
}


def read_bounded_regular_file(
    path: Path, *, maximum: int = MAX_PNG_BYTES, description: str = "PNG"
) -> bytes:
    """Read one regular file without following repository symlinks."""

    metadata = path.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise ValueError(f"{path} is not a regular file")
    if metadata.st_size > maximum:
        raise ValueError(f"{path} exceeds the {description} file-size limit")

    with path.open("rb") as stream:
        if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
            raise ValueError(f"{path} is not a regular file")
        data = stream.read(maximum + 1)
    if len(data) > maximum:
        raise ValueError(f"{path} exceeds the {description} file-size limit")
    return data


def screenshot_source_paths(root: Path = REPOSITORY_ROOT) -> tuple[Path, ...]:
    """Return every deterministic input to the native screenshot build."""

    paths = set(SCREENSHOT_SOURCE_FILES)
    for pattern in SCREENSHOT_SOURCE_GLOBS:
        paths.update(path.relative_to(root) for path in root.glob(pattern))
    return tuple(sorted(paths, key=lambda path: path.as_posix()))


def screenshot_source_digest(
    root: Path = REPOSITORY_ROOT, paths: tuple[Path, ...] | None = None
) -> str:
    """Hash path-framed screenshot inputs without accepting links or large files."""

    selected_paths = screenshot_source_paths(root) if paths is None else paths
    selected_paths = tuple(sorted(selected_paths, key=lambda path: path.as_posix()))
    if len(selected_paths) > 256:
        raise ValueError("screenshot input count exceeds the validation limit")

    digest = hashlib.sha256()
    total = 0
    for relative_path in selected_paths:
        encoded_path = relative_path.as_posix().encode("utf-8")
        data = read_bounded_regular_file(
            root / relative_path,
            maximum=MAX_SCREENSHOT_SOURCE_BYTES,
            description="screenshot source",
        )
        total += len(data)
        if total > MAX_SCREENSHOT_SOURCE_TOTAL_BYTES:
            raise ValueError("screenshot inputs exceed the aggregate size limit")
        digest.update(len(encoded_path).to_bytes(4, "big"))
        digest.update(encoded_path)
        digest.update(len(data).to_bytes(8, "big"))
        digest.update(data)
    return digest.hexdigest()


def png_dimensions(path: Path) -> tuple[int, int]:
    """Fully validate an 8-bit RGBA PNG and return its dimensions."""

    data = read_bounded_regular_file(path)
    if not data.startswith(PNG_SIGNATURE):
        raise ValueError(f"{path} is not a valid PNG")

    offset = len(PNG_SIGNATURE)
    dimensions: tuple[int, int] | None = None
    compressed = bytearray()
    saw_end = False
    while offset < len(data):
        if len(data) - offset < 12:
            raise ValueError(f"{path} has a truncated PNG chunk")
        length = struct.unpack(">I", data[offset : offset + 4])[0]
        chunk_end = offset + 12 + length
        if chunk_end > len(data):
            raise ValueError(f"{path} has a truncated PNG payload")
        chunk_type = data[offset + 4 : offset + 8]
        payload = data[offset + 8 : offset + 8 + length]
        expected_crc = struct.unpack(">I", data[offset + 8 + length : chunk_end])[0]
        actual_crc = zlib.crc32(payload, zlib.crc32(chunk_type)) & 0xFFFF_FFFF
        if actual_crc != expected_crc:
            raise ValueError(f"{path} has an invalid PNG checksum")

        if chunk_type == b"IHDR":
            if dimensions is not None or offset != len(PNG_SIGNATURE) or length != 13:
                raise ValueError(f"{path} has an invalid PNG header")
            width, height, bit_depth, color_type, compression, filtering, interlace = (
                struct.unpack(">IIBBBBB", payload)
            )
            if (
                width == 0
                or height == 0
                or bit_depth != 8
                or color_type != 6
                or compression != 0
                or filtering != 0
                or interlace != 0
            ):
                raise ValueError(f"{path} is not a supported 8-bit RGBA PNG")
            dimensions = (width, height)
        elif chunk_type == b"IDAT":
            if len(compressed) + len(payload) > MAX_PNG_BYTES:
                raise ValueError(f"{path} exceeds the compressed PNG data limit")
            compressed.extend(payload)
        elif chunk_type == b"IEND":
            if length != 0:
                raise ValueError(f"{path} has an invalid PNG end marker")
            saw_end = True
            offset = chunk_end
            break
        offset = chunk_end

    if dimensions is None or not saw_end or offset != len(data) or not compressed:
        raise ValueError(f"{path} is not a complete PNG")

    decompressor = zlib.decompressobj()
    try:
        pixels = decompressor.decompress(bytes(compressed), MAX_DECODED_BYTES + 1)
    except zlib.error as error:
        raise ValueError(f"{path} has invalid compressed PNG data") from error
    if len(pixels) > MAX_DECODED_BYTES or decompressor.unconsumed_tail:
        raise ValueError(f"{path} exceeds the decoded PNG pixel budget")
    if not decompressor.eof or decompressor.unused_data:
        raise ValueError(f"{path} has invalid trailing PNG image data")
    width, height = dimensions
    row_length = 1 + width * 4
    expected_decoded_bytes = row_length * height
    if expected_decoded_bytes > MAX_DECODED_BYTES:
        raise ValueError(f"{path} exceeds the decoded PNG pixel budget")
    if len(pixels) != expected_decoded_bytes:
        raise ValueError(f"{path} has an invalid RGBA pixel count")
    if any(pixels[row * row_length] > 4 for row in range(height)):
        raise ValueError(f"{path} uses an invalid PNG row filter")
    return dimensions


def validate(*, check_hashes: bool = True, check_source_freshness: bool = True) -> None:
    """Require complete approved screenshots, dimensions, and README references."""

    readme = (REPOSITORY_ROOT / "README.md").read_text(encoding="utf-8")
    failures: list[str] = []
    if check_source_freshness:
        try:
            source_digest = screenshot_source_digest()
        except (OSError, ValueError) as error:
            failures.append(str(error))
        else:
            if source_digest != EXPECTED_SCREENSHOT_SOURCE_SHA256:
                failures.append(
                    "README screenshots were not approved for the current native build inputs"
                )
    for relative_path in SCREENSHOTS:
        path = REPOSITORY_ROOT / relative_path
        if not path.is_file():
            failures.append(f"missing screenshot: {relative_path.as_posix()}")
            continue
        try:
            dimensions = png_dimensions(path)
        except ValueError as error:
            failures.append(str(error))
            continue
        if dimensions != EXPECTED_SIZE:
            failures.append(
                f"{relative_path.as_posix()} is {dimensions[0]}x{dimensions[1]}, "
                f"expected {EXPECTED_SIZE[0]}x{EXPECTED_SIZE[1]}"
            )
        if path.stat().st_size < 20_000:
            failures.append(f"{relative_path.as_posix()} is implausibly small")
        if check_hashes:
            digest = hashlib.sha256(read_bounded_regular_file(path)).hexdigest()
            if digest != EXPECTED_SHA256[relative_path]:
                failures.append(
                    f"{relative_path.as_posix()} does not match its approved SHA-256"
                )
        if relative_path.as_posix() not in readme:
            failures.append(f"README.md does not reference {relative_path.as_posix()}")

    if failures:
        raise SystemExit("\n".join(failures))


if __name__ == "__main__":
    validate()
