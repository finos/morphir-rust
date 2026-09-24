"""Python release metadata keeps both frontend and backend discovery."""

from package_extension_test_support import *
from extension_release_test_support import AssetFixture, extension_release, select_extension_assets
from ci_impact_test_support import classify


class PythonExtensionReleaseTests(unittest.TestCase):
    def test_frontend_descriptor_survives_selection_and_cannot_be_dropped(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = AssetFixture(Path(temporary).resolve())
            _, checksum, descriptor_path = fixture.add_extension(
                short_id="python", package="morphir-python-binding",
                artifact_base="morphir-python-binding", extension_id="morphir-python",
                version="0.1.0", name="Morphir Python",
            )
            # Checksums are an exact LF-delimited wire format on every platform.
            checksum.write_bytes(checksum.read_bytes().replace(b"\r\n", b"\n"))
            registry = fixture.root / ".github/extensions.toml"
            with registry.open("a", encoding="utf-8", newline="\n") as stream:
                stream.write('languages = [{ id = "python", file_extensions = [".py"] }]\n')
            descriptor = json.loads(descriptor_path.read_text(encoding="utf-8"))
            descriptor["artifacts"][0]["claims"] = claims_for_extension(
                tomllib.loads(registry.read_text())["extensions"]["python"], "0.1.0"
            )
            before = (json.dumps(descriptor, indent=2) + "\n").encode("utf-8")
            descriptor_path.write_bytes(before)
            fixture.bundles = descriptor_path.parent
            selection = fixture.select("extension/python/v0.1.0")
            uploaded = next(asset for asset in selection.uploads if asset.name.endswith(".release.json"))
            self.assertEqual(before, uploaded.source.read_bytes())
            del descriptor["artifacts"][0]["claims"]["capabilities"]["frontend"]["languages"]
            descriptor_path.write_bytes(json.dumps(descriptor).encode("utf-8"))
            with self.assertRaisesRegex(select_extension_assets.AssetError, "languages"):
                fixture.select("extension/python/v0.1.0")

    def test_python_tag_and_descriptor_preserve_both_capabilities(self) -> None:
        registry = tomllib.loads(EXTENSIONS_TOML.read_text(encoding="utf-8"))
        extension = registry["extensions"]["python"]
        release = extension_release.resolve_release(
            "extension/python/v0.1.0", registry, "0.2.0",
            {"morphir-python-binding": "0.1.0"},
        )
        self.assertEqual(["python"], release.short_ids)
        descriptor = json.loads(descriptor_bytes(
            "python", extension, "0.1.0", "morphir-python-binding-0.1.0.wasm", "0" * 64, "a" * 40,
        ))
        self.assertEqual("Morphir Python", descriptor["artifacts"][0]["claims"]["extension"]["name"])
        self.assertEqual(["python"], descriptor["artifacts"][0]["claims"]["capabilities"]["backend"]["targets"])
        self.assertEqual([{"id": "python", "fileExtensions": [".py"]}], descriptor["artifacts"][0]["claims"]["capabilities"]["frontend"]["languages"])
        self.assertEqual(["3", "4"], descriptor["artifacts"][0]["claims"]["capabilities"]["backend"]["irVersions"])

    def test_packaging_accepts_frontend_only_and_rejects_invalid_languages(self) -> None:
        registry = tomllib.loads(AVRO_REGISTRY)["extensions"]["avro"]
        registry["targets"] = []
        registry["languages"] = [{"id": "python", "file_extensions": [".py"]}]
        descriptor = json.loads(descriptor_bytes("python", registry, "0.1.0", "guest.wasm", "0" * 64, None))
        self.assertNotIn("backend", descriptor["artifacts"][0]["claims"]["capabilities"])
        for languages in [[], [{"id": "python", "file_extensions": ["py"]}],
                          [{"id": "python", "file_extensions": [".py", ".py"]}],
                          [{"id": "python", "file_extensions": [".py"]}] * 2]:
            with self.subTest(languages=languages), self.assertRaises(PackageError):
                descriptor_bytes("python", {**registry, "languages": languages}, "0.1.0", "guest.wasm", "0" * 64, None)

    def test_python_ci_publishes_a_downloadable_bundle(self) -> None:
        workflow = (REPOSITORY_ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        self.assertIn('mise run "extension:artifact:${{ matrix.id }}"', workflow)
        self.assertIn("name: morphir-${{ matrix.id }}-extension-bundle", workflow)
        self.assertIn("path: .morphir/build/extensions/${{ matrix.id }}/*", workflow)
        self.assertIn("include: ${{ fromJSON(needs.changes.outputs.extensions) }}", workflow)
        extensions = classify.load_extensions(REPOSITORY_ROOT / ".github/extensions.toml")
        self.assertIn(classify.Extension("python", "morphir-python-binding"), extensions)
