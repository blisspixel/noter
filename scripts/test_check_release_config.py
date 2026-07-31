"""Tests for the dependency-free release-configuration validator."""

from __future__ import annotations

import unittest

from check_release_config import (
    REPOSITORY_ROOT,
    read_regular_file,
    validate_license_inventory,
    validate_manifest,
    validate_repository,
    validate_sbom_generator,
    validate_wix,
    validate_workflow,
)


class ReleaseConfigurationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.manifest = read_regular_file(REPOSITORY_ROOT / "Cargo.toml").decode("utf-8")
        cls.workflow = read_regular_file(
            REPOSITORY_ROOT / ".github/workflows/release.yml"
        ).decode("utf-8")
        cls.wix = read_regular_file(REPOSITORY_ROOT / "wix/main.wxs")
        cls.platform_manifest = read_regular_file(
            REPOSITORY_ROOT / "crates/noter-platform/Cargo.toml"
        ).decode("utf-8")
        cls.about_config = read_regular_file(REPOSITORY_ROOT / "about.toml").decode(
            "utf-8"
        )
        cls.license_generator = read_regular_file(
            REPOSITORY_ROOT / "scripts/generate_third_party_licenses.py"
        ).decode("utf-8")
        cls.inventory = read_regular_file(
            REPOSITORY_ROOT / "THIRD-PARTY-LICENSES.html"
        ).decode("utf-8")
        cls.ci_workflow = read_regular_file(
            REPOSITORY_ROOT / ".github/workflows/ci.yml"
        ).decode("utf-8")
        cls.sbom_generator = read_regular_file(
            REPOSITORY_ROOT / "scripts/generate_release_sboms.py"
        ).decode("utf-8")

    def test_repository_configuration_is_consistent(self) -> None:
        self.assertEqual(validate_repository(), [])

    def test_rejects_collapsed_overview_only_license_texts(self) -> None:
        generator = self.license_generator.replace(
            "grouped_licenses", "license_buckets"
        )
        inventory = self.inventory.replace(
            '<section class="license-text">', "<section>"
        ).replace('<li class="license-user">', "<li>")
        errors = validate_license_inventory(
            self.manifest,
            self.platform_manifest,
            self.about_config,
            generator,
            inventory,
            self.ci_workflow,
        )
        self.assertIn("missing canonical license-text grouping", errors)
        self.assertIn(
            "third-party inventory collapses distinct source license texts", errors
        )
        self.assertIn(
            "third-party inventory omits per-license package mappings", errors
        )

    def test_rejects_license_targets_that_differ_from_release_targets(self) -> None:
        about_config = self.about_config.replace(
            '    "x86_64-unknown-linux-gnu",',
            '    "x86_64-unknown-linux-gnu",\n    "x86_64-unknown-linux-musl",',
            1,
        )
        errors = validate_license_inventory(
            self.manifest,
            self.platform_manifest,
            about_config,
            self.license_generator,
            self.inventory,
            self.ci_workflow,
        )
        self.assertIn("license inventory targets differ from the release set", errors)

    def test_rejects_a_nondeterministic_license_generator(self) -> None:
        changed = self.license_generator.replace(
            "ordered_licenses.sort(", "sorted(ordered_licenses,"
        )
        errors = validate_license_inventory(
            self.manifest,
            self.platform_manifest,
            self.about_config,
            changed,
            self.inventory,
            self.ci_workflow,
        )
        self.assertIn(
            "missing deterministic license ordering",
            errors,
        )

    def test_rejects_a_floating_release_tool(self) -> None:
        changed = self.workflow.replace(
            "releases/download/v0.7.5", "releases/latest/download", 1
        )
        self.assertTrue(
            any(
                "floating release-tool download" in error
                for error in validate_workflow(changed)
            )
        )

    def test_rejects_pre_host_write_permission(self) -> None:
        changed = self.workflow.replace('"contents": "read"', '"contents": "write"', 1)
        errors = validate_workflow(changed)
        self.assertTrue(any("pre-host" in error for error in errors))

    def test_rejects_singular_sbom_output(self) -> None:
        changed = self.workflow.replace(
            "steps.cargo-dist.outputs.paths",
            "steps.cargo-cyclonedx.output.paths",
            1,
        )
        self.assertIn(
            "release workflow contains singular SBOM output reference",
            validate_workflow(changed),
        )

    def test_rejects_an_unpinned_action(self) -> None:
        changed = self.workflow.replace(
            "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
            "actions/checkout@main",
            1,
        )
        self.assertTrue(
            any("not commit-pinned" in error for error in validate_workflow(changed))
        )

    def test_rejects_publication_outside_protected_main(self) -> None:
        changed = self.workflow.replace(
            '[ "$GITHUB_REF" != "refs/heads/main" ]',
            '[ "$GITHUB_REF" != "refs/heads/release" ]',
            1,
        )
        self.assertIn(
            "missing protected-main publication gate", validate_workflow(changed)
        )

    def test_rejects_publication_after_main_advances(self) -> None:
        changed = self.workflow.replace(
            '[ "$GITHUB_SHA" != "$(git rev-parse origin/main)" ]',
            '[ "$GITHUB_SHA" != "$(git rev-parse HEAD)" ]',
            1,
        )
        self.assertIn(
            "missing initial exact main-tip publication gate",
            validate_workflow(changed),
        )

    def test_rejects_publication_if_main_advances_after_hosting(self) -> None:
        changed = self.workflow.replace(
            '[ "$RELEASE_COMMIT" != "$(git rev-parse origin/main)" ]',
            '[ "$RELEASE_COMMIT" != "$(git rev-parse HEAD)" ]',
            1,
        )
        errors = validate_workflow(changed)
        self.assertIn("missing final exact main-tip publication gate", errors)
        self.assertIn("final main-tip gate must appear exactly once", errors)

    def test_rejects_a_release_tag_not_bound_to_the_built_commit(self) -> None:
        changed = self.workflow.replace(
            'git rev-parse "$RELEASE_TAG^{commit}"', "git rev-parse HEAD"
        )
        self.assertIn(
            "release tag must be verified before and after atomic creation",
            validate_workflow(changed),
        )

    def test_rejects_target_hints_without_verified_tag_creation(self) -> None:
        changed = self.workflow.replace("--verify-tag", '--target "$RELEASE_COMMIT"', 1)
        errors = validate_workflow(changed)
        self.assertIn("missing verified-tag release creation", errors)
        self.assertIn(
            "release workflow contains target-commit hint for a possibly existing tag",
            errors,
        )

    def test_rejects_stable_tag_publication(self) -> None:
        changed = self.workflow.replace(
            "^v[0-9]+\\.[0-9]+\\.[0-9]+-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*$",
            "^v[0-9]+\\.[0-9]+\\.[0-9]+$",
            1,
        )
        self.assertIn(
            "missing prerelease-only tag validation", validate_workflow(changed)
        )

    def test_rejects_an_sbom_matrix_missing_a_release_target(self) -> None:
        changed = self.sbom_generator.replace('    "aarch64-apple-darwin",\n', "", 1)
        self.assertIn(
            "missing SBOM target aarch64-apple-darwin",
            validate_sbom_generator(changed),
        )

    def test_rejects_generated_release_notes(self) -> None:
        changed = self.workflow.replace(
            "gh attestation verify PATH_TO_DOWNLOADED_ASSET",
            "announcement_github_body",
            1,
        )
        errors = validate_workflow(changed)
        self.assertIn(
            "release workflow contains unreviewed generated release notes", errors
        )
        self.assertIn("missing attestation-first release guidance", errors)

    def test_rejects_a_configurable_system_path_install(self) -> None:
        changed = self.wix.replace(
            b"            AllowAdvertise='no'",
            b"            ConfigurableDirectory='APPLICATIONFOLDER'\r\n"
            b"            AllowAdvertise='no'",
            1,
        )
        self.assertIn(
            "per-machine MSI must not expose a user-configurable install directory",
            validate_wix(changed),
        )

    def test_rejects_an_msi_without_third_party_notices(self) -> None:
        changed = self.wix.replace(b"ThirdPartyLicensesFile", b"MissingNoticeFile")
        self.assertIn(
            "MSI is missing third-party license inventory", validate_wix(changed)
        )

    def test_rejects_an_msi_without_a_monotonic_package_sequence(self) -> None:
        changed = self.wix.replace(b"Version='0.0.1'", b"Version='$(var.Version)'", 1)
        self.assertIn(
            "MSI package version differs from its monotonic release sequence",
            validate_wix(changed),
        )

    def test_rejects_ambiguous_same_version_msi_upgrades(self) -> None:
        changed = self.wix.replace(
            b"Schedule='afterInstallInitialize'",
            b"AllowSameVersionUpgrades='yes' Schedule='afterInstallInitialize'",
            1,
        )
        self.assertIn(
            "MSI must not permit ambiguous same-version upgrades",
            validate_wix(changed),
        )

    def test_rejects_missing_manual_drift_policy(self) -> None:
        changed = self.manifest.replace('allow-dirty = ["ci", "msi"]', "", 1)
        self.assertIn(
            "missing reviewed generated-file override", validate_manifest(changed)
        )

    def test_rejects_missing_archive_license_payload(self) -> None:
        changed = self.manifest.replace(
            'include = ["THIRD-PARTY-LICENSES.html", "assets/fonts/Inter-OFL.txt"]',
            "",
            1,
        )
        self.assertIn("missing archive license payload", validate_manifest(changed))

    def test_rejects_archive_notices_in_an_ignored_nested_table(self) -> None:
        include = (
            'include = ["THIRD-PARTY-LICENSES.html", "assets/fonts/Inter-OFL.txt"]'
        )
        changed = self.manifest.replace(include, "", 1)
        changed += f"\n[workspace.metadata.dist.artifacts.archives]\n{include}\n"
        self.assertIn("missing archive license payload", validate_manifest(changed))

    def test_rejects_reintroducing_an_uninventoried_musl_runtime(self) -> None:
        changed = self.manifest.replace(
            '"x86_64-unknown-linux-gnu",',
            '"x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl",',
            1,
        )
        self.assertIn(
            "cargo-dist targets differ from the approved release set",
            validate_manifest(changed),
        )

    def test_rejects_generic_sbom_generation(self) -> None:
        changed = self.manifest.replace(
            "cargo-cyclonedx = false", "cargo-cyclonedx = true", 1
        )
        self.assertIn(
            "cargo-dist generic SBOM generation must remain disabled",
            validate_manifest(changed),
        )

    def test_rejects_an_sbom_missing_from_the_dist_manifest(self) -> None:
        changed = self.manifest.replace(
            '    "noter_aarch64-apple-darwin.cdx.xml",\n', "", 1
        )
        self.assertIn(
            "cargo-dist manifest does not list the exact SBOM set",
            validate_manifest(changed),
        )

    def test_rejects_an_unreviewed_sbom_build_command(self) -> None:
        changed = self.manifest.replace(
            '["python", "scripts/generate_release_sboms.py"]',
            '["sh", "scripts/generate_release_sboms.sh"]',
            1,
        )
        self.assertIn(
            "cargo-dist does not use the reviewed SBOM generator",
            validate_manifest(changed),
        )

    def test_rejects_remote_fetches_in_the_sbom_generator(self) -> None:
        changed = (
            self.sbom_generator + '\nsubprocess.run(["curl", "example.invalid"])\n'
        )
        self.assertIn(
            "SBOM generator must not fetch remote content",
            validate_sbom_generator(changed),
        )


if __name__ == "__main__":
    unittest.main()
