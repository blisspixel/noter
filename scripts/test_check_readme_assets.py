"""Tests for the dependency-free README screenshot validator."""

from __future__ import annotations

import struct
import tempfile
import unittest
import zlib
from pathlib import Path

from check_readme_assets import MAX_DECODED_BYTES, PNG_SIGNATURE, png_dimensions
from update_readme_screenshots import validate_generated_screenshot


def png_chunk(chunk_type: bytes, payload: bytes) -> bytes:
    """Build one checksummed PNG chunk for a small test fixture."""

    checksum = zlib.crc32(payload, zlib.crc32(chunk_type)) & 0xFFFF_FFFF
    return (
        struct.pack(">I", len(payload))
        + chunk_type
        + payload
        + struct.pack(">I", checksum)
    )


def rgba_png(width: int, height: int, *, padding: int = 0) -> bytes:
    """Build a complete non-interlaced 8-bit RGBA PNG."""

    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    pixels = b"".join(b"\x00" + bytes(width * 4) for _ in range(height))
    chunks = PNG_SIGNATURE + png_chunk(b"IHDR", header)
    if padding:
        chunks += png_chunk(b"tEXt", bytes(padding))
    return chunks + png_chunk(b"IDAT", zlib.compress(pixels)) + png_chunk(b"IEND", b"")


class PngDimensionsTests(unittest.TestCase):
    def test_reads_dimensions_from_complete_png(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.png"
            path.write_bytes(rgba_png(2, 1))

            self.assertEqual(png_dimensions(path), (2, 1))

    def test_rejects_non_png_input(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.png"
            path.write_bytes(b"not a png")

            with self.assertRaisesRegex(ValueError, "not a valid PNG"):
                png_dimensions(path)

    def test_rejects_truncated_png(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.png"
            path.write_bytes(rgba_png(2, 1)[:-4])

            with self.assertRaisesRegex(ValueError, "truncated|complete"):
                png_dimensions(path)

    def test_rejects_bad_chunk_checksum(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.png"
            data = bytearray(rgba_png(2, 1))
            data[20] ^= 1
            path.write_bytes(data)

            with self.assertRaisesRegex(ValueError, "checksum"):
                png_dimensions(path)

    def test_rejects_compressed_data_that_exceeds_the_pixel_budget(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.png"
            header = struct.pack(">IIBBBBB", 1200, 760, 8, 6, 0, 0, 0)
            oversized_pixels = zlib.compress(bytes(MAX_DECODED_BYTES + 1))
            path.write_bytes(
                PNG_SIGNATURE
                + png_chunk(b"IHDR", header)
                + png_chunk(b"IDAT", oversized_pixels)
                + png_chunk(b"IEND", b"")
            )

            with self.assertRaisesRegex(ValueError, "decoded PNG pixel budget"):
                png_dimensions(path)


class GeneratedScreenshotTests(unittest.TestCase):
    def test_rejects_a_missing_fresh_render(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "missing.png"

            with self.assertRaisesRegex(RuntimeError, "did not create"):
                validate_generated_screenshot(path)

    def test_accepts_a_complete_fresh_readme_screenshot(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fresh.png"
            path.write_bytes(rgba_png(1200, 760, padding=20_000))

            validate_generated_screenshot(path)


if __name__ == "__main__":
    unittest.main()
