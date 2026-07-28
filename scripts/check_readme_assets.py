#!/usr/bin/env python3
"""Validate tracked README screenshots without requiring a graphical session."""

from __future__ import annotations

import hashlib
import struct
import zlib
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
SCREENSHOTS = (
    Path("docs/assets/noter-light.png"),
    Path("docs/assets/noter-dark.png"),
)
EXPECTED_SIZE = (1200, 760)
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
EXPECTED_SHA256 = {
    Path(
        "docs/assets/noter-light.png"
    ): "bd105e3f50a6a1d48962745b8cf034fe8e3b763f80bcbfcf5d17d26b5a1f6176",
    Path(
        "docs/assets/noter-dark.png"
    ): "83a5540be26475a43d0bf9beddec6a506bec97da4d846833fcc894e2633dfe18",
}


def png_dimensions(path: Path) -> tuple[int, int]:
    """Fully validate an 8-bit RGBA PNG and return its dimensions."""

    data = path.read_bytes()
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
        pixels = decompressor.decompress(bytes(compressed)) + decompressor.flush()
    except zlib.error as error:
        raise ValueError(f"{path} has invalid compressed PNG data") from error
    if not decompressor.eof or decompressor.unused_data or decompressor.unconsumed_tail:
        raise ValueError(f"{path} has invalid trailing PNG image data")
    width, height = dimensions
    row_length = 1 + width * 4
    if len(pixels) != row_length * height:
        raise ValueError(f"{path} has an invalid RGBA pixel count")
    if any(pixels[row * row_length] > 4 for row in range(height)):
        raise ValueError(f"{path} uses an invalid PNG row filter")
    return dimensions


def validate(*, check_hashes: bool = True) -> None:
    """Require complete approved screenshots, dimensions, and README references."""

    readme = (REPOSITORY_ROOT / "README.md").read_text(encoding="utf-8")
    failures: list[str] = []
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
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
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
