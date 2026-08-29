#!/usr/bin/env python3
"""Validate Noter's security-hardened release configuration."""

from __future__ import annotations

import os
import re
import stat
import tomllib
import xml.etree.ElementTree as element_tree
from hashlib import sha256
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
PINNED_RELEASE_TOOL_DIGESTS = {
    "b657cf8c04a8b7bc28f39d220f7e6dd11bbd2bdb072c552262bd9ccf597261b5",
    "a3435e9944f1a1297add11c6a8ac1f543c14a5ea88879ee05b24ff8218d46d87",
    "ffaafe99087affa66e3202ebb78fb0aeea6c6ff4019f941c82040502442f770a",
    "83a7d5955c7ac96ede5d896ac9ede5f7ecce9ece0e95d9e47acd766b09e2ef1b",
    "eeb2592233ffaa8536ca809ea50706618cc67b3e26684c5194e79cd642d91b0e",
    "fb8dbee9f182173e062a64a387b21a0badc6fab8b2abf9294973f012972bf6d8",
    "6ac824e1642d6f7277d0ed7ea09411a508f6116ba6fae0aa5f2c7daa2ff43d31",
}
PINNED_ACTION = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")
REVIEWED_RELEASE_WORKFLOW_SHA256 = (
    "d13c6aca54ae69148259df91f681c3b497fcd2aad96dc4b0807d89e3b5b680f1"
)
REVIEWED_WIX_SHA256 = "90d3892cab5d6b450a76a5f1b1596a061306f2bddcac2908c7e59a99a00fa6be"
REVIEWED_CI_WORKFLOW_SHA256 = (
    "4dcac19a085b03d619e669bf1d9eb232d26715cd7927b4410ee3eafa3dc7c77c"
)
REVIEWED_CI_TEST_JOB_SHA256 = (
    "7abb1436d4c1bbcd14c106d5bf60812866df7265966156226d7353561f2ad785"
)
REVIEWED_RELEASE_ARTIFACT_VALIDATOR_SHA256 = (
    "a50d1acbdfbc976eff13254896c9d876905612515f17ec985d91ddbb22e91632"
)
WIX_NAMESPACE = {"wix": "http://schemas.microsoft.com/wix/2006/wi"}
WIX_XML_COMMENT = re.compile(rb"<!--.*?-->", flags=re.DOTALL)
WIX_PROCESSING_INSTRUCTION = re.compile(rb"<\?\s*(.*?)\s*\?>", flags=re.DOTALL)
WIX_PROGRAM_FILES_POLICY = (
    b"if $(sys.BUILDARCH) = x64 or $(sys.BUILDARCH) = arm64",
    b'define PlatformProgramFilesFolder = "ProgramFiles64Folder"',
    b"else",
    b'define PlatformProgramFilesFolder = "ProgramFilesFolder"',
    b"endif",
)


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


def _literal_run_step(host: str, name: str, errors: list[str]) -> str | None:
    """Return one unconditional named host step's literal shell program."""

    lines = host.splitlines()
    marker = f"      - name: {name}"
    starts = [index for index, line in enumerate(lines) if line == marker]
    if len(starts) != 1:
        errors.append(f"host job must contain exactly one {name} step")
        return None
    start = starts[0]
    end = next(
        (
            index
            for index in range(start + 1, len(lines))
            if lines[index].startswith("      - ")
        ),
        len(lines),
    )
    step = lines[start:end]
    if any(re.match(r"^ {8}(?:if|continue-on-error)\s*:", line) for line in step):
        errors.append(f"host step {name} must be unconditional and fail closed")
        return None
    run_markers = [index for index, line in enumerate(step) if line == "        run: |"]
    if len(run_markers) != 1:
        errors.append(f"host step {name} must contain one literal run block")
        return None
    script_lines = step[run_markers[0] + 1 :]
    if any(line and not line.startswith("          ") for line in script_lines):
        errors.append(f"host step {name} has invalid literal-run indentation")
        return None
    return "\n".join(line[10:] if line else "" for line in script_lines)


def _executable_shell_lines(script: str | None) -> tuple[str, ...]:
    """Return nonempty, non-comment shell lines from a literal run block."""

    if script is None:
        return ()
    return tuple(
        stripped
        for line in script.splitlines()
        if (stripped := line.strip()) and not stripped.startswith("#")
    )


def _top_level_mapping(text: str, name: str) -> tuple[str, ...] | None:
    """Return one top-level YAML mapping without accepting duplicate keys."""

    lines = text.splitlines()
    marker = f"{name}:"
    starts = [index for index, line in enumerate(lines) if line == marker]
    if len(starts) != 1:
        return None
    start = starts[0]
    end = next(
        (
            index
            for index in range(start + 1, len(lines))
            if lines[index] and not lines[index][0].isspace()
        ),
        len(lines),
    )
    return tuple(lines[start:end])


def validate_manifest(text: str) -> list[str]:
    """Validate cargo-dist and MSI metadata that must remain stable."""

    errors: list[str] = []
    requirements = {
        'cargo-dist-version = "0.32.0"': "pinned cargo-dist version",
        'version = "0.1.0-alpha.2"': "prerelease package version",
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
    if sha256(text.encode("utf-8")).hexdigest() != REVIEWED_RELEASE_WORKFLOW_SHA256:
        errors.append("release workflow differs from its reviewed source")
    jobs_marker = text.find("\njobs:\n")
    host_marker = text.find("\n  host:\n")
    if jobs_marker < 0 or host_marker < 0:
        return ["release workflow is missing its jobs or host boundary"]

    header = text[:jobs_marker]
    pre_host = text[:host_marker]
    host = text[host_marker:]
    expected_concurrency = (
        "concurrency:",
        "  group: release-${{ github.event_name == 'workflow_dispatch' && inputs.tag != 'dry-run' && inputs.tag || github.run_id }}",
        "  queue: max",
        "  cancel-in-progress: false",
    )
    if _top_level_mapping(header, "concurrency") != expected_concurrency:
        errors.append(
            "publishing tags must queue without cancellation while PR plans and dry runs stay isolated"
        )
    immutable_target_script = _literal_run_step(
        host, "Validate immutable release target", errors
    )
    publication_script = _literal_run_step(host, "Finalize GitHub Release", errors)
    draft_script = _literal_run_step(
        host, "Create or refresh GitHub Draft Release", errors
    )
    remote_asset_script = _literal_run_step(
        host, "Verify GitHub Draft Release Assets", errors
    )
    immutable_target_lines = _executable_shell_lines(immutable_target_script)
    publication_lines = _executable_shell_lines(publication_script)
    draft_lines = _executable_shell_lines(draft_script)
    remote_asset_lines = _executable_shell_lines(remote_asset_script)
    host_step_headers = [
        line for line in host.splitlines() if line.startswith("      - ")
    ]
    hosting_headers = [
        index
        for index, header in enumerate(host_step_headers)
        if header == "      - id: host"
    ]
    immutable_headers = [
        index
        for index, header in enumerate(host_step_headers)
        if header == "      - name: Validate immutable release target"
    ]
    publication_headers = [
        index
        for index, header in enumerate(host_step_headers)
        if header == "      - name: Finalize GitHub Release"
    ]
    attestation_headers = [
        index
        for index, header in enumerate(host_step_headers)
        if header == "      - name: Attest"
    ]
    draft_headers = [
        index
        for index, header in enumerate(host_step_headers)
        if header == "      - name: Create or refresh GitHub Draft Release"
    ]
    hosting_artifact_headers = [
        index
        for index, header in enumerate(host_step_headers)
        if header == "      - name: Validate and assemble hosting artifacts"
    ]
    publication_artifact_headers = [
        index
        for index, header in enumerate(host_step_headers)
        if header == "      - name: Validate and assemble publication artifacts"
    ]
    remote_asset_headers = [
        index
        for index, header in enumerate(host_step_headers)
        if header == "      - name: Verify GitHub Draft Release Assets"
    ]
    if (
        len(hosting_headers) != 1
        or len(hosting_artifact_headers) != 1
        or len(immutable_headers) != 1
        or hosting_artifact_headers[0] >= immutable_headers[0]
        or immutable_headers[0] + 1 != hosting_headers[0]
    ):
        errors.append(
            "validated hosting artifacts and immutable target must precede hosting"
        )
    if (
        len(publication_headers) != 1
        or publication_headers[0] != len(host_step_headers) - 1
    ):
        errors.append("publication step must be the final host step")
    if (
        len(hosting_headers) != 1
        or len(draft_headers) != 1
        or len(publication_artifact_headers) != 1
        or len(remote_asset_headers) != 1
        or len(attestation_headers) != 1
        or len(publication_headers) != 1
        or not hosting_headers[0]
        < publication_artifact_headers[0]
        < draft_headers[0]
        < remote_asset_headers[0]
        < attestation_headers[0]
        < publication_headers[0]
    ):
        errors.append(
            "artifact assembly, draft upload, remote verification, attestation, and publication steps must remain ordered"
        )
    if (
        len(publication_artifact_headers) != 1
        or len(draft_headers) != 1
        or publication_artifact_headers[0] + 1 != draft_headers[0]
        or len(remote_asset_headers) != 1
        or draft_headers[0] + 1 != remote_asset_headers[0]
        or len(attestation_headers) != 1
        or remote_asset_headers[0] + 1 != attestation_headers[0]
    ):
        errors.append(
            "publication payload, remote draft, and attestation gates must be adjacent"
        )
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
        '"actions": "read"': "host workflow-read permission",
        '"contents": "write"': "host release permission",
        '"id-token": "write"': "host OIDC permission",
        "--steps=create --steps=upload --output-format=json": (
            "draft-only host upload stages"
        ),
        "needs.build-local-artifacts.result == 'success' && needs.build-global-artifacts.result == 'success'": (
            "required successful local and global release builds"
        ),
        "scripts/check_release_artifacts.py prepare": (
            "release artifact inventory validator"
        ),
        "--stage global": "validated local build payload",
        "--stage host": "validated hosting payload",
        "--stage publish": "validated publication payload",
        "merge-multiple: false": "collision-preserving artifact downloads",
        "steps.cargo-dist.outputs.paths": "manifest-driven artifact upload output",
        "artifacts/*.tar.gz": "source-archive attestation subject",
        "wix3141rtm/wix314-binaries.zip": "pinned WiX Toolset archive",
        "3.14.1.8722": "verified WiX Toolset version",
        "cargo-cyclonedx-0.5.9": "current pinned CycloneDX generator",
        'exec shasum -a 256 "$@"': "macOS archive-checksum compatibility shim",
        "cargo-auditable-receipt.json": "cargo-auditable install receipt verification",
        'test ! -L "$auditable_bin"': "non-symlink POSIX cargo-auditable binary",
        "[IO.FileAttributes]::ReparsePoint": (
            "non-reparse-point Windows cargo-auditable files"
        ),
        '"version":"0.7.5"': "pinned POSIX cargo-auditable receipt version",
        "cargo-auditable-x86_64-pc-windows-msvc.zip": (
            "pinned Windows cargo-auditable archive"
        ),
        "$sourceHash -cne $env:BINARY_SHA256": (
            "verified extracted Windows cargo-auditable binary"
        ),
        "$installedHash -cne $env:BINARY_SHA256": (
            "verified installed Windows cargo-auditable binary"
        ),
        "^v[0-9]+\\.[0-9]+\\.[0-9]+-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*$": (
            "prerelease-only tag validation"
        ),
        '[ "$GITHUB_REF" != "refs/heads/main" ]': "protected-main publication gate",
        "git fetch --no-tags --depth=1 origin main": "fresh main-tip fetch",
        '[ "$GITHUB_SHA" != "$(git rev-parse origin/main)" ]': (
            "initial exact main-tip publication gate"
        ),
        '"repos/$GITHUB_REPOSITORY/actions/workflows/ci.yml/runs"': (
            "exact-main CI run inspection"
        ),
        '-f head_sha="$GITHUB_SHA"': "immutable CI revision selection",
        '.conclusion == "success"': "successful exact-main CI requirement",
        '[ "$RELEASE_COMMIT" != "$(git rev-parse origin/main)" ]': (
            "final exact main-tip publication gate"
        ),
        'git ls-remote --exit-code --refs origin "refs/tags/$RELEASE_TAG"': (
            "remote release-tag inspection"
        ),
        'gh api --method POST "repos/$GITHUB_REPOSITORY/git/refs"': (
            "atomic release-tag creation"
        ),
        '-f ref="refs/tags/$RELEASE_TAG" -f sha="$GITHUB_SHA"': (
            "release-tag commit binding"
        ),
        "--verify-tag": "verified-tag release finalization",
        'gh release edit "$RELEASE_TAG"': "cargo-dist release finalization",
        'gh release create "$RELEASE_TAG"': "verified draft release creation",
        'gh release upload "$RELEASE_TAG" artifacts/*': ("verified draft retry upload"),
        "scripts/check_release_artifacts.py verify-remote": (
            "remote draft asset digest verification"
        ),
        '--release-json "$RUNNER_TEMP/release.json"': (
            "fresh remote draft inventory input"
        ),
        "--draft=false": "explicit post-attestation release publication",
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
        "cargo auditable --version": "ambiguous Cargo self-version probe",
        "cargo-auditable-installer.ps1": (
            "unverified Windows cargo-auditable installer"
        ),
        "--steps=release": "pre-attestation cargo-dist publication",
        "merge-multiple: true": "collision-prone release artifact merge",
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
    if text.count("merge-multiple: false") != 3:
        errors.append("every multi-artifact download must preserve source directories")
    if text.count("if-no-files-found: error") != 5:
        errors.append(
            "every release artifact upload must fail when its payload is absent"
        )
    if (
        'if [ "$GITHUB_SHA" != "$(git rev-parse origin/main)" ]; then'
        not in immutable_target_lines
    ):
        errors.append(
            "initial exact main-tip publication gate must be executable in its host step"
        )
    if not (
        any(
            line.startswith(
                'ci_runs="$(gh api --method GET "repos/$GITHUB_REPOSITORY/actions/workflows/ci.yml/runs"'
            )
            for line in immutable_target_lines
        )
        and '-f head_sha="$GITHUB_SHA" -f per_page=100)"' in immutable_target_lines
        and any(
            ".head_sha == $sha" in line
            and '.head_branch == "main"' in line
            and '.event == "push"' in line
            and '.status == "completed"' in line
            and '.conclusion == "success"' in line
            for line in immutable_target_lines
        )
        and 'if [ "$successful_ci" -lt 1 ]; then' in immutable_target_lines
    ):
        errors.append(
            "exact-main CI success must be enforced before release-tag creation"
        )
    if not (
        any(
            line.startswith('gh api --method POST "repos/$GITHUB_REPOSITORY/git/refs"')
            for line in immutable_target_lines
        )
        and '-f ref="refs/tags/$RELEASE_TAG" -f sha="$GITHUB_SHA" >/dev/null'
        in immutable_target_lines
    ):
        errors.append(
            "exact release tag must be atomically created in the immutable-target step"
        )
    if (
        'if [ "$RELEASE_COMMIT" != "$(git rev-parse origin/main)" ]; then'
        not in publication_lines
    ):
        errors.append(
            "final exact main-tip publication gate must be executable in its host step"
        )
    if (
        'if [ "$(git rev-parse "$RELEASE_TAG^{commit}")" != "$GITHUB_SHA" ]; then'
        not in immutable_target_lines
        or 'if [ "$(git rev-parse "$RELEASE_TAG^{commit}")" != "$RELEASE_COMMIT" ]; then'
        not in publication_lines
    ):
        errors.append(
            "release tag binding checks must be executable in their host steps"
        )
    final_release_indexes = [
        index
        for index, line in enumerate(publication_lines)
        if line.startswith('gh release edit "$RELEASE_TAG" ')
    ]
    # The release is still a draft here, so it is resolved out of the release
    # list by exact tag rather than the by-tag endpoint, which answers 404 for
    # drafts. The length check keeps an ambiguous match failing closed.
    final_remote_verification = (
        "gh api --method GET --paginate --slurp \\",
        '"repos/$GITHUB_REPOSITORY/releases" \\',
        "| jq --arg tag \"$RELEASE_TAG\" '[.[][] | select(.tag_name == $tag)]' \\",
        '> "$RUNNER_TEMP/final-release-matches.json"',
        'test "$(jq \'length\' "$RUNNER_TEMP/final-release-matches.json")" = 1',
        "jq '.[0]' \"$RUNNER_TEMP/final-release-matches.json\" \\",
        '> "$RUNNER_TEMP/final-release.json"',
        "python3 scripts/check_release_artifacts.py verify-remote \\",
        "--artifact-root artifacts \\",
        '--release-json "$RUNNER_TEMP/final-release.json" \\',
        '--expected-tag "$RELEASE_TAG"',
    )
    if (
        len(final_release_indexes) != 1
        or final_release_indexes[0] < len(final_remote_verification)
        or publication_lines[
            final_release_indexes[0]
            - len(final_remote_verification) : final_release_indexes[0]
        ]
        != final_remote_verification
        or not any(
            line == "git fetch --no-tags --depth=1 origin main"
            for line in publication_lines[: final_release_indexes[0]]
        )
    ):
        errors.append(
            "final main, tag, and remote-asset checks must immediately precede publication"
        )
    draft_creation_lines = [
        line
        for line in draft_lines
        if line.startswith('if ! gh release create "$RELEASE_TAG" ')
    ]
    if (
        len(draft_creation_lines) != 1
        or "--verify-tag" not in draft_creation_lines[0]
        or "--prerelease" not in draft_creation_lines[0]
        or "--draft" not in draft_creation_lines[0]
        or "artifacts/*; then" not in draft_lines
    ):
        errors.append(
            "draft creation must upload exact artifacts to one verified prerelease draft"
        )
    if text.count('gh release create "$RELEASE_TAG"') != 1:
        errors.append("draft release creation must appear exactly once")
    if not (
        # A draft is not addressable by tag, so the retry resolves it out of the
        # release list and fails closed unless exactly one release matches.
        'release_json="$(gh api --method GET --paginate --slurp '
        '"repos/$GITHUB_REPOSITORY/releases" | jq -c --arg tag "$RELEASE_TAG" '
        "'[.[][] | select(.tag_name == $tag)] | if length == 1 then .[0] else "
        'error("expected exactly one release for the release tag") end\')"'
        in draft_lines
        and 'if [ "$draft" != "true" ] || [ "$prerelease" != "true" ] || [ "$tag_name" != "$RELEASE_TAG" ]; then'
        in draft_lines
        and "done < <(printf '%s' \"$release_json\" | jq -r '.assets[].id')"
        in draft_lines
        and any(
            line.startswith(
                'gh api --method DELETE "repos/$GITHUB_REPOSITORY/releases/assets/$asset_id"'
            )
            for line in draft_lines
        )
        and 'gh release upload "$RELEASE_TAG" artifacts/*' in draft_lines
    ):
        errors.append(
            "draft retry must verify and replace only the exact private prerelease payload"
        )
    if not (
        any(
            line.startswith("gh api --method GET --paginate --slurp \\")
            for line in remote_asset_lines
        )
        # The draft is resolved out of the release list by exact tag, because
        # the by-tag endpoint answers 404 while a release is still a draft.
        and '"repos/$GITHUB_REPOSITORY/releases" \\' in remote_asset_lines
        and "| jq --arg tag \"$RELEASE_TAG\" '[.[][] | select(.tag_name == $tag)]' \\"
        in remote_asset_lines
        and '> "$RUNNER_TEMP/release-matches.json"' in remote_asset_lines
        # An absent or ambiguous match must fail closed instead of verifying
        # some other release's assets.
        and 'test "$(jq \'length\' "$RUNNER_TEMP/release-matches.json")" = 1'
        in remote_asset_lines
        and 'jq \'.[0]\' "$RUNNER_TEMP/release-matches.json" > "$RUNNER_TEMP/release.json"'
        in remote_asset_lines
        and "python3 scripts/check_release_artifacts.py verify-remote \\"
        in remote_asset_lines
        and "--artifact-root artifacts \\" in remote_asset_lines
        and '--release-json "$RUNNER_TEMP/release.json" \\' in remote_asset_lines
        and '--expected-tag "$RELEASE_TAG"' in remote_asset_lines
    ):
        errors.append(
            "remote draft assets must be read back and verified before attestation"
        )
    if text.count("scripts/check_release_artifacts.py verify-remote") != 2:
        errors.append(
            "remote draft assets must be verified before attestation and publication"
        )
    final_release_lines = [
        line
        for line in publication_lines
        if line.startswith('gh release edit "$RELEASE_TAG" ')
    ]
    if (
        len(final_release_lines) != 1
        or "--verify-tag" not in final_release_lines[0]
        or "--prerelease" not in final_release_lines[0]
        or "--draft=false" not in final_release_lines[0]
    ):
        errors.append(
            "final publication must edit and publish the attested draft release"
        )
    if not (
        any(
            line.startswith(
                'published_json="$(gh api --method GET "repos/$GITHUB_REPOSITORY/releases/tags/$RELEASE_TAG")"'
            )
            for line in publication_lines
        )
        and 'jq -r \'.draft\')" != "false"' in publication_script
        and 'jq -r \'.prerelease\')" != "true"' in publication_script
        and 'jq -r \'.tag_name\')" != "$RELEASE_TAG"' in publication_script
    ):
        errors.append("published prerelease state must be read back and verified")
    for digest in PINNED_RELEASE_TOOL_DIGESTS:
        require_text(text, digest, f"pinned release-tool digest {digest[:12]}", errors)

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


def validate_release_artifact_validator(contents: bytes) -> list[str]:
    """Bind the helper that enforces the release payload and remote digest boundary."""

    errors: list[str] = []
    if sha256(contents).hexdigest() != REVIEWED_RELEASE_ARTIFACT_VALIDATOR_SHA256:
        errors.append("release artifact validator differs from its reviewed source")
    try:
        text = contents.decode("utf-8")
    except UnicodeDecodeError:
        return ["release artifact validator must be UTF-8"]
    for expected, description in {
        "RELEASE_ARTIFACT_KINDS": "independent required release inventory",
        "LOCAL_RELEASE_ARTIFACTS": "required local build inventory",
        "LOCAL_RELEASE_ARTIFACT_TARGETS": "target-specific artifact ownership",
        "GLOBAL_RELEASE_ARTIFACTS": "global artifact ownership",
        "workflow artifact filename collision detected": (
            "cross-container collision rejection"
        ),
        'asset.get("state") != "uploaded"': "remote upload-state verification",
        'asset.get("size") != size': "remote asset-size verification",
        'asset.get("digest") != digest': "remote SHA-256 digest verification",
        "required global artifact build did not succeed": (
            "fail-closed global build result"
        ),
        "downloaded build payload differs from the release plan": (
            "exact hosting inventory verification"
        ),
        "publication payload differs from the release plan and host manifest": (
            "exact publication inventory verification"
        ),
        "release checksum does not match its artifact": (
            "artifact-sidecar checksum verification"
        ),
        "unified release checksum differs from the verified sidecars": (
            "unified checksum verification"
        ),
    }.items():
        require_text(text, expected, description, errors)
    if "subprocess" in text or "shell=True" in text:
        errors.append("release artifact validator must not invoke subprocesses")
    return errors


def validate_wix(contents: bytes) -> list[str]:
    """Validate that the privileged MSI installs only below protected Program Files."""

    errors: list[str] = []
    if sha256(contents).hexdigest() != REVIEWED_WIX_SHA256:
        errors.append("MSI authoring differs from its reviewed source")
    uncommented = WIX_XML_COMMENT.sub(b"", contents)
    instructions = tuple(
        b" ".join(instruction.split())
        for instruction in WIX_PROCESSING_INSTRUCTION.findall(uncommented)
    )
    if (
        not instructions
        or instructions[0] != b"xml version='1.0' encoding='windows-1252'"
        or instructions[1:] != WIX_PROGRAM_FILES_POLICY
    ):
        errors.append(
            "MSI protected-directory macro differs from approved Program Files mapping"
        )
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
    missing_required_node = False
    for description, node in required_nodes.items():
        if node is None:
            errors.append(f"MSI is missing {description}")
            missing_required_node = True
    if missing_required_node:
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
    if product.get("Version") != "0.0.2":
        errors.append("MSI package version differs from its monotonic release sequence")
    if display_version.get("Value") != "0.1.0-alpha.2":
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
    errors.extend(validate_ci_mutation_topology(ci_workflow))
    if sha256(ci_workflow.encode("utf-8")).hexdigest() != REVIEWED_CI_WORKFLOW_SHA256:
        errors.append("CI workflow differs from its reviewed source")
    test_job_marker = ci_workflow.find("\n  test:\n")
    mutation_job_marker = ci_workflow.find("\n  mutation:\n")
    if (
        test_job_marker < 0
        or mutation_job_marker < 0
        or mutation_job_marker <= test_job_marker
    ):
        errors.append("CI is missing its reviewed cross-platform test job boundary")
    else:
        test_job = ci_workflow[test_job_marker:mutation_job_marker]
        if sha256(test_job.encode("utf-8")).hexdigest() != REVIEWED_CI_TEST_JOB_SHA256:
            errors.append(
                "CI test job differs from its reviewed cross-platform program"
            )
    for text, expected, description in [
        (manifest, 'version = "0.1.0-alpha.2"', "root prerelease version"),
        (
            manifest,
            'noter-platform = { version = "=0.1.0-alpha.2"',
            "path dependency prerelease version",
        ),
        (
            platform_manifest,
            'version = "0.1.0-alpha.2"',
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
        (
            generator,
            "MAX_NOTICE_INPUT_BYTES = 16 * 1024 * 1024",
            "bounded packaged legal-file input",
        ),
        (
            generator,
            "MAX_SOURCE_ENTRIES = 131072",
            "bounded third-party source traversal",
        ),
        (
            generator,
            "_collect_packaged_notices",
            "packaged legal-file collection",
        ),
        (generator, "notice_sources", "packaged legal-file aggregation"),
        (generator, "ordered_texts.sort(", "deterministic legal-text ordering"),
        (generator, 'parsed.scheme not in {"http", "https"}', "safe link policy"),
        (generator, "shell=False", "shell-free license generation"),
        (generator, "os.replace(", "atomic license inventory replacement"),
        (generator, '"--format",', "cargo-about JSON mode"),
        (generator, '"--workspace",', "workspace license scan"),
        (generator, '"--all-features",', "all-feature license scan"),
        (generator, '"--frozen",', "frozen license scan"),
        (
            ci_workflow,
            "CARGO_PROFILE_TEST_CODEGEN_UNITS: '8'",
            "bounded macOS mutation codegen units",
        ),
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
        (
            ci_workflow,
            "diff --text --unified=3",
            "bounded license inventory drift diagnostic",
        ),
        (
            ci_workflow,
            "| head -c 65536 || true",
            "license inventory diagnostic output bound",
        ),
    ]:
        require_text(text, expected, description, errors)
    if inventory.count("<tr>") < 50:
        errors.append("third-party license inventory has implausibly few components")
    if inventory.count('<section class="license-text">') < 20:
        errors.append("third-party inventory collapses distinct source license texts")
    if inventory.count('<li class="notice-source">') < 100:
        errors.append("third-party inventory omits packaged legal-file mappings")
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


def validate_ci_mutation_topology(ci_workflow: str) -> list[str]:
    """Validate complete sharding behind one fail-closed mutation gate."""

    errors: list[str] = []
    mutation_marker = ci_workflow.find("\n  mutation:\n")
    gate_marker = ci_workflow.find("\n  mutation-gate:\n")
    docs_marker = ci_workflow.find("\n  docs:\n")
    if (
        mutation_marker < 0
        or gate_marker <= mutation_marker
        or docs_marker <= gate_marker
    ):
        return ["CI is missing its ordered mutation workers, gate, or docs boundary"]

    workers = ci_workflow[mutation_marker:gate_marker]
    gate = ci_workflow[gate_marker:docs_marker]
    matrix_rows = re.findall(
        r"^          - os: ([^\n]+)\n"
        r"            scope: ([^\n]+)"
        r"(?:\n            shard: ([^\n]+))?",
        workers,
        flags=re.MULTILINE,
    )
    expected_rows = [
        ("ubuntu-latest", "linux-0-of-3", "0/3"),
        ("ubuntu-latest", "linux-1-of-3", "1/3"),
        ("ubuntu-latest", "linux-2-of-3", "2/3"),
        ("windows-latest", "windows-0-of-8", "0/8"),
        ("windows-latest", "windows-1-of-8", "1/8"),
        ("windows-latest", "windows-2-of-8", "2/8"),
        ("windows-latest", "windows-3-of-8", "3/8"),
        ("windows-latest", "windows-4-of-8", "4/8"),
        ("windows-latest", "windows-5-of-8", "5/8"),
        ("windows-latest", "windows-6-of-8", "6/8"),
        ("windows-latest", "windows-7-of-8", "7/8"),
        ("macos-latest", "macos", ""),
    ]
    if matrix_rows != expected_rows:
        errors.append(
            "CI mutation partitions are incomplete, reordered, or inconsistent"
        )
    if workers.count("--shard ${{ matrix.shard }}") != 2:
        errors.append("CI mutation workers do not apply both declared shard partitions")
    if "continue-on-error:" in workers:
        errors.append("CI mutation workers must fail closed")
    for expected, description in [
        ("    name: mutation-${{ matrix.scope }}", "stable mutation worker names"),
        ("    timeout-minutes: 90", "bounded mutation worker time"),
        ("          if-no-files-found: error", "required mutation artifacts"),
    ]:
        require_text(workers, expected, description, errors)

    for expected, description in [
        ("    name: mutation-gate", "stable mutation gate name"),
        ("    needs: mutation", "mutation gate dependency"),
        ("    if: ${{ always() }}", "always-evaluated mutation gate"),
        (
            "MUTATION_MATRIX_RESULT: ${{ needs.mutation.result }}",
            "exact mutation matrix result",
        ),
        (
            'if [ "$MUTATION_MATRIX_RESULT" != success ]; then',
            "fail-closed mutation matrix result check",
        ),
    ]:
        require_text(gate, expected, description, errors)
    if "continue-on-error:" in gate:
        errors.append("CI mutation gate must fail closed")
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
        release_artifact_validator = read_regular_file(
            root / "scripts/check_release_artifacts.py"
        )
        read_regular_file(root / "assets/fonts/Inter-OFL.txt")
    except (OSError, UnicodeError, ValueError) as error:
        return [str(error)]

    errors = validate_manifest(manifest)
    errors.extend(validate_workflow(workflow))
    errors.extend(validate_sbom_generator(sbom_generator))
    errors.extend(validate_release_artifact_validator(release_artifact_validator))
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
