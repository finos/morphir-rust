"""Tests for extension release tag routing."""

from extension_release_test_support import *

class ExtensionReleaseRoutingTests(unittest.TestCase):
    def test_dedicated_tag_selects_one_extension(self) -> None:
        release = extension_release.resolve_release(
            "extension/avro/v0.1.0", REGISTRY, "0.2.0", PACKAGE_VERSIONS
        )

        self.assertEqual(["avro"], release.short_ids)
        self.assertEqual("0.1.0", release.version)
        self.assertEqual("extension", release.kind)

    def test_workspace_tag_selects_opted_in_extensions_in_sorted_order(self) -> None:
        release = extension_release.resolve_release(
            "v0.2.0", REGISTRY, "0.2.0", PACKAGE_VERSIONS
        )

        self.assertEqual(["avro", "zeta"], release.short_ids)
        self.assertEqual("workspace", release.kind)
        self.assertEqual(
            '{"include":[{"short_id":"avro","version":"0.1.0"},'
            '{"short_id":"zeta","version":"0.3.0"}]}',
            extension_release.compact_matrix(release, REGISTRY, PACKAGE_VERSIONS),
        )

    def test_dedicated_matrix_uses_the_extension_version(self) -> None:
        release = extension_release.resolve_release(
            "extension/private/v1.2.3", REGISTRY, "0.2.0", PACKAGE_VERSIONS
        )

        self.assertEqual(
            '{"include":[{"short_id":"private","version":"1.2.3"}]}',
            extension_release.compact_matrix(release, REGISTRY, PACKAGE_VERSIONS),
        )

    def test_rejects_malformed_tags(self) -> None:
        malformed = [
            "0.2.0",
            "v1",
            "v01.2.3",
            "v1.2.3/extra",
            "extension/avro/0.1.0",
            "extension/Avro/v0.1.0",
            "extension/avro/v0.1",
            "extension/avro/v0.1.0/extra",
        ]

        for tag in malformed:
            with self.subTest(tag=tag):
                with self.assertRaisesRegex(extension_release.ReleaseError, "tag"):
                    extension_release.resolve_release(
                        tag, REGISTRY, "0.2.0", PACKAGE_VERSIONS
                    )

    def test_rejects_unknown_short_id(self) -> None:
        with self.assertRaisesRegex(extension_release.ReleaseError, "unknown.*missing"):
            extension_release.resolve_release(
                "extension/missing/v1.0.0", REGISTRY, "0.2.0", PACKAGE_VERSIONS
            )

    def test_rejects_dedicated_tag_crate_version_mismatch(self) -> None:
        with self.assertRaisesRegex(extension_release.ReleaseError, "0.1.1.*0.1.0"):
            extension_release.resolve_release(
                "extension/avro/v0.1.1", REGISTRY, "0.2.0", PACKAGE_VERSIONS
            )

    def test_rejects_workspace_tag_version_mismatch(self) -> None:
        with self.assertRaisesRegex(extension_release.ReleaseError, "0.2.1.*0.2.0"):
            extension_release.resolve_release(
                "v0.2.1", REGISTRY, "0.2.0", PACKAGE_VERSIONS
            )

    def test_cli_writes_sorted_compact_github_outputs(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / ".github").mkdir()
            (root / "crates" / "morphir-avro-extension").mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[workspace.package]\nversion = "0.2.0"\n', encoding="utf-8"
            )
            (root / ".github" / "extensions.toml").write_text(
                textwrap.dedent(
                    """\
                    [extensions.avro]
                    package = "morphir-avro-extension"
                    artifact = "morphir-avro-extension"
                    extension_id = "morphir-avro"
                    mep_versions = ["0.1"]
                    targets = ["avro"]
                    ir_versions = ["3", "4"]
                    release_with_workspace = true
                    """
                ),
                encoding="utf-8",
            )
            (root / "crates" / "morphir-avro-extension" / "Cargo.toml").write_text(
                '[package]\nname = "morphir-avro-extension"\nversion = "0.1.0"\n',
                encoding="utf-8",
            )
            github_output = root / "github-output"
            result = subprocess.run(
                [
                    sys.executable,
                    str(RELEASE_SCRIPT),
                    "--root",
                    str(root),
                    "--tag",
                    "v0.2.0",
                    "--commit",
                    RELEASE_COMMIT,
                ],
                check=False,
                capture_output=True,
                text=True,
                env={**os.environ, "GITHUB_OUTPUT": str(github_output)},
            )

            self.assertEqual(0, result.returncode, result.stderr)
            self.assertEqual("", result.stdout)
            self.assertEqual(
                "tag=v0.2.0\n"
                "kind=workspace\n"
                "version=0.2.0\n"
                f"commit={RELEASE_COMMIT}\n"
                'matrix={"include":[{"short_id":"avro","version":"0.1.0"}]}\n',
                github_output.read_text(encoding="utf-8"),
            )
