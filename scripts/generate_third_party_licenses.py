#!/usr/bin/env python3
"""Generate a deterministic, bounded third-party license inventory."""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import os
import re
import stat
import subprocess
import tempfile
from pathlib import Path, PurePosixPath
from typing import Any
from urllib.parse import urlsplit


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
MAX_JSON_BYTES = 16 * 1024 * 1024
MAX_OUTPUT_BYTES = 16 * 1024 * 1024
MAX_COMPONENTS = 4096
MAX_LICENSES = 4096
MAX_MAPPINGS = 16384
MAX_NOTICE_FILES = 16384
MAX_NOTICE_INPUT_BYTES = 16 * 1024 * 1024
MAX_SOURCE_ENTRIES = 131072
MAX_LICENSE_TEXT_BYTES = 2 * 1024 * 1024
WINDOWS_REPARSE_POINT_ATTRIBUTE = 0x400
LEGAL_FILE_PATTERN = re.compile(
    r"^(?:licen[cs]es?|copying|notices?|copyrights?|unlicense|authors?|credits?|patents?)"
    r"(?:$|[._-])",
    re.IGNORECASE,
)
LEGAL_NAME_TOKENS = frozenset(
    {
        "copyright",
        "copyrights",
        "licence",
        "licences",
        "license",
        "licenses",
        "notice",
        "notices",
    }
)
LEGAL_COMPACT_STEMS = frozenset({"ofl", "thirdpartynotice", "thirdpartynotices", "ufl"})
LEGAL_DIRECTORY_NAMES = frozenset(
    {"legal", "licence", "licences", "license", "licenses"}
)
NON_RUNTIME_DIRECTORY_NAMES = frozenset({"benches", "examples", "tests"})
FONT_FILE_SUFFIXES = frozenset({".eot", ".otf", ".ttc", ".ttf", ".woff", ".woff2"})
SOURCE_CODE_SUFFIXES = frozenset(
    {
        ".bat",
        ".c",
        ".cc",
        ".cmd",
        ".cpp",
        ".cs",
        ".cxx",
        ".go",
        ".h",
        ".hpp",
        ".java",
        ".js",
        ".jsx",
        ".kt",
        ".kts",
        ".lua",
        ".m",
        ".mm",
        ".php",
        ".pl",
        ".ps1",
        ".py",
        ".rb",
        ".rs",
        ".scala",
        ".sh",
        ".swift",
        ".ts",
        ".tsx",
        ".zig",
    }
)


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


def _is_packaged_legal_name(name: str, sibling_names: set[str]) -> bool:
    """Recognize conventional legal names and text sidecars for packaged fonts."""

    if LEGAL_FILE_PATTERN.match(name) is not None:
        return True
    path = Path(name)
    stem = path.stem.casefold()
    tokens = frozenset(filter(None, re.split(r"[^0-9a-z]+", stem)))
    if tokens & LEGAL_NAME_TOKENS or stem in LEGAL_COMPACT_STEMS:
        return True
    if path.suffix.casefold() != ".txt":
        return False
    return any(
        f"{path.stem}{suffix}".casefold() in sibling_names
        for suffix in FONT_FILE_SUFFIXES
    )


def _metadata_is_link_like(metadata: os.stat_result) -> bool:
    """Identify link metadata across POSIX and Windows filesystems."""

    return stat.S_ISLNK(metadata.st_mode) or bool(
        getattr(metadata, "st_file_attributes", 0) & WINDOWS_REPARSE_POINT_ATTRIBUTE
    )


def _is_link_like(path: Path) -> bool:
    """Identify a current symbolic link or Windows reparse point."""

    try:
        metadata = path.lstat()
    except OSError:
        return False
    return _metadata_is_link_like(metadata)


def _is_within(path: Path, root: Path) -> bool:
    """Return whether a resolved path remains below a resolved root."""

    try:
        path.relative_to(root)
    except ValueError:
        return False
    return True


def _metadata_fingerprint(metadata: os.stat_result) -> tuple[int, int, int]:
    """Return mutation-sensitive metadata for one already identified object."""

    return metadata.st_size, metadata.st_mtime_ns, metadata.st_ctime_ns


def _path_open_fingerprint(metadata: os.stat_result) -> tuple[int, ...]:
    """Return fields comparable between path and descriptor metadata."""

    fingerprint = (metadata.st_size, metadata.st_mtime_ns)
    if os.name == "nt":
        return fingerprint
    return (*fingerprint, metadata.st_ctime_ns)


def _read_descriptor_bound(
    path: Path,
    expected: os.stat_result,
    maximum_bytes: int,
    context: str,
    limit_name: str,
    parent_identity: tuple[Path, os.stat_result],
) -> bytes:
    """Read the checked object once without following a replacement final link."""

    flags = os.O_RDONLY
    flags |= getattr(os, "O_BINARY", 0)
    flags |= getattr(os, "O_CLOEXEC", 0)
    flags |= getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise InventoryError(f"{context} is unreadable") from error
    try:
        opened = os.fstat(descriptor)
        if (
            not stat.S_ISREG(opened.st_mode)
            or opened.st_nlink != 1
            or not os.path.samestat(expected, opened)
            or _path_open_fingerprint(expected) != _path_open_fingerprint(opened)
        ):
            raise InventoryError(f"{context} changed while it was being opened")
        chunks: list[bytes] = []
        remaining = maximum_bytes + 1
        while remaining > 0:
            chunk = os.read(descriptor, remaining)
            if not chunk:
                break
            chunks.append(chunk)
            remaining -= len(chunk)
        payload = b"".join(chunks)
        if len(payload) > maximum_bytes:
            raise InventoryError(f"{context} exceeds the {limit_name}")
        after_read = os.fstat(descriptor)
        if not os.path.samestat(opened, after_read) or _metadata_fingerprint(
            opened
        ) != _metadata_fingerprint(after_read):
            raise InventoryError(f"{context} changed while it was being read")
        try:
            current = path.lstat()
            parent_path, expected_parent = parent_identity
            current_parent = parent_path.lstat()
        except OSError as error:
            raise InventoryError(
                f"{context} changed while it was being read"
            ) from error
        if (
            not stat.S_ISREG(current.st_mode)
            or current.st_nlink != 1
            or _metadata_is_link_like(current)
            or not os.path.samestat(opened, current)
            or _path_open_fingerprint(current) != _path_open_fingerprint(after_read)
            or not stat.S_ISDIR(current_parent.st_mode)
            or _metadata_is_link_like(current_parent)
            or not os.path.samestat(expected_parent, current_parent)
        ):
            raise InventoryError(f"{context} changed while it was being read")
        return payload
    finally:
        os.close(descriptor)


def _load_bounded_json(path: Path) -> dict[str, Any]:
    metadata = path.lstat()
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > MAX_JSON_BYTES:
        raise InventoryError("cargo-about JSON is not a bounded regular file")
    parent_identity = (path.parent, path.parent.lstat())
    payload = _read_descriptor_bound(
        path,
        metadata,
        MAX_JSON_BYTES,
        "cargo-about JSON",
        "size limit",
        parent_identity,
    )
    try:
        parsed = json.loads(payload)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise InventoryError("cargo-about emitted invalid UTF-8 JSON") from error
    return _mapping(parsed, "cargo-about output")


def _collect_packaged_notices(evidence: dict[str, Any]) -> list[dict[str, Any]]:
    """Collect bounded legal files from locked third-party package sources."""

    component_records = _list(
        evidence.get("crates"), "cargo-about output.crates", MAX_COMPONENTS
    )
    notices: list[dict[str, Any]] = []
    total_bytes = 0
    visited_entries = 0
    for index, raw_record in enumerate(component_records):
        record = _mapping(raw_record, f"crates[{index}]")
        package = _mapping(record.get("package"), f"crates[{index}].package")
        identity = _package_identity(package, f"crates[{index}].package")
        source = package.get("source")
        if source is None:
            continue
        _text(source, f"crates[{index}].package.source", 4096)
        manifest_value = _text(
            package.get("manifest_path"),
            f"crates[{index}].package.manifest_path",
            4096,
        )
        manifest_path = Path(manifest_value)
        if not manifest_path.is_absolute() or _is_link_like(manifest_path):
            raise InventoryError(
                f"third-party manifest for {identity[0]} {identity[1]} "
                "must be an absolute regular file"
            )
        try:
            resolved_manifest = manifest_path.resolve(strict=True)
            metadata = resolved_manifest.stat()
        except OSError as error:
            raise InventoryError(
                f"third-party manifest for {identity[0]} {identity[1]} is unreadable"
            ) from error
        if resolved_manifest.name != "Cargo.toml" or not stat.S_ISREG(metadata.st_mode):
            raise InventoryError(
                f"third-party manifest for {identity[0]} {identity[1]} "
                "must be a Cargo.toml regular file"
            )
        package_root = resolved_manifest.parent
        explicit_notice_path: Path | None = None
        license_file = package.get("license_file")
        if license_file is not None:
            license_file_value = _text(
                license_file,
                f"crates[{index}].package.license_file",
                4096,
            )
            candidate = Path(license_file_value)
            if not candidate.is_absolute():
                candidate = package_root / candidate
            if _is_link_like(candidate):
                raise InventoryError(
                    f"explicit license file for {identity[0]} {identity[1]} "
                    "must be a regular file inside its package"
                )
            try:
                explicit_notice_path = candidate.resolve(strict=True)
            except OSError as error:
                raise InventoryError(
                    f"explicit license file for {identity[0]} {identity[1]} "
                    "is unreadable"
                ) from error
            if not _is_within(explicit_notice_path, package_root):
                raise InventoryError(
                    f"explicit license file for {identity[0]} {identity[1]} "
                    "must remain inside its package"
                )
        found_explicit_notice = False

        def raise_walk_error(error: OSError) -> None:
            raise InventoryError(
                f"third-party source tree for {identity[0]} {identity[1]} is unreadable"
            ) from error

        for directory, directory_names, file_names in os.walk(
            package_root,
            topdown=True,
            onerror=raise_walk_error,
            followlinks=False,
        ):
            directory_path = Path(directory)
            try:
                before_resolution = directory_path.lstat()
                if _metadata_is_link_like(before_resolution):
                    raise InventoryError(
                        f"third-party source tree for {identity[0]} {identity[1]} "
                        "contains a link-like directory"
                    )
                if not stat.S_ISDIR(before_resolution.st_mode):
                    raise InventoryError(
                        f"third-party source tree for {identity[0]} {identity[1]} "
                        "changed during traversal"
                    )
                resolved_directory = directory_path.resolve(strict=True)
                directory_metadata = directory_path.lstat()
            except OSError as error:
                raise InventoryError(
                    f"third-party source tree for {identity[0]} {identity[1]} "
                    "is unreadable"
                ) from error
            if _metadata_is_link_like(directory_metadata):
                raise InventoryError(
                    f"third-party source tree for {identity[0]} {identity[1]} "
                    "contains a link-like directory"
                )
            if not os.path.samestat(before_resolution, directory_metadata):
                raise InventoryError(
                    f"third-party source tree for {identity[0]} {identity[1]} "
                    "changed during traversal"
                )
            if not _is_within(resolved_directory, package_root):
                raise InventoryError(
                    f"third-party source tree for {identity[0]} {identity[1]} "
                    "contains a directory outside its package"
                )
            if not stat.S_ISDIR(directory_metadata.st_mode):
                raise InventoryError(
                    f"third-party source tree for {identity[0]} {identity[1]} "
                    "changed during traversal"
                )
            visited_entries += len(directory_names) + len(file_names)
            if visited_entries > MAX_SOURCE_ENTRIES:
                raise InventoryError(
                    "third-party source trees exceed the filesystem-entry limit"
                )
            relative_directory = directory_path.relative_to(package_root)
            inside_legal_directory = any(
                part.casefold() in LEGAL_DIRECTORY_NAMES
                for part in relative_directory.parts
            )
            inside_non_runtime_directory = any(
                part.casefold() in NON_RUNTIME_DIRECTORY_NAMES
                for part in relative_directory.parts
            )
            safe_directories: list[str] = []
            for name in sorted(directory_names):
                candidate = directory_path / name
                if _is_link_like(candidate):
                    raise InventoryError(
                        f"third-party source tree for {identity[0]} {identity[1]} "
                        "contains a link-like directory"
                    )
                safe_directories.append(name)
            directory_names[:] = safe_directories
            sibling_names = {name.casefold() for name in file_names}
            for name in sorted(file_names):
                notice_path = directory_path / name
                is_explicit_notice = notice_path == explicit_notice_path
                if not is_explicit_notice and (
                    inside_non_runtime_directory
                    or notice_path.suffix.casefold() in SOURCE_CODE_SUFFIXES
                    or (
                        not inside_legal_directory
                        and not _is_packaged_legal_name(name, sibling_names)
                    )
                ):
                    continue
                if is_explicit_notice:
                    found_explicit_notice = True
                try:
                    notice_metadata = notice_path.lstat()
                except OSError as error:
                    raise InventoryError(
                        f"packaged legal file for {identity[0]} {identity[1]} "
                        "is unreadable"
                    ) from error
                if not stat.S_ISREG(notice_metadata.st_mode) or _metadata_is_link_like(
                    notice_metadata
                ):
                    raise InventoryError(
                        f"packaged legal file for {identity[0]} {identity[1]} "
                        "must be regular"
                    )
                if notice_metadata.st_nlink != 1:
                    raise InventoryError(
                        f"packaged legal file for {identity[0]} {identity[1]} "
                        "must not be hard-linked"
                    )
                if notice_metadata.st_size > MAX_LICENSE_TEXT_BYTES:
                    raise InventoryError(
                        f"packaged legal file for {identity[0]} {identity[1]} "
                        "exceeds the per-file limit"
                    )
                context = f"packaged legal file for {identity[0]} {identity[1]}"
                payload = _read_descriptor_bound(
                    notice_path,
                    notice_metadata,
                    MAX_LICENSE_TEXT_BYTES,
                    context,
                    "per-file limit",
                    (directory_path, directory_metadata),
                )
                total_bytes += len(payload)
                if total_bytes > MAX_NOTICE_INPUT_BYTES:
                    raise InventoryError(
                        "packaged legal files exceed the total byte limit"
                    )
                try:
                    decoded = payload.decode("utf-8-sig")
                except UnicodeDecodeError as error:
                    raise InventoryError(
                        f"packaged legal file for {identity[0]} {identity[1]} "
                        "is not UTF-8"
                    ) from error
                relative_path = notice_path.relative_to(package_root).as_posix()
                notices.append(
                    {
                        "package": {"name": identity[0], "version": identity[1]},
                        "path": relative_path,
                        "text": _normalize_license_text(
                            decoded,
                            f"packaged legal file {relative_path}",
                        ),
                    }
                )
                if len(notices) > MAX_NOTICE_FILES:
                    raise InventoryError(
                        f"packaged legal files exceed the {MAX_NOTICE_FILES}-item limit"
                    )
        if explicit_notice_path is not None and not found_explicit_notice:
            raise InventoryError(
                f"explicit license file for {identity[0]} {identity[1]} "
                "was not a regular file in its package"
            )
    if not notices:
        raise InventoryError("no packaged third-party legal files were found")
    return notices


def _canonical_inventory(
    evidence: dict[str, Any],
    packaged_notices: Any,
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

    selected_texts: set[str] = set()
    names_by_identifier: dict[str, str] = {}
    mapped_components: set[tuple[str, str]] = set()
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
        selected_texts.add(license_text)
        used_by = _list(
            record.get("used_by"),
            f"licenses[{index}].used_by",
            MAX_MAPPINGS,
        )
        if not used_by:
            raise InventoryError(f"licenses[{index}] has no package mappings")
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
            mapped_components.add(identity)
            mapping_count += 1
            if mapping_count > MAX_MAPPINGS:
                raise InventoryError(
                    f"license mappings exceed the {MAX_MAPPINGS}-item limit"
                )

    missing = sorted(set(components) - mapped_components)
    if missing:
        preview = ", ".join(f"{name} {version}" for name, version in missing[:3])
        raise InventoryError(f"components without license text mappings: {preview}")

    notice_records = _list(packaged_notices, "packaged legal files", MAX_NOTICE_FILES)
    if not notice_records:
        raise InventoryError("packaged legal files must not be empty")
    notice_sources: dict[str, set[tuple[str, str, str]]] = {}
    for index, raw_record in enumerate(notice_records):
        record = _mapping(raw_record, f"packaged legal files[{index}]")
        package = _mapping(
            record.get("package"), f"packaged legal files[{index}].package"
        )
        identity = _package_identity(package, f"packaged legal files[{index}].package")
        if identity not in components:
            raise InventoryError(
                f"packaged legal file references unknown component "
                f"{identity[0]} {identity[1]}"
            )
        relative_path = _text(
            record.get("path"), f"packaged legal files[{index}].path", 4096
        )
        canonical_path = PurePosixPath(relative_path)
        if (
            canonical_path.is_absolute()
            or "\\" in relative_path
            or canonical_path.as_posix() != relative_path
            or any(part in {"", ".", ".."} for part in canonical_path.parts)
        ):
            raise InventoryError(
                f"packaged legal files[{index}].path must be canonical and relative"
            )
        license_text = _normalize_license_text(
            record.get("text"), f"packaged legal files[{index}].text"
        )
        notice_sources.setdefault(license_text, set()).add(
            (identity[0], identity[1], relative_path)
        )

    ordered_components = sorted(
        components.values(),
        key=lambda item: (
            str(item["name"]).casefold(),
            str(item["name"]),
            str(item["version"]),
        ),
    )
    ordered_texts: list[dict[str, Any]] = []
    for license_text in selected_texts | set(notice_sources):
        ordered_texts.append(
            {
                "text": license_text,
                "sources": sorted(
                    notice_sources.get(license_text, set()),
                    key=lambda item: (
                        item[0].casefold(),
                        item[0],
                        item[1],
                        item[2].casefold(),
                        item[2],
                    ),
                ),
            }
        )
    ordered_texts.sort(
        key=lambda item: (
            hashlib.sha256(item["text"].encode("utf-8")).hexdigest(),
            item["text"],
        )
    )
    return ordered_components, ordered_texts


def render_inventory(evidence: Any, packaged_notices: Any) -> str:
    """Render validated cargo-about evidence and packaged legal files."""

    evidence = _mapping(evidence, "cargo-about output")
    components, legal_texts = _canonical_inventory(evidence, packaged_notices)
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
        "      This document lists the locked Rust packages included in Noter, their",
        "      declared license expressions, and distinct legal texts selected from or",
        "      packaged with those sources. The bundled Inter typeface is covered",
        "      separately by <code>Inter-OFL.txt</code>.",
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
            "      Declared SPDX expressions and preserved texts are intentionally separate.",
            "      Copyright and notice text can carry information that an SPDX identifier",
            "      does not. Text is preserved verbatim apart from UTF-8 BOM removal and",
            "      line-ending normalization.",
            "    </p>",
        ]
    )
    for index, legal_text in enumerate(legal_texts):
        lines.append('      <section class="license-text">')
        lines.append(f'        <h3 id="license-text-{index}">Distinct legal text</h3>')
        sources = legal_text["sources"]
        if sources:
            lines.extend(["        <p>Packaged in:</p>", "        <ul>"])
            for package_name, package_version, relative_path in sources:
                lines.append(
                    '          <li class="notice-source">'
                    f"{html.escape(package_name)} {html.escape(package_version)}: "
                    f"<code>{html.escape(relative_path)}</code></li>"
                )
            lines.append("        </ul>")
        else:
            lines.append(
                '        <p class="metadata-license">Selected by cargo-about from '
                "the locked dependency metadata.</p>"
            )
        lines.extend(
            [
                f"        <pre>{html.escape(str(legal_text['text']))}</pre>",
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
    packaged_notices = _collect_packaged_notices(evidence)
    rendered = render_inventory(evidence, packaged_notices)
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
