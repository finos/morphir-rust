"""Frontend release metadata keeps its languages through packaging and selection."""

from package_extension_test_support import *
from extension_release_test_support import AssetFixture, select_extension_assets


class FrontendExtensionReleaseTests(unittest.TestCase):
    def test_frontend_descriptor_survives_selection_and_cannot_be_dropped(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = AssetFixture(Path(temporary).resolve())
            _, checksum, descriptor_path = fixture.add_extension(
                short_id="rust", package="morphir-rust-binding",
                artifact_base="morphir-rust-binding", extension_id="morphir-rust",
                version="0.1.0", name="Morphir Rust",
            )
            # Checksums are an exact LF-delimited wire format on every platform.
            checksum.write_bytes(checksum.read_bytes().replace(b"\r\n", b"\n"))
            registry = fixture.root / ".github/extensions.toml"
            with registry.open("a", encoding="utf-8", newline="\n") as stream:
                stream.write('languages = [{ id = "rust", file_extensions = [".rs"] }]\n')
            descriptor = json.loads(descriptor_path.read_text(encoding="utf-8"))
            descriptor["artifacts"][0]["claims"] = claims_for_extension(
                tomllib.loads(registry.read_text())["extensions"]["rust"], "0.1.0"
            )
            before = (json.dumps(descriptor, indent=2) + "\n").encode("utf-8")
            descriptor_path.write_bytes(before)
            fixture.bundles = descriptor_path.parent
            selection = fixture.select("extension/rust/v0.1.0")
            uploaded = next(asset for asset in selection.uploads if asset.name.endswith(".release.json"))
            self.assertEqual(before, uploaded.source.read_bytes())
            del descriptor["artifacts"][0]["claims"]["capabilities"]["frontend"]["languages"]
            descriptor_path.write_bytes(json.dumps(descriptor).encode("utf-8"))
            with self.assertRaisesRegex(select_extension_assets.AssetError, "languages"):
                fixture.select("extension/rust/v0.1.0")

    def test_packaging_accepts_frontend_only_and_rejects_invalid_languages(self) -> None:
        registry = tomllib.loads(AVRO_REGISTRY)["extensions"]["avro"]
        registry["targets"] = []
        registry["languages"] = [{"id": "rust", "file_extensions": [".rs"]}]
        descriptor = json.loads(descriptor_bytes("rust", registry, "0.1.0", "guest.wasm", "0" * 64, None))
        self.assertNotIn("backend", descriptor["artifacts"][0]["claims"]["capabilities"])
        for languages in [[], [{"id": "rust", "file_extensions": ["rs"]}],
                          [{"id": "rust", "file_extensions": [".rs", ".rs"]}],
                          [{"id": "rust", "file_extensions": [".rs"]}] * 2]:
            with self.subTest(languages=languages), self.assertRaises(PackageError):
                descriptor_bytes("rust", {**registry, "languages": languages}, "0.1.0", "guest.wasm", "0" * 64, None)


if __name__ == "__main__":
    unittest.main()
