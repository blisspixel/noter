#!/usr/bin/env python3
"""Generate a deterministic, bounded third-party license inventory."""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import os
import stat
import subprocess
import tempfile
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
MAX_JSON_BYTES = 16 * 1024 * 1024
MAX_OUTPUT_BYTES = 16 * 1024 * 1024
MAX_COMPONENTS = 4096
MAX_LICENSES = 4096
MAX_MAPPINGS = 16384
MAX_LICENSE_TEXT_BYTES = 2 * 1024 * 1024


class InventoryError(ValueError):
    """Report malformed or incomplete cargo-about evidence."""


def _mapping(value: Any, context: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise InventoryError(f"{context} must be an object")
    return value


def _list(value: Any, context: str, maximum: int) -> list[Any]:
    if not isinstance(value, list):
        raise InventoryError(f"{context} must be an array")
    if len(value) > maximum:
        raise InventoryError(f"{context} exceeds the {maximum}-item limit")
    return value


def _text(value: Any, context: str, maximum_bytes: int = 4096) -> str:
    if not isinstance(value, str) or not value:
        raise InventoryError(f"{context} must be a non-empty string")
    if len(value.encode("utf-8")) > maximum_bytes:
        raise InventoryError(f"{context} exceeds the {maximum_bytes}-byte limit")
    if "\x00" in value:
        raise InventoryError(f"{context} contains a null byte")
    return value


def _package_identity(package: dict[str, Any], context: str) -> tuple[str, str]:
    return (
        _text(package.get("name"), f"{context}.name", 256),
        _text(package.get("version"), f"{context}.version", 128),
    )


def _safe_repository(value: Any) -> str | None:
    """Return a link only when registry metadata contains a safe public URL."""

    if value is None:
        return None
    if not isinstance(value, str) or len(value.encode("utf-8")) > 4096:
        return None
    if any(character.isspace() or ord(character) < 0x20 for character in value):
        return None
    try:
        parsed = urlsplit(value)
        if (
            parsed.scheme not in {"http", "https"}
            or not parsed.netloc
            or parsed.username is not None
            or parsed.password is not None
        ):
            return None
    except ValueError:
        return None
    return value


def _normalize_license_text(value: Any, context: str) -> str:
    text = _text(value, context, MAX_LICENSE_TEXT_BYTES)
    return text.replace("\r\n", "\n").replace("\r", "\n")


def _load_bounded_json(path: Path) -> dict[str, Any]:
    metadata = path.lstat()
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > MAX_JSON_BYTES:
        raise InventoryError("cargo-about JSON is not a bounded regular file")
    payload = path.read_bytes()
    if len(payload) > MAX_JSON_BYTES:
        raise InventoryError("cargo-about JSON exceeds the size limit")
    try:
        parsed = json.loads(payload)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise InventoryError("cargo-about emitted invalid UTF-8 JSON") from error
    return _mapping(parsed, "cargo-about output")


def _canonical_inventory(
    evidence: dict[str, Any],
) -> tuple[list[dict[str, str | None]], list[dict[str, Any]]]:
    component_records = _list(
        evidence.get("crates"), "cargo-about output.crates", MAX_COMPONENTS
    )
    license_records = _list(
        evidence.get("licenses"), "cargo-about output.licenses", MAX_LICENSES
    )
    if not component_records or not license_records:
        raise InventoryError("cargo-about inventory must not be empty")

    components: dict[tuple[str, str], dict[str, str | None]] = {}
    for index, raw_record in enumerate(component_records):
        record = _mapping(raw_record, f"crates[{index}]")
        package = _mapping(record.get("package"), f"crates[{index}].package")
        identity = _package_identity(package, f"crates[{index}].package")
        if identity in components:
            raise InventoryError(
                f"duplicate component identity {identity[0]} {identity[1]}"
            )
        components[identity] = {
            "name": identity[0],
            "version": identity[1],
            "license": _text(record.get("license"), f"crates[{index}].license", 512),
            "repository": _safe_repository(package.get("repository")),
        }

    grouped_licenses: dict[tuple[str, str, str], set[tuple[str, str]]] = {}
    names_by_identifier: dict[str, str] = {}
    mapping_count = 0
    for index, raw_record in enumerate(license_records):
        record = _mapping(raw_record, f"licenses[{index}]")
        identifier = _text(record.get("id"), f"licenses[{index}].id", 256)
        name = _text(record.get("name"), f"licenses[{index}].name", 512)
        previous_name = names_by_identifier.setdefault(identifier, name)
        if previous_name != name:
            raise InventoryError(
                f"license identifier {identifier} has inconsistent display names"
            )
        license_text = _normalize_license_text(
            record.get("text"), f"licenses[{index}].text"
        )
        used_by = _list(
            record.get("used_by"),
            f"licenses[{index}].used_by",
            MAX_MAPPINGS,
        )
        if not used_by:
            raise InventoryError(f"licenses[{index}] has no package mappings")
        mappings = grouped_licenses.setdefault((identifier, name, license_text), set())
        for used_index, raw_usage in enumerate(used_by):
            usage = _mapping(raw_usage, f"licenses[{index}].used_by[{used_index}]")
            package = _mapping(
                usage.get("crate"),
                f"licenses[{index}].used_by[{used_index}].crate",
            )
            identity = _package_identity(
                package, f"licenses[{index}].used_by[{used_index}].crate"
            )
            if identity not in components:
                raise InventoryError(
                    f"license mapping references unknown component {identity[0]} "
                    f"{identity[1]}"
                )
            mappings.add(identity)
            mapping_count += 1
            if mapping_count > MAX_MAPPINGS:
                raise InventoryError(
                    f"license mappings exceed the {MAX_MAPPINGS}-item limit"
                )

    mapped_components = {
        identity for mappings in grouped_licenses.values() for identity in mappings
    }
    missing = sorted(set(components) - mapped_components)
    if missing:
        preview = ", ".join(f"{name} {version}" for name, version in missing[:3])
        raise InventoryError(f"components without license text mappings: {preview}")

    ordered_components = sorted(
        components.values(),
        key=lambda item: (
            str(item["name"]).casefold(),
            str(item["name"]),
            str(item["version"]),
        ),
    )
    ordered_licenses: list[dict[str, Any]] = []
    for (identifier, name, license_text), mappings in grouped_licenses.items():
        ordered_licenses.append(
            {
                "id": identifier,
                "name": name,
                "text": license_text,
                "used_by": sorted(
                    mappings,
                    key=lambda item: (
                        item[0].casefold(),
                        item[0],
                        item[1],
                    ),
                ),
            }
        )
    ordered_licenses.sort(
        key=lambda item: (
            item["id"].casefold(),
            item["id"],
            hashlib.sha256(item["text"].encode("utf-8")).hexdigest(),
            item["text"],
        )
    )
    return ordered_components, ordered_licenses


def render_inventory(evidence: Any) -> str:
    """Render cargo-about JSON after canonical validation and ordering."""

    evidence = _mapping(evidence, "cargo-about output")
    components, licenses = _canonical_inventory(evidence)
    lines = [
        "<!doctype html>",
        '<html lang="en">',
        "<head>",
        '  <meta charset="utf-8">',
        '  <meta name="viewport" content="width=device-width, initial-scale=1">',
        "  <title>Noter third-party licenses</title>",
        "  <style>",
        "    body { font: 16px/1.5 system-ui, sans-serif; margin: 2rem auto; max-width: 72rem; padding: 0 1rem; }",
        "    table { border-collapse: collapse; width: 100%; }",
        "    th, td { border-bottom: 1px solid #aaa; padding: 0.4rem; text-align: left; vertical-align: top; }",
        "    pre { overflow-wrap: anywhere; white-space: pre-wrap; }",
        "  </style>",
        "</head>",
        "<body>",
        "  <main>",
        "    <h1>Noter third-party licenses</h1>",
        "    <p>",
        "      This document lists the Rust packages included in Noter and the license",
        "      text selected for each package's declared license expression. The bundled",
        "      Inter typeface is covered separately by <code>Inter-OFL.txt</code>.",
        "    </p>",
        "    <h2>Components</h2>",
        "    <table>",
        "      <thead><tr><th>Package</th><th>Version</th><th>License</th></tr></thead>",
        "      <tbody>",
    ]
    for component in components:
        name = html.escape(str(component["name"]), quote=True)
        repository = component["repository"]
        if repository is not None:
            name = f'<a href="{html.escape(repository, quote=True)}">{name}</a>'
        lines.extend(
            [
                "        <tr>",
                f"          <td>{name}</td>",
                f"          <td>{html.escape(str(component['version']))}</td>",
                f"          <td>{html.escape(str(component['license']))}</td>",
                "        </tr>",
            ]
        )
    lines.extend(
        [
            "      </tbody>",
            "    </table>",
            "    <h2>License texts</h2>",
            "    <p>",
            "      Distinct source texts are kept separately so package-specific notices are",
            "      not collapsed merely because packages share an SPDX identifier.",
            "    </p>",
        ]
    )
    previous_identifier: str | None = None
    for index, license_record in enumerate(licenses):
        identifier = str(license_record["id"])
        lines.append('      <section class="license-text">')
        if identifier != previous_identifier:
            lines.append(
                f'        <h3 id="license-group-{index}">'
                f"{html.escape(str(license_record['name']))}</h3>"
            )
            previous_identifier = identifier
        lines.extend(
            [
                f'        <h4 id="license-text-{index}">Distinct source text</h4>',
                "        <p>Used by:</p>",
                "        <ul>",
            ]
        )
        for package_name, package_version in license_record["used_by"]:
            lines.append(
                '          <li class="license-user">'
                f"{html.escape(package_name)} {html.escape(package_version)}</li>"
            )
        lines.extend(
            [
                "        </ul>",
                f"        <pre>{html.escape(str(license_record['text']))}</pre>",
                "      </section>",
            ]
        )
    lines.extend(["  </main>", "</body>", "</html>"])
    rendered = "\n".join(lines) + "\n"
    if len(rendered.encode("utf-8")) > MAX_OUTPUT_BYTES:
        raise InventoryError("rendered license inventory exceeds the size limit")
    return rendered


def _write_atomically(output: Path, rendered: str) -> None:
    parent = output.parent.resolve(strict=True)
    if not parent.is_dir():
        raise InventoryError("license inventory parent is not a directory")
    replacement_mode = 0o644
    if output.exists() or output.is_symlink():
        metadata = output.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
            raise InventoryError("license inventory output must be a regular file")
        replacement_mode = stat.S_IMODE(metadata.st_mode)
    temporary_path: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w",
            encoding="utf-8",
            newline="\n",
            prefix=f".{output.name}.",
            suffix=".tmp",
            dir=parent,
            delete=False,
        ) as stream:
            temporary_path = Path(stream.name)
            stream.write(rendered)
            stream.flush()
            os.fsync(stream.fileno())
        os.chmod(temporary_path, replacement_mode)
        os.replace(temporary_path, output)
        temporary_path = None
    finally:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)


def generate_inventory(output: Path) -> str:
    """Run the pinned cargo-about installation and write canonical HTML."""

    with tempfile.TemporaryDirectory(prefix="noter-license-") as temporary:
        evidence_path = Path(temporary) / "cargo-about.json"
        subprocess.run(
            [
                "cargo",
                "about",
                "generate",
                "--format",
                "json",
                "--output-file",
                str(evidence_path),
                "--workspace",
                "--all-features",
                "--frozen",
                "--fail",
            ],
            cwd=REPOSITORY_ROOT,
            check=True,
            shell=False,
        )
        evidence = _load_bounded_json(evidence_path)
    rendered = render_inventory(evidence)
    _write_atomically(output, rendered)
    return hashlib.sha256(rendered.encode("utf-8")).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate Noter's deterministic third-party license inventory."
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=REPOSITORY_ROOT / "THIRD-PARTY-LICENSES.html",
        help="HTML file to replace atomically",
    )
    arguments = parser.parse_args()
    try:
        digest = generate_inventory(arguments.output)
    except (InventoryError, OSError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"license inventory generation failed: {error}") from error
    print(f"Generated {arguments.output} (SHA-256 {digest})")


if __name__ == "__main__":
    main()
