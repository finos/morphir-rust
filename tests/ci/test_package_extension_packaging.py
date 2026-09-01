"""Tests for deterministic extension release bundle construction."""

from package_extension_test_support import *

class PackageExtensionTests(unittest.TestCase):
    def test_avro_registry_entry_is_independently_versioned(self) -> None:
        registry = tomllib.loads(EXTENSIONS_TOML.read_text(encoding="utf-8"))
        avro = registry["extensions"]["avro"]

        self.assertEqual("morphir-avro-extension", avro["package"])
        self.assertEqual("morphir-avro-extension", avro["artifact"])
        self.assertEqual("morphir-avro", avro["extension_id"])
        self.assertEqual(["0.1"], avro["mep_versions"])
        self.assertEqual(["avro"], avro["targets"])
        self.assertEqual(["3", "4"], avro["ir_versions"])
        self.assertTrue(avro["release_with_workspace"])

    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.fixture = PackageFixture(
            Path(self.temporary_directory.name) / "repository"
        )

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def test_release_descriptor_uses_crate_version_and_wasm_digest(self) -> None:
        result = self.fixture.package()
        self.assertEqual(0, result.returncode, result.stderr)

        descriptor = json.loads(
            (self.fixture.output / "release.json").read_text(encoding="utf-8")
        )
        artifact = self.fixture.output / descriptor["artifact"]
        self.assertEqual(1, descriptor["schemaVersion"])
        self.assertEqual("avro", descriptor["shortId"])
        self.assertEqual("morphir-avro", descriptor["extensionId"])
        self.assertEqual("morphir-avro-extension", descriptor["package"])
        self.assertEqual("0.1.0", descriptor["version"])
        self.assertEqual(["0.1"], descriptor["mepVersions"])
        self.assertEqual("wasm", descriptor["runtime"])
        self.assertEqual(["avro"], descriptor["targets"])
        self.assertEqual(["3", "4"], descriptor["irVersions"])
        self.assertEqual(sha256(artifact), descriptor["sha256"])
        self.assertEqual(self.fixture.wasm.read_bytes(), artifact.read_bytes())

    def test_validate_extension_staging_accepts_any_registered_short_id(self) -> None:
        root = self.fixture.root
        staging = root / ".morphir" / "build" / "extensions" / "openapi"
        staging.mkdir(parents=True)

        validate_extension_staging(root, "openapi")

    def test_clean_extension_staging_refuses_a_traversing_short_id(self) -> None:
        root = self.fixture.root

        with self.assertRaises(PackageError):
            clean_extension_staging(root, "../avro")

    def test_head_snapshot_name_is_scoped_to_the_short_id(self) -> None:
        root = self.fixture.root
        snapshot = root.parent / "morphir-openapi-head.ABC123"
        snapshot.mkdir()

        clean_head_snapshot(root, snapshot, "openapi")

        self.assertFalse(snapshot.exists())

    def test_head_snapshot_rejects_a_mismatched_short_id(self) -> None:
        root = self.fixture.root
        snapshot = root.parent / "morphir-avro-head.ABC123"
        snapshot.mkdir()

        with self.assertRaises(PackageError):
            clean_head_snapshot(root, snapshot, "openapi")

    def test_rejects_unknown_short_id(self) -> None:
        result = self.fixture.package("unknown")

        self.assertNotEqual(0, result.returncode)
        self.assertIn("unknown extension short ID: unknown", result.stderr)
        self.assertFalse(self.fixture.output.exists())

    def test_rejects_missing_wasm(self) -> None:
        missing = self.fixture.root / "input" / "missing.wasm"
        result = self.fixture.package(wasm=missing)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(f"WASM file does not exist: {missing}", result.stderr)
        self.assertFalse(self.fixture.output.exists())

    def test_rejects_registry_and_cargo_package_mismatch(self) -> None:
        self.temporary_directory.cleanup()
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.fixture = PackageFixture(
            Path(self.temporary_directory.name) / "repository",
            cargo_package="another-package",
        )

        result = self.fixture.package()

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "registry package morphir-avro-extension does not match Cargo package another-package",
            result.stderr,
        )
        self.assertFalse(self.fixture.output.exists())

    def test_refuses_to_overwrite_a_different_staged_artifact(self) -> None:
        first = self.fixture.package()
        self.assertEqual(0, first.returncode, first.stderr)
        artifact = self.fixture.output / "morphir-avro-extension-0.1.0.wasm"
        artifact.write_bytes(b"different staged bytes")

        second = self.fixture.package()

        self.assertNotEqual(0, second.returncode)
        self.assertIn("refusing to overwrite different staged bundle", second.stderr)
        self.assertEqual(b"different staged bytes", artifact.read_bytes())

    def test_repeating_an_identical_package_is_idempotent(self) -> None:
        first = self.fixture.package(git_commit="abc123")
        self.assertEqual(0, first.returncode, first.stderr)
        before = {
            path.name: path.read_bytes() for path in self.fixture.output.iterdir()
        }

        second = self.fixture.package(git_commit="abc123")

        self.assertEqual(0, second.returncode, second.stderr)
        after = {path.name: path.read_bytes() for path in self.fixture.output.iterdir()}
        self.assertEqual(before, after)

    def test_checksum_is_basename_only_with_final_newline(self) -> None:
        result = self.fixture.package()
        self.assertEqual(0, result.returncode, result.stderr)
        artifact_name = "morphir-avro-extension-0.1.0.wasm"
        digest = sha256(self.fixture.output / artifact_name)

        checksum = (self.fixture.output / f"{artifact_name}.sha256").read_bytes()

        self.assertEqual(f"{digest}  {artifact_name}\n".encode(), checksum)
        self.assertNotIn(str(self.fixture.output).encode(), checksum)

    def test_descriptor_has_one_final_newline_and_deterministic_field_order(self) -> None:
        result = self.fixture.package()
        self.assertEqual(0, result.returncode, result.stderr)

        raw_descriptor = (self.fixture.output / "release.json").read_bytes()
        descriptor = json.loads(raw_descriptor)

        self.assertTrue(raw_descriptor.endswith(b"\n"))
        self.assertFalse(raw_descriptor.endswith(b"\n\n"))
        self.assertEqual(
            [
                "schemaVersion",
                "shortId",
                "extensionId",
                "package",
                "version",
                "mepVersions",
                "runtime",
                "targets",
                "irVersions",
                "artifact",
                "sha256",
            ],
            list(descriptor),
        )

    def test_git_commit_is_optional_and_last_when_supplied(self) -> None:
        without_commit = self.fixture.package()
        self.assertEqual(0, without_commit.returncode, without_commit.stderr)
        descriptor = json.loads(
            (self.fixture.output / "release.json").read_text(encoding="utf-8")
        )
        self.assertNotIn("gitCommit", descriptor)

        shutil.rmtree(self.fixture.output)
        with_commit = self.fixture.package(git_commit="0123456789abcdef")
        self.assertEqual(0, with_commit.returncode, with_commit.stderr)
        descriptor = json.loads(
            (self.fixture.output / "release.json").read_text(encoding="utf-8")
        )
        self.assertEqual("0123456789abcdef", descriptor["gitCommit"])
        self.assertEqual("gitCommit", list(descriptor)[-1])

    def test_bundle_contains_only_wasm_checksum_and_descriptor(self) -> None:
        result = self.fixture.package()
        self.assertEqual(0, result.returncode, result.stderr)

        self.assertEqual(
            [
                "morphir-avro-extension-0.1.0.wasm",
                "morphir-avro-extension-0.1.0.wasm.sha256",
                "release.json",
            ],
            sorted(path.name for path in self.fixture.output.iterdir()),
        )

    def test_transfer_verifies_and_atomically_copies_the_exact_bundle(self) -> None:
        source = self.fixture.root / "snapshot-bundle"
        self.fixture.output = source
        packaged = self.fixture.package(git_commit="0123456789abcdef")
        self.assertEqual(0, packaged.returncode, packaged.stderr)
        expected = {path.name: path.read_bytes() for path in source.iterdir()}
        final = self.fixture.root / "final-bundle"

        transferred = self.fixture.transfer(source, output=final)

        self.assertEqual(0, transferred.returncode, transferred.stderr)
        self.assertEqual(
            expected, {path.name: path.read_bytes() for path in final.iterdir()}
        )

    def test_transfer_rejects_a_bundle_with_a_checksum_mismatch(self) -> None:
        source = self.fixture.root / "snapshot-bundle"
        self.fixture.output = source
        packaged = self.fixture.package(git_commit="0123456789abcdef")
        self.assertEqual(0, packaged.returncode, packaged.stderr)
        artifact = source / "morphir-avro-extension-0.1.0.wasm"
        artifact.write_bytes(b"tampered")
        final = self.fixture.root / "final-bundle"

        transferred = self.fixture.transfer(source, output=final)

        self.assertNotEqual(0, transferred.returncode)
        self.assertIn("bundle checksum mismatch", transferred.stderr)
        self.assertNotIn("Traceback", transferred.stderr)
        self.assertFalse(final.exists())

    def test_rejects_non_portable_short_ids_before_registry_lookup(self) -> None:
        for short_id in ("../avro", r"..\avro", "Avro", "avro_thing", "avro--idl"):
            with self.subTest(short_id=short_id):
                result = self.fixture.package(short_id)
                self.assertNotEqual(0, result.returncode)
                self.assertIn("invalid extension short ID", result.stderr)
                self.assertNotIn("Traceback", result.stderr)

    def test_rejects_non_portable_registry_package_and_artifact_ids(self) -> None:
        for field, invalid in (("package", "../escape"), ("artifact", r"bad\name")):
            with self.subTest(field=field):
                self.temporary_directory.cleanup()
                self.temporary_directory = tempfile.TemporaryDirectory()
                registry = AVRO_REGISTRY.replace(
                    f'{field} = "morphir-avro-extension"',
                    f'{field} = "{invalid.replace(chr(92), chr(92) * 2)}"',
                )
                self.fixture = PackageFixture(
                    Path(self.temporary_directory.name) / "repository",
                    registry=registry,
                )

                result = self.fixture.package()

                self.assertNotEqual(0, result.returncode)
                self.assertIn(f"invalid extension {field}", result.stderr)
                self.assertNotIn("Traceback", result.stderr)

    def test_rejects_non_semver_cargo_version(self) -> None:
        self.temporary_directory.cleanup()
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.fixture = PackageFixture(
            Path(self.temporary_directory.name) / "repository", cargo_version="1.2"
        )

        result = self.fixture.package()

        self.assertNotEqual(0, result.returncode)
        self.assertIn("invalid Cargo SemVer: 1.2", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    def test_accepts_cargo_semver_with_prerelease_and_build_metadata(self) -> None:
        self.temporary_directory.cleanup()
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.fixture = PackageFixture(
            Path(self.temporary_directory.name) / "repository",
            cargo_version="1.2.3-rc.1+build.7",
        )

        result = self.fixture.package()

        self.assertEqual(0, result.returncode, result.stderr)
        descriptor = json.loads(
            (self.fixture.output / "release.json").read_text(encoding="utf-8")
        )
        self.assertEqual("1.2.3-rc.1+build.7", descriptor["version"])

    def test_rejects_a_crate_manifest_that_resolves_outside_crates(self) -> None:
        crate = self.fixture.root / "crates" / "morphir-avro-extension"
        external_crate = self.fixture.root.parent / "external-crate"
        shutil.rmtree(crate)
        external_crate.mkdir()
        (external_crate / "Cargo.toml").write_text(
            '[package]\nname = "morphir-avro-extension"\nversion = "0.1.0"\n',
            encoding="utf-8",
            newline="\n",
        )
        crate.symlink_to(external_crate, target_is_directory=True)

        result = self.fixture.package()

        self.assertNotEqual(0, result.returncode)
        self.assertIn("Cargo manifest escapes crates directory", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    def test_rejects_an_output_directory_symlink_even_when_bundle_matches(self) -> None:
        external = self.fixture.root.parent / "external-bundle"
        self.fixture.output = external
        first = self.fixture.package()
        self.assertEqual(0, first.returncode, first.stderr)
        artifact = external / "morphir-avro-extension-0.1.0.wasm"
        expected_bytes = artifact.read_bytes()
        symlink = self.fixture.root / "stage"
        symlink.symlink_to(external, target_is_directory=True)
        self.fixture.output = symlink

        result = self.fixture.package()

        self.assertNotEqual(0, result.returncode)
        self.assertIn("symbolic link", result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        self.assertEqual(expected_bytes, artifact.read_bytes())

    def test_rejects_a_symlink_in_an_output_ancestor(self) -> None:
        external = self.fixture.root.parent / "external-parent"
        self.fixture.output = external / "stage"
        first = self.fixture.package()
        self.assertEqual(0, first.returncode, first.stderr)
        (self.fixture.root / "linked").symlink_to(external, target_is_directory=True)
        self.fixture.output = self.fixture.root / "linked" / "stage"

        result = self.fixture.package()

        self.assertNotEqual(0, result.returncode)
        self.assertIn("symbolic link", result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        self.assertTrue((external / "stage/release.json").is_file())

    def test_output_parent_io_error_has_no_traceback(self) -> None:
        blocked_parent = self.fixture.root / "blocked"
        blocked_parent.write_text("not a directory", encoding="utf-8")
        self.fixture.output = blocked_parent / "stage"

        result = self.fixture.package()

        self.assertNotEqual(0, result.returncode)
        self.assertIn("output ancestor is not a directory", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    @unittest.skipIf(os.name == "nt", "POSIX permission behavior")
    def test_unreadable_wasm_io_error_has_no_traceback(self) -> None:
        self.fixture.wasm.chmod(0)
        try:
            result = self.fixture.package()
        finally:
            self.fixture.wasm.chmod(0o600)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("cannot read WASM file", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    @unittest.skipIf(os.name == "nt", "POSIX permission behavior")
    def test_unreadable_existing_bundle_io_error_has_no_traceback(self) -> None:
        first = self.fixture.package()
        self.assertEqual(0, first.returncode, first.stderr)
        artifact = self.fixture.output / "morphir-avro-extension-0.1.0.wasm"
        artifact.chmod(0)
        try:
            result = self.fixture.package()
        finally:
            artifact.chmod(0o600)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("cannot read staged bundle entry", result.stderr)
        self.assertNotIn("Traceback", result.stderr)
