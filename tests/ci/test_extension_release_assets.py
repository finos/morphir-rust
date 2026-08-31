"""Tests for exact-byte extension release asset selection."""

from extension_release_test_support import *

class ExtensionAssetSelectionTests(unittest.TestCase):
    def fixture(self, temporary: str) -> AssetFixture:
        return AssetFixture(Path(temporary).resolve())

    def test_valid_bundle_selects_every_exact_asset(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)

            selection = fixture.select()

            self.assertEqual(
                [
                    "morphir-avro-extension-0.1.0.wasm",
                    "morphir-avro-extension-0.1.0.wasm.sha256",
                    "morphir-avro-extension-0.1.0.release.json",
                ],
                [asset.name for asset in selection.uploads],
            )
            self.assertEqual([], selection.skipped)

    def test_workspace_selects_two_exact_bundles_with_namespaced_descriptors(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)
            zeta = fixture.add_extension(
                short_id="zeta",
                package="morphir-zeta-extension",
                artifact_base="morphir-zeta-extension",
                extension_id="morphir-zeta",
                version="0.3.0",
            )
            original_bytes = {
                path: path.read_bytes()
                for path in [fixture.artifact, fixture.checksum, fixture.descriptor, *zeta]
            }

            selection = fixture.select("v0.2.0")

            self.assertEqual(
                [
                    "morphir-avro-extension-0.1.0.wasm",
                    "morphir-avro-extension-0.1.0.wasm.sha256",
                    "morphir-avro-extension-0.1.0.release.json",
                    "morphir-zeta-extension-0.3.0.wasm",
                    "morphir-zeta-extension-0.3.0.wasm.sha256",
                    "morphir-zeta-extension-0.3.0.release.json",
                ],
                [asset.name for asset in selection.uploads],
            )
            self.assertEqual(
                original_bytes,
                {path: path.read_bytes() for path in original_bytes},
            )

    def test_rejects_cross_bundle_collision_after_publication_name_mapping(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)
            fixture.add_extension(
                short_id="zeta",
                package="morphir-zeta-extension",
                artifact_base="morphir-avro-extension",
                extension_id="morphir-zeta",
                version="0.1.0",
            )

            with self.assertRaisesRegex(
                select_extension_assets.AssetError, "duplicate release asset name"
            ):
                fixture.select("v0.2.0")

    def test_workspace_tag_validates_descriptor_against_package_version(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)

            selection = fixture.select("v0.2.0")

            self.assertEqual(3, len(selection.uploads))

    def test_rejects_missing_checksum(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)
            fixture.checksum.unlink()

            with self.assertRaisesRegex(
                select_extension_assets.AssetError, "bundle entries"
            ):
                fixture.select()

    def test_rejects_checksum_file_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)
            fixture.checksum.write_text(
                f"{'0' * 64}  {fixture.artifact_name}\n", encoding="utf-8"
            )

            with self.assertRaisesRegex(
                select_extension_assets.AssetError, "checksum"
            ):
                fixture.select()

    def test_rejects_artifact_bytes_that_do_not_match_checksum(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)
            fixture.artifact.write_bytes(b"changed after packaging")

            with self.assertRaisesRegex(
                select_extension_assets.AssetError, "SHA-256"
            ):
                fixture.select()

    def test_rejects_descriptor_tag_version_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)
            fixture.rewrite_descriptor(version="0.1.1")

            with self.assertRaisesRegex(
                select_extension_assets.AssetError, "version"
            ):
                fixture.select()

    def test_rejects_descriptor_registry_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)
            fixture.rewrite_descriptor(extensionId="other-extension")

            with self.assertRaisesRegex(
                select_extension_assets.AssetError, "extensionId"
            ):
                fixture.select()

    def test_rejects_malformed_descriptor(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)
            fixture.descriptor.write_text("[]\n", encoding="utf-8")

            with self.assertRaisesRegex(
                select_extension_assets.AssetError, "JSON object"
            ):
                fixture.select()

    def test_rejects_missing_descriptor_commit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)
            descriptor = json.loads(fixture.descriptor.read_text(encoding="utf-8"))
            descriptor.pop("gitCommit")
            fixture.descriptor.write_text(
                json.dumps(descriptor, indent=2) + "\n", encoding="utf-8"
            )

            with self.assertRaisesRegex(
                select_extension_assets.AssetError, "gitCommit"
            ):
                fixture.select()

    def test_rejects_null_descriptor_commit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)
            fixture.rewrite_descriptor(gitCommit=None)

            with self.assertRaisesRegex(
                select_extension_assets.AssetError, "gitCommit"
            ):
                fixture.select()

    def test_rejects_descriptor_commit_from_another_tag_target(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)
            fixture.rewrite_descriptor(gitCommit="f" * 40)

            with self.assertRaisesRegex(
                select_extension_assets.AssetError, "gitCommit"
            ):
                fixture.select()

    def test_existing_assets_with_same_bytes_are_skipped(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)
            shutil.copy2(fixture.artifact, fixture.existing / fixture.artifact.name)
            shutil.copy2(fixture.checksum, fixture.existing / fixture.checksum.name)
            shutil.copy2(
                fixture.descriptor,
                fixture.existing / "morphir-avro-extension-0.1.0.release.json",
            )

            selection = fixture.select()

            self.assertEqual([], selection.uploads)
            self.assertEqual(
                [
                    "morphir-avro-extension-0.1.0.wasm",
                    "morphir-avro-extension-0.1.0.wasm.sha256",
                    "morphir-avro-extension-0.1.0.release.json",
                ],
                [asset.name for asset in selection.expected],
            )
            self.assertEqual(
                [
                    "morphir-avro-extension-0.1.0.wasm",
                    "morphir-avro-extension-0.1.0.wasm.sha256",
                    "morphir-avro-extension-0.1.0.release.json",
                ],
                selection.skipped,
            )

    def test_final_check_detects_deleted_or_mutated_initially_skipped_asset(self) -> None:
        for mutation in ("delete", "replace"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                fixture = self.fixture(temporary)
                shutil.copy2(fixture.artifact, fixture.existing / fixture.artifact.name)
                shutil.copy2(fixture.checksum, fixture.existing / fixture.checksum.name)
                published_descriptor = (
                    fixture.existing / "morphir-avro-extension-0.1.0.release.json"
                )
                shutil.copy2(fixture.descriptor, published_descriptor)
                self.assertEqual([], fixture.select().uploads)
                if mutation == "delete":
                    published_descriptor.unlink()
                else:
                    published_descriptor.write_bytes(b"replaced after initial check")

                with self.assertRaisesRegex(
                    select_extension_assets.AssetError,
                    "missing expected|refusing to replace",
                ):
                    fixture.select(require_all_existing=True)

    def test_existing_namespaced_descriptor_with_different_bytes_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)
            published_descriptor = (
                fixture.existing / "morphir-avro-extension-0.1.0.release.json"
            )
            published_descriptor.write_bytes(b"different descriptor bytes")

            with self.assertRaisesRegex(
                select_extension_assets.AssetError, "refusing to replace"
            ):
                fixture.select()

    def test_existing_asset_with_different_bytes_fails_instead_of_clobbering(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)
            (fixture.existing / fixture.artifact_name).write_bytes(b"old bytes")

            with self.assertRaisesRegex(
                select_extension_assets.AssetError, "refusing to replace"
            ):
                fixture.select()

    def test_cli_writes_only_new_assets_to_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)
            shutil.copy2(fixture.checksum, fixture.existing / fixture.checksum.name)
            manifest = fixture.root / "assets.txt"
            expected_manifest = fixture.root / "expected-assets.txt"
            prepared = fixture.root / "prepared"

            result = subprocess.run(
                [
                    sys.executable,
                    str(SELECT_SCRIPT),
                    "--tag",
                    "extension/avro/v0.1.0",
                    "--commit",
                    RELEASE_COMMIT,
                    "--root",
                    str(fixture.root),
                    "--bundles",
                    str(fixture.bundles),
                    "--existing-assets",
                    str(fixture.existing),
                    "--output-manifest",
                    str(manifest),
                    "--prepared-assets",
                    str(prepared),
                    "--expected-manifest",
                    str(expected_manifest),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertEqual(0, result.returncode, result.stderr)
            self.assertEqual("", result.stdout)
            self.assertEqual(
                [
                    prepared.resolve() / fixture.artifact_name,
                    prepared.resolve()
                    / "morphir-avro-extension-0.1.0.release.json",
                ],
                [Path(line) for line in manifest.read_text(encoding="utf-8").splitlines()],
            )
            self.assertEqual(
                [
                    prepared.resolve() / fixture.artifact_name,
                    prepared.resolve() / fixture.checksum.name,
                    prepared.resolve()
                    / "morphir-avro-extension-0.1.0.release.json",
                ],
                [
                    Path(line)
                    for line in expected_manifest.read_text(encoding="utf-8").splitlines()
                ],
            )
            self.assertEqual(
                fixture.descriptor.read_bytes(),
                (
                    prepared / "morphir-avro-extension-0.1.0.release.json"
                ).read_bytes(),
            )
            self.assertTrue(stat.S_ISREG(manifest.lstat().st_mode))
            self.assertEqual(
                [],
                [path.name for path in fixture.root.iterdir() if ".assets.txt." in path.name],
            )

    def test_rejects_reused_empty_or_partial_prepared_directory(self) -> None:
        for partial in (False, True):
            with self.subTest(partial=partial), tempfile.TemporaryDirectory() as temporary:
                fixture = self.fixture(temporary)
                prepared = fixture.root / "prepared"
                prepared.mkdir()
                if partial:
                    (prepared / "partial.wasm").write_bytes(b"partial")

                with self.assertRaisesRegex(
                    select_extension_assets.AssetError, "prepared assets.*already exists"
                ):
                    select_extension_assets.prepare_publication(
                        prepared,
                        fixture.root / "assets.txt",
                        fixture.root / "expected-assets.txt",
                        fixture.select(),
                    )

    def test_rejects_symlinked_prepared_directory_or_component(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)
            outside = fixture.root / "outside"
            outside.mkdir()
            link = fixture.root / "linked"
            link.symlink_to(outside, target_is_directory=True)

            with self.assertRaisesRegex(
                select_extension_assets.AssetError, "symbolic link"
            ):
                select_extension_assets.prepare_publication(
                    link / "prepared",
                    fixture.root / "assets.txt",
                    fixture.root / "expected-assets.txt",
                    fixture.select(),
                )
            self.assertEqual([], list(outside.iterdir()))

    def test_rejects_reused_or_symlinked_manifest_without_partial_output(self) -> None:
        cases = ("regular", "symlink")
        for case in cases:
            with self.subTest(case=case), tempfile.TemporaryDirectory() as temporary:
                fixture = self.fixture(temporary)
                manifest = fixture.root / "assets.txt"
                outside = fixture.root / "outside-manifest"
                outside.write_text("untouched\n", encoding="utf-8")
                if case == "regular":
                    manifest.write_text("old\n", encoding="utf-8")
                else:
                    manifest.symlink_to(outside)
                prepared = fixture.root / "prepared"

                with self.assertRaisesRegex(
                    select_extension_assets.AssetError,
                    "manifest.*already exists|symbolic link",
                ):
                    select_extension_assets.prepare_publication(
                        prepared,
                        manifest,
                        fixture.root / "expected-assets.txt",
                        fixture.select(),
                    )
                self.assertFalse(prepared.exists())
                self.assertEqual("untouched\n", outside.read_text(encoding="utf-8"))

    def test_rejects_manifest_parent_symlink_without_partial_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)
            outside = fixture.root / "outside"
            outside.mkdir()
            linked = fixture.root / "linked"
            linked.symlink_to(outside, target_is_directory=True)
            prepared = fixture.root / "prepared"

            with self.assertRaisesRegex(
                select_extension_assets.AssetError, "symbolic link"
            ):
                select_extension_assets.prepare_publication(
                    prepared,
                    linked / "assets.txt",
                    fixture.root / "expected-assets.txt",
                    fixture.select(),
                )
            self.assertFalse(prepared.exists())
            self.assertEqual([], list(outside.iterdir()))

    def test_cli_validates_one_matrix_bundle_without_publication_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(temporary)

            result = subprocess.run(
                [
                    sys.executable,
                    str(SELECT_SCRIPT),
                    "--tag",
                    "v0.2.0",
                    "--commit",
                    RELEASE_COMMIT,
                    "--root",
                    str(fixture.root),
                    "--bundles",
                    str(fixture.bundle),
                    "--validate-short-id",
                    "avro",
                ],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertEqual(0, result.returncode, result.stderr)
            self.assertEqual("", result.stdout)
