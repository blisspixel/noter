#!/usr/bin/env python3
"""Validate Noter's security-hardened release configuration."""

from __future__ import annotations

import os
import re
import stat
import tomllib
import xml.etree.ElementTree as element_tree
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
MAX_CONFIG_BYTES = 2 * 1024 * 1024
UPGRADE_GUID = "04EFB570-9D04-49E1-B908-F5CEBD979E1B"
PATH_GUID = "495D73C5-0B48-41F9-A34D-30DAFD03BB54"
APPROVED_RELEASE_TARGETS = {
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
}
APPROVED_SBOM_ARTIFACTS = {
    f"noter_{target}.cdx.xml" for target in APPROVED_RELEASE_TARGETS
}
PINNED_INSTALLERS = {
    "b657cf8c04a8b7bc28f39d220f7e6dd11bbd2bdb072c552262bd9ccf597261b5",
    "a3435e9944f1a1297add11c6a8ac1f543c14a5ea88879ee05b24ff8218d46d87",
    "ffaafe99087affa66e3202ebb78fb0aeea6c6ff4019f941c82040502442f770a",
    "2cb2764f6f5e339a1dee20051c02f53b6ed712ef3a79aca2de495762108cdb64",
    "fb8dbee9f182173e062a64a387b21a0badc6fab8b2abf9294973f012972bf6d8",
    "6ac824e1642d6f7277d0ed7ea09411a508f6116ba6fae0aa5f2c7daa2ff43d31",
}
PINNED_ACTION = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")
WIX_NAMESPACE = {"wix": "http://schemas.microsoft.com/wix/2006/wi"}


def read_regular_file(path: Path) -> bytes:
    """Read a bounded regular file without accepting a repository symlink."""

    metadata = path.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise ValueError(f"{path} is not a regular file")
    if metadata.st_size > MAX_CONFIG_BYTES:
        raise ValueError(f"{path} exceeds the release-config size limit")
    with path.open("rb") as stream:
        if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
            raise ValueError(f"{path} is not a regular file")
        contents = stream.read(MAX_CONFIG_BYTES + 1)
    if len(contents) > MAX_CONFIG_BYTES:
        raise ValueError(f"{path} exceeds the release-config size limit")
    return contents


def require_text(text: str, expected: str, description: str, errors: list[str]) -> None:
    """Record a missing release invariant."""

    if expected not in text:
        errors.append(f"missing {description}")


def validate_manifest(text: str) -> list[str]:
    """Validate cargo-dist and MSI metadata that must remain stable."""

    errors: list[str] = []
    requirements = {
        'cargo-dist-version = "0.32.0"': "pinned cargo-dist version",
        'version = "0.1.0-alpha.1"': "prerelease package version",
        'installers = ["shell", "powershell", "homebrew", "msi"]': (
            "cross-platform installer set"
        ),
        'upgrade-guid = "04EFB570-9D04-49E1-B908-F5CEBD979E1B"': (
            "stable MSI upgrade GUID"
        ),
        'path-guid = "495D73C5-0B48-41F9-A34D-30DAFD03BB54"': ("stable MSI PATH GUID"),
        'license = "wix/License.rtf"': "MSI license sidecar",
        'allow-dirty = ["ci", "msi"]': "reviewed generated-file override",
        '"*.tar.gz"': "source-archive attestation filter",
        "github-attestations = true": "GitHub artifact attestations",
        "cargo-auditable = true": "auditable release binaries",
        "cargo-cyclonedx = false": "manifest-driven target-specific SBOM policy",
        "dispatch-releases = true": "explicit release dispatch",
    }
    for expected, description in requirements.items():
        require_text(text, expected, description, errors)
    if text.count("allow-dirty =") != 1:
        errors.append("release generation must have exactly one allow-dirty policy")
    try:
        manifest = tomllib.loads(text)
        dist = manifest["workspace"]["metadata"]["dist"]
        package_dist = manifest["package"]["metadata"]["dist"]
    except (tomllib.TOMLDecodeError, KeyError, TypeError):
        errors.append(
            "release metadata must be valid workspace cargo-dist configuration"
        )
    else:
        expected_notices = [
            "THIRD-PARTY-LICENSES.html",
            "assets/fonts/Inter-OFL.txt",
        ]
        if dist.get("include") != expected_notices:
            errors.append("missing archive license payload")
        targets = dist.get("targets")
        if (
            not isinstance(targets, list)
            or len(targets) != len(APPROVED_RELEASE_TARGETS)
            or set(targets) != APPROVED_RELEASE_TARGETS
        ):
            errors.append("cargo-dist targets differ from the approved release set")
        if dist.get("cargo-cyclonedx") is not False:
            errors.append("cargo-dist generic SBOM generation must remain disabled")
        extra_artifacts = package_dist.get("extra-artifacts")
        if not isinstance(extra_artifacts, list) or len(extra_artifacts) != 1:
            errors.append("release must declare exactly one target-specific SBOM set")
        else:
            sbom_entry = extra_artifacts[0]
            artifacts = sbom_entry.get("artifacts")
            if (
                not isinstance(artifacts, list)
                or len(artifacts) != len(APPROVED_SBOM_ARTIFACTS)
                or set(artifacts) != APPROVED_SBOM_ARTIFACTS
            ):
                errors.append("cargo-dist manifest does not list the exact SBOM set")
            if sbom_entry.get("build") != [
                "python",
                "scripts/generate_release_sboms.py",
            ]:
                errors.append("cargo-dist does not use the reviewed SBOM generator")
    return errors


def validate_workflow(text: str) -> list[str]:
    """Validate least privilege, pinned bootstraps, and artifact coverage."""

    errors: list[str] = []
    jobs_marker = text.find("\njobs:\n")
    host_marker = text.find("\n  host:\n")
    if jobs_marker < 0 or host_marker < 0:
        return ["release workflow is missing its jobs or host boundary"]

    header = text[:jobs_marker]
    pre_host = text[:host_marker]
    host = text[host_marker:]
    require_text(
        header,
        'permissions:\n  "contents": "read"',
        "read-only default permissions",
        errors,
    )
    if '"contents": "write"' in pre_host:
        errors.append("a pre-host release job has contents write permission")
    if "GH_TOKEN:" in pre_host:
        errors.append("a pre-host release job receives the GitHub write token")
    if text.count("GH_TOKEN:") != 1:
        errors.append("the GitHub token must appear only once in the host job")
    for expected, description in {
        '"attestations": "write"': "host attestation permission",
        '"contents": "write"': "host release permission",
        '"id-token": "write"': "host OIDC permission",
        "--steps=create --steps=upload --steps=release": "host publication stages",
        "steps.cargo-dist.outputs.paths": "manifest-driven artifact upload output",
        "artifacts/*.tar.gz": "source-archive attestation subject",
        "wix3141rtm/wix314-binaries.zip": "pinned WiX Toolset archive",
        "3.14.1.8722": "verified WiX Toolset version",
        "cargo-cyclonedx-0.5.9": "current pinned CycloneDX generator",
        'exec shasum -a 256 "$@"': "macOS archive-checksum compatibility shim",
        "^v[0-9]+\\.[0-9]+\\.[0-9]+-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*$": (
            "prerelease-only tag validation"
        ),
        '[ "$GITHUB_REF" != "refs/heads/main" ]': "protected-main publication gate",
        "git fetch --no-tags --depth=1 origin main": "fresh main-tip fetch",
        '[ "$GITHUB_SHA" != "$(git rev-parse origin/main)" ]': (
            "initial exact main-tip publication gate"
        ),
        '[ "$RELEASE_COMMIT" != "$(git rev-parse origin/main)" ]': (
            "final exact main-tip publication gate"
        ),
        'git ls-remote --exit-code --refs origin "refs/tags/$RELEASE_TAG"': (
            "remote release-tag inspection"
        ),
        'gh api --method POST "repos/$GITHUB_REPOSITORY/git/refs"': (
            "atomic release-tag creation"
        ),
        '-f ref="refs/tags/$RELEASE_TAG" -f sha="$RELEASE_COMMIT"': (
            "release-tag commit binding"
        ),
        "--verify-tag": "verified-tag release creation",
        "gh attestation verify PATH_TO_DOWNLOADED_ASSET": (
            "attestation-first release guidance"
        ),
        "--prerelease": "fail-closed prerelease publication",
    }.items():
        require_text(
            host if description.startswith("host ") else text,
            expected,
            description,
            errors,
        )
    forbidden = {
        "steps.cargo-cyclonedx.": "unmanifested manual SBOM output",
        "steps.cargo-cyclonedx.output.paths": "singular SBOM output reference",
        "matrix.install_dist": "unverified matrix cargo-dist installer",
        "matrix.install_cargo_auditable": "unverified matrix cargo-auditable installer",
        "releases/latest/download": "floating release-tool download",
        "announcement_github_body": "unreviewed generated release notes",
        "brew install noter": "unverified Homebrew installation guidance",
        '--target "$RELEASE_COMMIT"': "target-commit hint for a possibly existing tag",
    }
    for token, description in forbidden.items():
        if token in text:
            errors.append(f"release workflow contains {description}")
    if re.search(r"\|\s*(?:sh|bash)(?:\s|$)", text, flags=re.MULTILINE):
        errors.append("release workflow contains piped shell installer")
    if re.search(r"\|\s*iex(?:\s|$)", text, flags=re.IGNORECASE | re.MULTILINE):
        errors.append("release workflow contains piped PowerShell installer")
    if text.count('git rev-parse "$RELEASE_TAG^{commit}"') != 2:
        errors.append("release tag must be verified before and after atomic creation")
    if text.count("git fetch --no-tags --depth=1 origin main") != 2:
        errors.append(
            "main must be refreshed both before hosting and before publication"
        )
    if text.count('[ "$GITHUB_SHA" != "$(git rev-parse origin/main)" ]') != 1:
        errors.append("initial main-tip gate must appear exactly once")
    if text.count('[ "$RELEASE_COMMIT" != "$(git rev-parse origin/main)" ]') != 1:
        errors.append("final main-tip gate must appear exactly once")

    for digest in PINNED_INSTALLERS:
        require_text(text, digest, f"pinned installer digest {digest[:12]}", errors)

    uses = re.findall(r"^\s*-\s+uses:\s+([^\s#]+)", text, flags=re.MULTILINE)
    if not uses:
        errors.append("release workflow contains no reusable actions")
    for action in uses:
        if not action.startswith("./") and PINNED_ACTION.fullmatch(action) is None:
            errors.append(f"release workflow action is not commit-pinned: {action}")
    return errors


def validate_sbom_generator(text: str) -> list[str]:
    """Validate the bounded generator behind cargo-dist's declared SBOM artifacts."""

    errors: list[str] = []
    for expected, description in {
        '"GITHUB_SHA"': "immutable CI revision selection",
        '["git", "show", "-s", "--format=%ct", revision]': (
            "commit-derived SBOM timestamp"
        ),
        'environment["SOURCE_DATE_EPOCH"]': "reproducible SBOM timestamp",
        '"cyclonedx"': "CycloneDX generator command",
        '"--target-in-filename"': "target-qualified SBOM names",
        '"--license-strict"': "strict SBOM license parsing",
        '"MIT/Apache-2.0"': "reviewed upstream slash-license exception",
        '"MIT / Apache-2.0"': "reviewed spaced slash-license exception",
        '"Apache-2.0/MIT"': "reviewed reverse slash-license exception",
        '"--spec-version"': "explicit CycloneDX specification version",
        '"--no-build-deps"': "runtime-only SBOM policy",
        "MAX_SBOM_BYTES": "bounded SBOM output",
        "stat.S_ISREG": "regular-file SBOM policy",
        '"{http://cyclonedx.org/schema/bom/1.5}bom"': (
            "CycloneDX 1.5 namespace validation"
        ),
    }.items():
        require_text(text, expected, description, errors)
    for target in APPROVED_RELEASE_TARGETS:
        require_text(text, f'"{target}"', f"SBOM target {target}", errors)
    if "shell=True" in text:
        errors.append("SBOM generator must not invoke a command shell")
    if re.search(r"\b(?:curl|wget)\b", text):
        errors.append("SBOM generator must not fetch remote content")
    return errors


def validate_wix(contents: bytes) -> list[str]:
    """Validate that the privileged MSI installs only below protected Program Files."""

    errors: list[str] = []
    try:
        root = element_tree.fromstring(contents)
    except element_tree.ParseError as error:
        return [f"wix/main.wxs is not valid XML: {error}"]

    product = root.find("wix:Product", WIX_NAMESPACE)
    package = root.find(".//wix:Package", WIX_NAMESPACE)
    application = root.find(".//wix:Directory[@Id='APPLICATIONFOLDER']", WIX_NAMESPACE)
    feature = root.find(".//wix:Feature[@Id='Binaries']", WIX_NAMESPACE)
    major_upgrade = root.find(".//wix:MajorUpgrade", WIX_NAMESPACE)
    display_version = root.find(
        ".//wix:Property[@Id='ARPDisplayVersion']", WIX_NAMESPACE
    )
    path_component = root.find(".//wix:Component[@Id='Path']", WIX_NAMESPACE)
    environment = root.find(".//wix:Environment[@Id='PATH']", WIX_NAMESPACE)
    license_file = root.find(".//wix:File[@Id='LicenseFile']", WIX_NAMESPACE)
    third_party_file = root.find(
        ".//wix:File[@Id='ThirdPartyLicensesFile']", WIX_NAMESPACE
    )
    inter_license_file = root.find(".//wix:File[@Id='InterLicenseFile']", WIX_NAMESPACE)
    required_nodes = {
        "Product": product,
        "Package": package,
        "APPLICATIONFOLDER": application,
        "Binaries feature": feature,
        "major-upgrade policy": major_upgrade,
        "user-facing display version": display_version,
        "PATH component": path_component,
        "PATH environment entry": environment,
        "license sidecar": license_file,
        "third-party license inventory": third_party_file,
        "Inter license": inter_license_file,
    }
    for description, node in required_nodes.items():
        if node is None:
            errors.append(f"MSI is missing {description}")
    if errors:
        return errors

    assert product is not None
    assert package is not None
    assert application is not None
    assert feature is not None
    assert major_upgrade is not None
    assert display_version is not None
    assert path_component is not None
    assert environment is not None
    assert license_file is not None
    assert third_party_file is not None
    assert inter_license_file is not None
    if product.get("UpgradeCode") != UPGRADE_GUID:
        errors.append("MSI upgrade GUID differs from its permanent product identity")
    if product.get("Id") != "*" or package.get("Id") != "*":
        errors.append(
            "MSI product and package identities must rotate per release build"
        )
    if product.get("Version") != "0.0.1":
        errors.append("MSI package version differs from its monotonic release sequence")
    if display_version.get("Value") != "0.1.0-alpha.1":
        errors.append("MSI display version differs from the Noter package version")
    if major_upgrade.get("AllowSameVersionUpgrades") is not None:
        errors.append("MSI must not permit ambiguous same-version upgrades")
    if not major_upgrade.get("DowngradeErrorMessage"):
        errors.append("MSI does not refuse package-version downgrades")
    if major_upgrade.get("Schedule") != "afterInstallInitialize":
        errors.append("MSI major upgrade is not scheduled at the reviewed commit point")
    if path_component.get("Guid") != PATH_GUID:
        errors.append("MSI PATH component GUID differs from its permanent identity")
    if package.get("InstallScope") != "perMachine":
        errors.append(
            "MSI install scope changed without a matching directory and PATH review"
        )
    if feature.get("ConfigurableDirectory") is not None:
        errors.append(
            "per-machine MSI must not expose a user-configurable install directory"
        )
    if application.get("Name") != "Noter":
        errors.append("MSI application directory changed unexpectedly")
    parent_map = {child: parent for parent in root.iter() for child in parent}
    application_parent = parent_map.get(application)
    if (
        application_parent is None
        or application_parent.get("Id") != "$(var.PlatformProgramFilesFolder)"
    ):
        errors.append("per-machine MSI is not rooted below protected Program Files")
    if environment.get("System") != "yes" or environment.get("Value") != "[Bin]":
        errors.append("MSI PATH entry changed without an install-scope review")
    if license_file.get("Source") != "wix/License.rtf":
        errors.append("MSI does not embed the reviewed Apache-2.0 license sidecar")
    if third_party_file.get("Source") != "THIRD-PARTY-LICENSES.html":
        errors.append("MSI does not embed the generated third-party license inventory")
    if inter_license_file.get("Source") != r"assets\fonts\Inter-OFL.txt":
        errors.append("MSI does not embed the bundled Inter license")
    component_refs = {
        node.get("Id") for node in feature.findall("wix:ComponentRef", WIX_NAMESPACE)
    }
    for expected in {"License", "ThirdPartyLicenses", "InterLicense", "binary0"}:
        if expected not in component_refs:
            errors.append(f"MSI Binaries feature omits component {expected}")
    return errors


def validate_license_inventory(
    manifest: str,
    platform_manifest: str,
    about_config: str,
    generator: str,
    inventory: str,
    ci_workflow: str,
) -> list[str]:
    """Validate the checked-in third-party license generation contract."""

    errors: list[str] = []
    for text, expected, description in [
        (manifest, 'version = "0.1.0-alpha.1"', "root prerelease version"),
        (
            manifest,
            'noter-platform = { version = "=0.1.0-alpha.1"',
            "path dependency prerelease version",
        ),
        (
            platform_manifest,
            'version = "0.1.0-alpha.1"',
            "platform crate prerelease version",
        ),
        (about_config, "ignore-dev-dependencies = true", "runtime-only inventory"),
        (
            about_config,
            '"x86_64-pc-windows-msvc"',
            "Windows license target",
        ),
        (
            about_config,
            '"aarch64-apple-darwin"',
            "Apple Silicon license target",
        ),
        (generator, "MAX_JSON_BYTES = 16 * 1024 * 1024", "bounded JSON input"),
        (generator, "grouped_licenses", "canonical license-text grouping"),
        (generator, "ordered_licenses.sort(", "deterministic license ordering"),
        (generator, 'parsed.scheme not in {"http", "https"}', "safe link policy"),
        (generator, "shell=False", "shell-free license generation"),
        (generator, "os.replace(", "atomic license inventory replacement"),
        (generator, '"--format",', "cargo-about JSON mode"),
        (generator, '"--workspace",', "workspace license scan"),
        (generator, '"--all-features",', "all-feature license scan"),
        (generator, '"--frozen",', "frozen license scan"),
        (generator, '"--fail",', "fail-closed license scan"),
        (
            inventory,
            "<h1>Noter third-party licenses</h1>",
            "generated license inventory heading",
        ),
        (
            inventory,
            "<code>Inter-OFL.txt</code>",
            "generated bundled-font license reference",
        ),
        (
            ci_workflow,
            "cargo install cargo-about --version 0.9.1 --locked --features cli",
            "pinned license generator",
        ),
        (
            ci_workflow,
            "python3 scripts/generate_third_party_licenses.py \\",
            "third-party inventory generation command",
        ),
        (
            ci_workflow,
            "cmp THIRD-PARTY-LICENSES.html",
            "license inventory drift check",
        ),
    ]:
        require_text(text, expected, description, errors)
    if inventory.count("<tr>") < 50:
        errors.append("third-party license inventory has implausibly few components")
    if inventory.count('<section class="license-text">') < 20:
        errors.append("third-party inventory collapses distinct source license texts")
    if inventory.count('<li class="license-user">') < 100:
        errors.append("third-party inventory omits per-license package mappings")
    try:
        configured_targets = tomllib.loads(about_config)["targets"]
    except (tomllib.TOMLDecodeError, KeyError, TypeError):
        errors.append("license inventory target configuration is invalid")
    else:
        if (
            not isinstance(configured_targets, list)
            or len(configured_targets) != len(APPROVED_RELEASE_TARGETS)
            or set(configured_targets) != APPROVED_RELEASE_TARGETS
        ):
            errors.append("license inventory targets differ from the release set")
    return errors


def render_license_rtf(license_text: str) -> bytes:
    """Render the repository license into the deterministic MSI sidecar format."""

    if not license_text.isascii():
        raise ValueError("LICENSE must remain ASCII for the Windows-1252 MSI sidecar")
    escaped = license_text.replace("\\", "\\\\").replace("{", "\\{").replace("}", "\\}")
    escaped = re.sub(r"\r?\n", "\\\\par\r\n", escaped)
    header = (
        r"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fmodern Courier New;}}"
        r"\viewkind4\uc1\pard\f0\fs18"
    )
    return f"{header}\r\n{escaped}}}\r\n".encode("ascii")


def validate_repository(root: Path = REPOSITORY_ROOT) -> list[str]:
    """Return every release-configuration contract violation."""

    try:
        manifest = read_regular_file(root / "Cargo.toml").decode("utf-8")
        platform_manifest = read_regular_file(
            root / "crates/noter-platform/Cargo.toml"
        ).decode("utf-8")
        workflow = read_regular_file(root / ".github/workflows/release.yml").decode(
            "utf-8"
        )
        ci_workflow = read_regular_file(root / ".github/workflows/ci.yml").decode(
            "utf-8"
        )
        wix = read_regular_file(root / "wix/main.wxs")
        license_text = read_regular_file(root / "LICENSE").decode("utf-8")
        license_rtf = read_regular_file(root / "wix/License.rtf")
        about_config = read_regular_file(root / "about.toml").decode("utf-8")
        license_generator = read_regular_file(
            root / "scripts/generate_third_party_licenses.py"
        ).decode("utf-8")
        license_inventory = read_regular_file(
            root / "THIRD-PARTY-LICENSES.html"
        ).decode("utf-8")
        sbom_generator = read_regular_file(
            root / "scripts/generate_release_sboms.py"
        ).decode("utf-8")
        read_regular_file(root / "assets/fonts/Inter-OFL.txt")
    except (OSError, UnicodeError, ValueError) as error:
        return [str(error)]

    errors = validate_manifest(manifest)
    errors.extend(validate_workflow(workflow))
    errors.extend(validate_sbom_generator(sbom_generator))
    errors.extend(validate_wix(wix))
    errors.extend(
        validate_license_inventory(
            manifest,
            platform_manifest,
            about_config,
            license_generator,
            license_inventory,
            ci_workflow,
        )
    )
    try:
        expected_rtf = render_license_rtf(license_text)
    except ValueError as error:
        errors.append(str(error))
    else:
        if license_rtf != expected_rtf:
            errors.append("wix/License.rtf is not an exact rendering of root LICENSE")
    return errors


def main() -> None:
    """Exit unsuccessfully when any release invariant has drifted."""

    errors = validate_repository()
    if errors:
        raise SystemExit("\n".join(f"- {error}" for error in errors))
    print("Release configuration is internally consistent and security-hardened.")


if __name__ == "__main__":
    main()
