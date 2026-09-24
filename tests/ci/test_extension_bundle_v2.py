"""Version-2 bundles preserve guest claims and reject registry drift."""

import argparse
from copy import deepcopy
import json
from pathlib import Path
import subprocess
from unittest.mock import patch

from package_extension_test_support import *
from extension_packaging.cli import package


def an_avro_claim_set(version="0.1.0"):
    return {
        "claimsVersion": "0.1.0-draft.2",
        "protocolVersions": ["0.1"],
        "extension": {"id": "morphir-avro", "name": "Morphir Avro", "version": version,
                      "types": ["backend"]},
        "capabilities": {"backend": {"targets": ["avro"], "irVersions": ["3", "4"],
                                     "generate": True}},
    }


class VersionTwoBundleTests(unittest.TestCase):
    def run_package(self, fixture, claims):
        def describe(command, **kwargs):
            self.assertEqual(["cargo", "run", "--quiet", "--locked", "-p",
                              "morphir-host-native", "--bin", "extension-claims", "--"], command[:-1])
            self.assertEqual(fixture.wasm.read_bytes(), Path(command[-1]).read_bytes())
            return subprocess.CompletedProcess(command, 0, json.dumps(claims))
        with patch("subprocess.run", side_effect=describe):
            package(fixture.root, argparse.Namespace(short_id="avro", wasm=fixture.wasm,
                    output=fixture.output, git_commit="abc123"))

    def test_packages_exact_guest_claims_in_portable_artifact(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = PackageFixture(Path(directory) / "repo")
            claims = an_avro_claim_set()
            claims["capabilities"]["backend"]["futureOptional"] = {"enabled": True}
            self.run_package(fixture, claims)
            descriptor = json.loads((fixture.output / "release.json").read_bytes())
            self.assertEqual("2.0.0-draft.2", descriptor["schemaVersion"])
            self.assertEqual({"schemaVersion", "shortId", "extensionId", "version", "artifacts", "gitCommit"}, set(descriptor))
            artifact, = descriptor["artifacts"]
            self.assertEqual({"runtime", "filename", "sha256", "claims"}, set(artifact))
            self.assertEqual("wasm", artifact["runtime"])
            self.assertEqual(claims, artifact["claims"])
            self.assertEqual(sha256(fixture.output / artifact["filename"]), artifact["sha256"])

    def test_claims_version_ignores_build_metadata_only(self):
        """The reader compares the draft exactly but ignores SemVer build metadata."""
        with tempfile.TemporaryDirectory() as directory:
            fixture = PackageFixture(Path(directory) / "repo")
            claims = an_avro_claim_set()
            claims["claimsVersion"] = "0.1.0-draft.2+guest.42"
            self.run_package(fixture, claims)
            descriptor = json.loads((fixture.output / "release.json").read_bytes())
            self.assertEqual("0.1.0-draft.2+guest.42",
                             descriptor["artifacts"][0]["claims"]["claimsVersion"])
        for version in ["0.1.0-draft.1", "0.1.0-draft.3", "0.1.0", 2]:
            with self.subTest(version=version), tempfile.TemporaryDirectory() as directory:
                fixture = PackageFixture(Path(directory) / "repo")
                claims = an_avro_claim_set()
                claims["claimsVersion"] = version
                with self.assertRaisesRegex(PackageError, "claimsVersion"):
                    self.run_package(fixture, claims)
                self.assertFalse(fixture.output.exists())

    def test_mismatched_claims_refuse_before_output(self):
        cases = [
            ("extension", "id", "wrong"), ("extension", "version", "9.0.0"),
            ("extension", "types", ["frontend", "backend"]),
            ("backend", "targets", ["wrong"]), ("backend", "irVersions", ["4"]),
            (None, "protocolVersions", ["9"]),
        ]
        for section, member, value in cases:
            with self.subTest(member=member), tempfile.TemporaryDirectory() as directory:
                fixture = PackageFixture(Path(directory) / "repo")
                claims = an_avro_claim_set()
                target = claims if section is None else claims["capabilities"]["backend"] if section == "backend" else claims[section]
                target[member] = value
                with self.assertRaisesRegex(PackageError, member):
                    self.run_package(fixture, claims)
                self.assertFalse(fixture.output.exists())

    def test_claims_tool_failure_refuses_without_traceback(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = PackageFixture(Path(directory) / "repo")
            with patch("subprocess.run", side_effect=subprocess.CalledProcessError(7, ["cargo"])):
                with self.assertRaisesRegex(PackageError, "claims"):
                    package(fixture.root, argparse.Namespace(short_id="avro", wasm=fixture.wasm,
                            output=fixture.output, git_commit=None))
            self.assertFalse(fixture.output.exists())

    def test_invalid_claims_output_refuses(self):
        for output in ("not JSON", "null", "[]", "{}"):
            with self.subTest(output=output), tempfile.TemporaryDirectory() as directory:
                fixture = PackageFixture(Path(directory) / "repo")
                with patch("subprocess.run", return_value=subprocess.CompletedProcess([], 0, output)):
                    with self.assertRaises(PackageError):
                        package(fixture.root, argparse.Namespace(short_id="avro", wasm=fixture.wasm,
                                output=fixture.output, git_commit=None))
                self.assertFalse(fixture.output.exists())

    def test_frontend_workspace_claims_must_match_declared_capabilities(self):
        from extension_claims_test_support import claims_for_extension
        from extension_packaging.model import descriptor_bytes as build_descriptor
        extension = tomllib.loads(EXTENSIONS_TOML.read_text())["extensions"]["gleam"]
        claims = claims_for_extension(extension, "0.2.0")
        mutations = [
            ("frontend", "languages", [{"id": "gleam", "fileExtensions": [".wrong"]}]),
            ("frontend", "irVersions", ["4"]),
            ("frontend", "incremental", False),
            ("workspace", "discover", False),
        ]
        for kind, member, value in mutations:
            with self.subTest(member=member):
                changed = deepcopy(claims)
                changed["capabilities"][kind][member] = value
                with self.assertRaisesRegex(PackageError, member):
                    build_descriptor("gleam", extension, "0.2.0", "gleam.wasm", "0" * 64, None, changed)
        del claims["capabilities"]["workspace"]
        with self.assertRaisesRegex(PackageError, "kinds"):
            build_descriptor("gleam", extension, "0.2.0", "gleam.wasm", "0" * 64, None, claims)

    def test_transfer_and_asset_selection_reject_non_packager_shapes(self):
        from extension_release_test_support import AssetFixture, select_extension_assets
        from extension_packaging.bundles import verified_bundle
        def artifact(value):
            return value["artifacts"][0]
        mutations = [
            lambda value: value.update(schemaVersion=1),
            lambda value: value.update(extra=True),
            lambda value: value.pop("version"),
            lambda value: value.update(artifacts=[]),
            lambda value: value["artifacts"].append(deepcopy(artifact(value))),
            lambda value: artifact(value).update(platform=None),
            lambda value: artifact(value).update(extra=True),
            lambda value: artifact(value).pop("claims"),
            lambda value: artifact(value).pop("sha256"),
            lambda value: artifact(value).update(sha256="0" * 64),
            lambda value: artifact(value).update(filename="../escape.wasm"),
        ]
        for mutate in mutations:
            with self.subTest(mutation=mutate), tempfile.TemporaryDirectory() as directory:
                fixture = AssetFixture(Path(directory).resolve())
                descriptor = json.loads(fixture.descriptor.read_bytes())
                mutate(descriptor)
                fixture.descriptor.write_text(json.dumps(descriptor))
                with self.assertRaises(PackageError):
                    verified_bundle(fixture.bundle)
                with self.assertRaises(select_extension_assets.AssetError):
                    fixture.select()

    def test_optional_language_metadata_survives(self):
        from extension_packaging.model import descriptor_bytes as build_descriptor
        extension = tomllib.loads(EXTENSIONS_TOML.read_text())["extensions"]["gleam"]
        claims = claims_for_extension(extension, "0.2.0")
        claims["capabilities"]["frontend"]["languages"][0]["displayName"] = "Gleam"
        descriptor = json.loads(build_descriptor("gleam", extension, "0.2.0", "gleam.wasm",
                                                 "0" * 64, None, claims))
        self.assertEqual(claims, descriptor["artifacts"][0]["claims"])

    def test_malformed_known_capability_members_refuse(self):
        from extension_packaging.model import descriptor_bytes as build_descriptor
        extension = tomllib.loads(EXTENSIONS_TOML.read_text())["extensions"]["gleam"]
        cases = [("frontend", "fragments", "false"), ("frontend", "multiDocument", 1),
                 ("workspace", "protocolVersions", 42), ("workspace", "protocolVersions", ["0.1"])]
        for kind, member, value in cases:
            with self.subTest(member=member):
                claims = claims_for_extension(extension, "0.2.0")
                claims["capabilities"][kind][member] = value
                with self.assertRaisesRegex(PackageError, member):
                    build_descriptor("gleam", extension, "0.2.0", "gleam.wasm", "0" * 64, None, claims)

    def test_asset_selection_preserves_optional_claims_and_rejects_registry_drift(self):
        from extension_release_test_support import AssetFixture, select_extension_assets
        with tempfile.TemporaryDirectory() as directory:
            fixture = AssetFixture(Path(directory).resolve())
            descriptor = json.loads(fixture.descriptor.read_bytes())
            backend = descriptor["artifacts"][0]["claims"]["capabilities"]["backend"]
            backend["futureOptional"] = {"enabled": True}
            original = (json.dumps(descriptor, indent=2) + "\n").encode()
            fixture.descriptor.write_bytes(original)
            selected = fixture.select()
            asset = next(asset for asset in selected.uploads if asset.name.endswith(".release.json"))
            self.assertEqual(original, asset.source.read_bytes())
            backend["targets"] = ["wrong"]
            fixture.descriptor.write_text(json.dumps(descriptor))
            with self.assertRaisesRegex(select_extension_assets.AssetError, "targets"):
                fixture.select()
