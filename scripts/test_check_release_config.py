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
        generator = self.license_generator.replace("notice_sources", "legal_sources")
        inventory = self.inventory.replace(
            '<section class="license-text">', "<section>"
        ).replace('<li class="notice-source">', "<li>")
        errors = validate_license_inventory(
            self.manifest,
            self.platform_manifest,
            self.about_config,
            generator,
            inventory,
            self.ci_workflow,
        )
        self.assertIn("missing packaged legal-file aggregation", errors)
        self.assertIn(
            "third-party inventory collapses distinct source license texts", errors
        )
        self.assertIn(
            "third-party inventory omits packaged legal-file mappings", errors
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
            "ordered_texts.sort(", "sorted(ordered_texts,"
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
            "missing deterministic legal-text ordering",
            errors,
        )

    def test_rejects_an_unbounded_license_inventory_diagnostic(self) -> None:
        changed = self.ci_workflow.replace("| head -c 65536 || true", "|| true")
        errors = validate_license_inventory(
            self.manifest,
            self.platform_manifest,
            self.about_config,
            self.license_generator,
            self.inventory,
            changed,
        )
        self.assertIn("missing license inventory diagnostic output bound", errors)

    def test_rejects_native_windows_script_test_bypasses(self) -> None:
        step = "\n".join(
            [
                "      - name: Run Windows repository validation tests",
                "        if: runner.os == 'Windows'",
                "        run: python -m unittest discover -s scripts -p 'test_*.py'",
            ]
        )
        mutations = {
            "wrong runner": self.ci_workflow.replace(
                "        if: runner.os == 'Windows'",
                "        if: runner.os == 'Linux'",
                1,
            ),
            "no-op command": self.ci_workflow.replace(
                "        run: python -m unittest discover -s scripts -p 'test_*.py'",
                "        run: echo skipped",
                1,
            ),
            "comment-only marker": self.ci_workflow.replace(
                step,
                "\n".join(f"      # {line.strip()}" for line in step.splitlines()),
                1,
            ),
        }

        for description, changed in mutations.items():
            with self.subTest(description):
                errors = validate_license_inventory(
                    self.manifest,
                    self.platform_manifest,
                    self.about_config,
                    self.license_generator,
                    self.inventory,
                    changed,
                )
                self.assertIn(
                    "CI test job differs from its reviewed cross-platform program",
                    errors,
                )

    def test_rejects_a_global_ci_execution_context_bypass(self) -> None:
        changed = self.ci_workflow.replace(
            "permissions:\n",
            "defaults:\n  run:\n    shell: bash -c 'exit 0' {0}\n\npermissions:\n",
            1,
        )

        errors = validate_license_inventory(
            self.manifest,
            self.platform_manifest,
            self.about_config,
            self.license_generator,
            self.inventory,
            changed,
        )

        self.assertIn("CI workflow differs from its reviewed source", errors)

    def test_rejects_unbounded_macos_mutation_codegen_units(self) -> None:
        changed = self.ci_workflow.replace(
            "CARGO_PROFILE_TEST_CODEGEN_UNITS: '8'",
            "CARGO_PROFILE_TEST_CODEGEN_UNITS: '256'",
            1,
        )
        self.assertNotEqual(changed, self.ci_workflow)

        errors = validate_license_inventory(
            self.manifest,
            self.platform_manifest,
            self.about_config,
            self.license_generator,
            self.inventory,
            changed,
        )

        self.assertIn("missing bounded macOS mutation codegen units", errors)

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

    def test_rejects_cargo_self_version_as_auditable_identity(self) -> None:
        changed = self.workflow.replace(
            'test ! -L "$auditable_bin"',
            "cargo auditable --version | grep -F '0.7.5'",
            1,
        )

        errors = validate_workflow(changed)

        self.assertIn(
            "release workflow contains ambiguous Cargo self-version probe", errors
        )
        self.assertIn("missing non-symlink POSIX cargo-auditable binary", errors)

    def test_rejects_unverified_windows_auditable_installer(self) -> None:
        changed = self.workflow.replace(
            "cargo-auditable-x86_64-pc-windows-msvc.zip",
            "cargo-auditable-installer.ps1",
            1,
        )

        errors = validate_workflow(changed)

        self.assertIn(
            "release workflow contains unverified Windows cargo-auditable installer",
            errors,
        )
        self.assertIn("missing pinned Windows cargo-auditable archive", errors)

    def test_rejects_changed_windows_auditable_archive_digest(self) -> None:
        changed = self.workflow.replace(
            "83a7d5955c7ac96ede5d896ac9ede5f7ecce9ece0e95d9e47acd766b09e2ef1b",
            "0" * 64,
            1,
        )

        self.assertIn(
            "missing pinned release-tool digest 83a7d5955c7a",
            validate_workflow(changed),
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

    def test_rejects_a_final_main_tip_guard_preserved_only_in_a_comment(
        self,
    ) -> None:
        guard = "\n".join(
            [
                '          if [ "$RELEASE_COMMIT" != "$(git rev-parse origin/main)" ]; then',
                '            echo "Main advanced before final publication; refusing stale publication." >&2',
                "            exit 2",
                "          fi",
            ]
        )
        comment = (
            '          # Marker only: [ "$RELEASE_COMMIT" != '
            '"$(git rev-parse origin/main)" ]'
        )
        changed = self.workflow.replace(guard, comment, 1)

        self.assertNotEqual(changed, self.workflow)
        self.assertIn(
            "final exact main-tip publication gate must be executable in its host step",
            validate_workflow(changed),
        )

    def test_rejects_an_exact_main_tip_guard_hidden_in_dead_shell_control_flow(
        self,
    ) -> None:
        guard = "\n".join(
            [
                '          if [ "$RELEASE_COMMIT" != "$(git rev-parse origin/main)" ]; then',
                '            echo "Main advanced before final publication; refusing stale publication." >&2',
                "            exit 2",
                "          fi",
            ]
        )
        wrapped_guard = "\n".join(
            [
                "          if false; then",
                *[f"  {line}" for line in guard.splitlines()],
                "          fi",
            ]
        )
        changed = self.workflow.replace(guard, wrapped_guard, 1)

        self.assertNotEqual(changed, self.workflow)
        self.assertIn(
            "release workflow differs from its reviewed source",
            validate_workflow(changed),
        )

    def test_rejects_an_immutable_target_gate_moved_after_hosting(self) -> None:
        marker = "      - name: Validate immutable release target"
        start = self.workflow.index(marker)
        end = self.workflow.index("      - ", start + len(marker))
        gate_step = self.workflow[start:end]
        changed = self.workflow[:start] + self.workflow[end:]
        host_start = changed.index("      - id: host")
        host_end = changed.index("      - ", host_start + 1)
        changed = changed[:host_end] + gate_step + changed[host_end:]

        self.assertIn(
            "immutable release target step must immediately precede hosting",
            validate_workflow(changed),
        )

    def test_rejects_host_job_execution_context_bypasses(self) -> None:
        step = "      - name: Validate immutable release target\n"
        mutations = {
            "spaced step condition": self.workflow.replace(
                step,
                step + "        if : ${{ false }}\n",
                1,
            ),
            "ignored gate failure": self.workflow.replace(
                step,
                step + "        continue-on-error: true\n",
                1,
            ),
            "shell override": self.workflow.replace(
                step + "        shell: bash\n",
                step + "        shell: bash -c 'exit 0' {0}\n",
                1,
            ),
            "step-local shell initialization": self.workflow.replace(
                step + "        shell: bash\n        env:\n          RELEASE_TAG:",
                step + "        shell: bash\n        env:\n"
                "          BASH_ENV: scripts/release-shell-init.sh\n"
                "          RELEASE_TAG:",
                1,
            ),
            "prior environment poisoning": self.workflow.replace(
                step,
                "      - name: Poison later shells\n"
                "        run: echo BASH_ENV=release-shell-init.sh >> $GITHUB_ENV\n"
                + step,
                1,
            ),
        }

        for description, changed in mutations.items():
            with self.subTest(description):
                self.assertNotEqual(changed, self.workflow)
                self.assertIn(
                    "release workflow differs from its reviewed source",
                    validate_workflow(changed),
                )

    def test_rejects_pre_host_release_controls_preserved_only_as_text(self) -> None:
        plan_guard = "\n".join(
            [
                '            if [ "$GITHUB_REF" != "refs/heads/main" ]; then',
                '              echo "Publishing must be dispatched from the protected main branch." >&2',
                "              exit 2",
                "            fi",
            ]
        )
        installer_digest = (
            "a3435e9944f1a1297add11c6a8ac1f543c14a5ea88879ee05b24ff8218d46d87"
        )
        mutations = {
            "commented plan guard": self.workflow.replace(
                plan_guard,
                '            # Marker only: [ "$GITHUB_REF" != "refs/heads/main" ]',
                1,
            ),
            "spaced unpinned action key": self.workflow.replace(
                "      - uses:", "      - uses :", 1
            ),
            "installer digest moved to comment": self.workflow.replace(
                installer_digest,
                "0" * 64,
                1,
            ).replace(
                "\n  host:\n",
                f"\n  # Marker only: {installer_digest}\n  host:\n",
                1,
            ),
        }

        for description, changed in mutations.items():
            with self.subTest(description):
                self.assertNotEqual(changed, self.workflow)
                self.assertIn(
                    "release workflow differs from its reviewed source",
                    validate_workflow(changed),
                )

    def test_rejects_a_release_tag_not_bound_to_the_built_commit(self) -> None:
        changed = self.workflow.replace(
            'git rev-parse "$RELEASE_TAG^{commit}"', "git rev-parse HEAD"
        )
        self.assertIn(
            "release tag must be verified before and after atomic creation",
            validate_workflow(changed),
        )

    def test_rejects_release_tag_guards_preserved_only_in_comments(self) -> None:
        initial_guard = "\n".join(
            [
                '            if [ "$(git rev-parse "$RELEASE_TAG^{commit}")" != "$GITHUB_SHA" ]; then',
                '              echo "The release tag already points to a different commit." >&2',
                "              exit 2",
                "            fi",
            ]
        )
        final_guard = "\n".join(
            [
                '          if [ "$(git rev-parse "$RELEASE_TAG^{commit}")" != "$RELEASE_COMMIT" ]; then',
                '            echo "The release tag does not identify the built commit." >&2',
                "            exit 2",
                "          fi",
            ]
        )
        marker = 'git rev-parse "$RELEASE_TAG^{commit}"'
        changed = self.workflow.replace(
            initial_guard,
            f"            # Marker only: {marker}",
            1,
        ).replace(
            final_guard,
            f"          # Marker only: {marker}",
            1,
        )

        self.assertNotEqual(changed, self.workflow)
        self.assertIn(
            "release tag binding checks must be executable in their host steps",
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

    def test_rejects_an_msi_macro_that_redirects_outside_program_files(self) -> None:
        changed = self.wix.replace(
            b'PlatformProgramFilesFolder = "ProgramFiles64Folder"',
            b'PlatformProgramFilesFolder = "WindowsFolder"',
            1,
        ).replace(
            b'PlatformProgramFilesFolder = "ProgramFilesFolder"',
            b'PlatformProgramFilesFolder = "WindowsFolder"',
            1,
        )

        self.assertIn(
            "MSI protected-directory macro differs from approved Program Files mapping",
            validate_wix(changed),
        )

    def test_rejects_an_approved_msi_mapping_hidden_in_dead_preprocessor_code(
        self,
    ) -> None:
        start = self.wix.index(b"<?if $(sys.BUILDARCH)")
        end = self.wix.index(b"<?endif ?>", start) + len(b"<?endif ?>")
        approved_policy = self.wix[start:end]
        changed = b"".join(
            [
                self.wix[:start],
                b"<?if 1 = 0 ?>\n",
                approved_policy,
                b"\n<?endif ?>\n",
                b"<?define PlatformProgramFilesFolder = WindowsFolder ?>",
                self.wix[end:],
            ]
        )

        self.assertIn(
            "MSI protected-directory macro differs from approved Program Files mapping",
            validate_wix(changed),
        )

    def test_rejects_dynamic_redirection_of_the_msi_application_directory(
        self,
    ) -> None:
        changed = self.wix.replace(
            b"    </Product>",
            b"        <SetDirectory Id='APPLICATIONFOLDER' "
            b"Value='[LocalAppDataFolder]Noter' Sequence='execute'>"
            b"1</SetDirectory>\r\n    </Product>",
            1,
        )

        self.assertNotEqual(changed, self.wix)
        self.assertIn(
            "MSI authoring differs from its reviewed source",
            validate_wix(changed),
        )

    def test_rejects_an_msi_without_third_party_notices(self) -> None:
        changed = self.wix.replace(b"ThirdPartyLicensesFile", b"MissingNoticeFile")
        self.assertIn(
            "MSI is missing third-party license inventory", validate_wix(changed)
        )

    def test_rejects_an_msi_without_a_monotonic_package_sequence(self) -> None:
        changed = self.wix.replace(b"Version='0.0.2'", b"Version='$(var.Version)'", 1)
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
