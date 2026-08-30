"""Tests for extension release routing and exact-byte publication."""

from __future__ import annotations

import importlib.util
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
RELEASE_SCRIPT = REPOSITORY_ROOT / ".github" / "scripts" / "extension_release.py"
SELECT_SCRIPT = (
    REPOSITORY_ROOT / ".github" / "scripts" / "select_extension_assets.py"
)
RELEASE_WORKFLOW = REPOSITORY_ROOT / ".github" / "workflows" / "release.yml"
CI_WORKFLOW = REPOSITORY_ROOT / ".github" / "workflows" / "ci.yml"
RELEASE_COMMIT = "0123456789abcdef0123456789abcdef01234567"


def load_script(path: Path, name: str):
    """Import a repository script as a module."""
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise AssertionError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


extension_release = load_script(RELEASE_SCRIPT, "extension_release_under_test")
select_extension_assets = load_script(SELECT_SCRIPT, "select_extension_assets_under_test")


REGISTRY = {
    "extensions": {
        "zeta": {
            "package": "morphir-zeta-extension",
            "artifact": "morphir-zeta-extension",
            "extension_id": "morphir-zeta",
            "mep_versions": ["0.1"],
            "targets": ["zeta"],
            "ir_versions": ["4"],
            "release_with_workspace": True,
        },
        "avro": {
            "package": "morphir-avro-extension",
            "artifact": "morphir-avro-extension",
            "extension_id": "morphir-avro",
            "mep_versions": ["0.1"],
            "targets": ["avro"],
            "ir_versions": ["3", "4"],
            "release_with_workspace": True,
        },
        "private": {
            "package": "morphir-private-extension",
            "artifact": "morphir-private-extension",
            "extension_id": "morphir-private",
            "mep_versions": ["0.1"],
            "targets": ["private"],
            "ir_versions": ["4"],
            "release_with_workspace": False,
        },
    }
}

PACKAGE_VERSIONS = {
    "morphir-avro-extension": "0.1.0",
    "morphir-private-extension": "1.2.3",
    "morphir-zeta-extension": "0.3.0",
}


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


class AssetFixture:
    """Create a repository plus one packaged extension bundle."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.bundles = root / "bundles"
        self.bundle = self.bundles / "extension-avro"
        self.existing = root / "existing"
        self.bundle.mkdir(parents=True)
        self.existing.mkdir()
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
        self.artifact_name = "morphir-avro-extension-0.1.0.wasm"
        self.artifact = self.bundle / self.artifact_name
        self.checksum = self.bundle / f"{self.artifact_name}.sha256"
        self.descriptor = self.bundle / "release.json"
        self.artifact.write_bytes(b"\x00asm\x01\x00\x00\x00release-fixture")
        digest = hashlib.sha256(self.artifact.read_bytes()).hexdigest()
        self.checksum.write_text(
            f"{digest}  {self.artifact_name}\n", encoding="utf-8"
        )
        self.descriptor.write_text(
            json.dumps(
                {
                    "schemaVersion": 1,
                    "shortId": "avro",
                    "extensionId": "morphir-avro",
                    "package": "morphir-avro-extension",
                    "version": "0.1.0",
                    "mepVersions": ["0.1"],
                    "runtime": "wasm",
                    "targets": ["avro"],
                    "irVersions": ["3", "4"],
                    "artifact": self.artifact_name,
                    "sha256": digest,
                    "gitCommit": RELEASE_COMMIT,
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

    def add_extension(
        self,
        *,
        short_id: str,
        package: str,
        artifact_base: str,
        extension_id: str,
        version: str,
    ) -> tuple[Path, Path, Path]:
        """Add another opted-in registry package and valid downloaded bundle."""
        registry_path = self.root / ".github" / "extensions.toml"
        with registry_path.open("a", encoding="utf-8", newline="\n") as registry:
            registry.write(
                textwrap.dedent(
                    f"""\

                    [extensions.{short_id}]
                    package = "{package}"
                    artifact = "{artifact_base}"
                    extension_id = "{extension_id}"
                    mep_versions = ["0.1"]
                    targets = ["{short_id}"]
                    ir_versions = ["4"]
                    release_with_workspace = true
                    """
                )
            )
        manifest = self.root / "crates" / package / "Cargo.toml"
        manifest.parent.mkdir(parents=True)
        manifest.write_text(
            f'[package]\nname = "{package}"\nversion = "{version}"\n',
            encoding="utf-8",
        )
        bundle = self.bundles / f"extension-{short_id}"
        bundle.mkdir()
        artifact_name = f"{artifact_base}-{version}.wasm"
        artifact = bundle / artifact_name
        checksum = bundle / f"{artifact_name}.sha256"
        descriptor_path = bundle / "release.json"
        artifact.write_bytes(
            b"\x00asm\x01\x00\x00\x00" + f"{short_id}-fixture".encode("ascii")
        )
        digest = hashlib.sha256(artifact.read_bytes()).hexdigest()
        checksum.write_text(f"{digest}  {artifact_name}\n", encoding="utf-8")
        descriptor_path.write_text(
            json.dumps(
                {
                    "schemaVersion": 1,
                    "shortId": short_id,
                    "extensionId": extension_id,
                    "package": package,
                    "version": version,
                    "mepVersions": ["0.1"],
                    "runtime": "wasm",
                    "targets": [short_id],
                    "irVersions": ["4"],
                    "artifact": artifact_name,
                    "sha256": digest,
                    "gitCommit": RELEASE_COMMIT,
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        return artifact, checksum, descriptor_path

    def select(
        self,
        tag: str = "extension/avro/v0.1.0",
        *,
        require_all_existing: bool = False,
    ):
        return select_extension_assets.select_assets(
            tag,
            RELEASE_COMMIT,
            self.root,
            self.bundles,
            self.existing,
            require_all_existing=require_all_existing,
        )

    def rewrite_descriptor(self, **updates: object) -> None:
        descriptor = json.loads(self.descriptor.read_text(encoding="utf-8"))
        descriptor.update(updates)
        self.descriptor.write_text(
            json.dumps(descriptor, indent=2) + "\n", encoding="utf-8"
        )


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


class ReleaseWorkflowDefinitionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.release_workflow = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        cls.ci_workflow = CI_WORKFLOW.read_text(encoding="utf-8")

    def job(self, name: str, next_name: str | None) -> str:
        start = f"  {name}:\n"
        self.assertIn(start, self.release_workflow)
        body = self.release_workflow.split(start, 1)[1]
        if next_name is not None:
            body = body.split(f"  {next_name}:\n", 1)[0]
        return body

    def run_script_for_step(self, name: str, root: Path, *, fail_refresh: bool):
        """Execute one workflow shell step with deterministic fake git and gh CLIs."""
        marker = f"      - name: {name}\n"
        section = self.release_workflow.split(marker, 1)[1]
        section = section.split("\n      - name: ", 1)[0]
        script = textwrap.dedent(section.split("        run: |\n", 1)[1])
        fake_bin = root / "bin"
        fake_bin.mkdir()
        log = root / "gh.log"
        state = root / "release-state"
        state.write_text("missing\n", encoding="utf-8")
        git = fake_bin / "git"
        git.write_text(
            textwrap.dedent(
                f"""\
                #!/bin/sh
                set -eu
                case "$1" in
                  fetch) exit 0 ;;
                  rev-parse) printf '%s\\n' '{RELEASE_COMMIT}' ;;
                  *) exit 64 ;;
                esac
                """
            ),
            encoding="utf-8",
        )
        git.chmod(0o755)
        jq = fake_bin / "jq"
        jq.write_text(
            "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$RELEASE_TAG\"\n",
            encoding="utf-8",
        )
        jq.chmod(0o755)
        gh = fake_bin / "gh"
        gh.write_text(
            textwrap.dedent(
                """\
                #!/bin/sh
                set -eu
                printf '%s\\n' "$*" >> "$FAKE_GH_LOG"
                if [ "$1" = "api" ]; then
                  case "$2" in
                    */releases/tags/*)
                      if [ "$(cat "$FAKE_RELEASE_STATE")" = "missing" ]; then
                        printf '%s\\n' 'gh: Not Found (HTTP 404)' >&2
                        exit 1
                      fi
                      if [ "$FAKE_FAIL_REFRESH" = "1" ]; then
                        printf '%s\\n' 'gh: API unavailable (HTTP 503)' >&2
                        exit 1
                      fi
                      printf '%s\\n' '123'
                      exit 0
                      ;;
                    */releases/123/assets)
                      exit 0
                      ;;
                    *) exit 65 ;;
                  esac
                fi
                if [ "$1" = "release" ] && [ "$2" = "create" ]; then
                  printf '%s\\n' 'created' > "$FAKE_RELEASE_STATE"
                  exit 0
                fi
                if [ "$1" = "release" ] && [ "$2" = "upload" ]; then
                  exit 0
                fi
                exit 66
                """
            ),
            encoding="utf-8",
        )
        gh.chmod(0o755)
        runner_temp = root / "runner"
        manifest_dir = runner_temp / "extension-release"
        manifest_dir.mkdir(parents=True)
        asset = root / "asset.wasm"
        asset.write_bytes(b"exact asset bytes")
        (manifest_dir / "assets.txt").write_text(
            f"{asset}\n", encoding="utf-8"
        )
        result = subprocess.run(
            ["bash", "-eu", "-o", "pipefail", "-c", script],
            check=False,
            capture_output=True,
            text=True,
            env={
                **os.environ,
                "PATH": f"{fake_bin}{os.pathsep}{os.environ['PATH']}",
                "FAKE_GH_LOG": str(log),
                "FAKE_RELEASE_STATE": str(state),
                "FAKE_FAIL_REFRESH": "1" if fail_refresh else "0",
                "GITHUB_REPOSITORY": "finos/morphir-rust",
                "GH_TOKEN": "test-token",
                "RELEASE_TAG": "v0.2.0",
                "EXPECTED_COMMIT": RELEASE_COMMIT,
                "RUNNER_TEMP": str(runner_temp),
            },
        )
        return result, log.read_text(encoding="utf-8")

    def test_triggers_only_supported_tags_and_manual_existing_tag_dispatch(self) -> None:
        self.assertIn('      - "v*"', self.release_workflow)
        self.assertIn('      - "extension/*/v*"', self.release_workflow)
        self.assertIn("workflow_dispatch:", self.release_workflow)
        self.assertIn("tag:\n        description: Existing release tag", self.release_workflow)
        self.assertIn("required: true", self.release_workflow)
        self.assertIn("inputs.tag || github.ref_name", self.release_workflow)
        self.assertIn('refs/tags/${RELEASE_TAG}^{commit}', self.release_workflow)
        self.assertNotIn("git tag ", self.release_workflow)

    def test_manual_dispatch_checkout_uses_fully_qualified_tag_ref(self) -> None:
        release_info = self.job("release-info", "create-extension-artifacts")
        self.assertIn("format('refs/tags/{0}', inputs.tag)", release_info)
        self.assertNotIn("ref: ${{ inputs.tag || github.ref }}", release_info)

    def test_fully_qualified_tag_ref_wins_over_same_named_branch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary).resolve()
            subprocess.run(["git", "init", "-q", str(repository)], check=True)
            subprocess.run(
                ["git", "config", "user.email", "release@example.invalid"],
                cwd=repository,
                check=True,
            )
            subprocess.run(
                ["git", "config", "user.name", "Release Test"],
                cwd=repository,
                check=True,
            )
            marker = repository / "marker"
            marker.write_text("tag\n", encoding="utf-8")
            subprocess.run(["git", "add", "marker"], cwd=repository, check=True)
            subprocess.run(["git", "commit", "-qm", "tag target"], cwd=repository, check=True)
            tag_commit = subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=repository, text=True
            ).strip()
            subprocess.run(["git", "tag", "collision"], cwd=repository, check=True)
            marker.write_text("branch\n", encoding="utf-8")
            subprocess.run(["git", "commit", "-qam", "branch target"], cwd=repository, check=True)
            branch_commit = subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=repository, text=True
            ).strip()
            subprocess.run(["git", "branch", "collision"], cwd=repository, check=True)

            peeled = subprocess.check_output(
                ["git", "rev-parse", "refs/tags/collision^{commit}"],
                cwd=repository,
                text=True,
            ).strip()

            self.assertEqual(tag_commit, peeled)
            self.assertNotEqual(branch_commit, peeled)

    def test_serializes_each_tag_without_cancelling_an_active_publication(self) -> None:
        before_jobs = self.release_workflow.split("jobs:\n", 1)[0]
        self.assertIn("concurrency:", before_jobs)
        self.assertIn(
            "group: release-${{ inputs.tag || github.ref_name }}", before_jobs
        )
        self.assertIn("cancel-in-progress: false", before_jobs)

    def test_release_info_peels_once_and_downstream_jobs_checkout_commit(self) -> None:
        release_info = self.job("release-info", "create-extension-artifacts")
        create = self.job("create-extension-artifacts", "publish-extensions")
        publish = self.job("publish-extensions", None)
        self.assertIn("RELEASE_COMMIT=\"$(", release_info)
        self.assertIn('git rev-parse --verify "$tag_ref"', release_info)
        self.assertIn('--commit "$RELEASE_COMMIT"', release_info)
        self.assertIn("commit: ${{ steps.release.outputs.commit }}", release_info)
        self.assertIn("ref: ${{ needs.release-info.outputs.commit }}", create)
        self.assertIn("ref: ${{ needs.release-info.outputs.commit }}", publish)
        self.assertNotIn("ref: ${{ needs.release-info.outputs.tag }}", create)
        self.assertNotIn("ref: ${{ needs.release-info.outputs.tag }}", publish)
        self.assertIn('--commit "$RELEASE_COMMIT"', create)
        self.assertIn('--commit "$RELEASE_COMMIT"', publish)

    def test_publish_rechecks_remote_tag_commit_before_any_mutation(self) -> None:
        publish = self.job("publish-extensions", None)
        self.assertIn("verify_remote_tag_commit", publish)
        self.assertIn('EXPECTED_COMMIT: ${{ needs.release-info.outputs.commit }}', publish)
        self.assertIn('git fetch --force --no-tags origin "refs/tags/$RELEASE_TAG"', publish)
        self.assertIn('tag moved from $EXPECTED_COMMIT to $actual_commit', publish)
        mutation = min(
            publish.index("gh release create"),
            publish.index("gh release upload"),
        )
        self.assertLess(publish.index("verify_remote_tag_commit"), mutation)

    def test_creation_jobs_have_read_only_contents_permissions(self) -> None:
        self.assertIn("permissions:\n  contents: read", self.release_workflow)
        release_info = self.job("release-info", "create-extension-artifacts")
        create = self.job("create-extension-artifacts", "publish-extensions")
        self.assertIn("permissions:\n      contents: read", release_info)
        self.assertIn("permissions:\n      contents: read", create)
        self.assertNotIn("contents: write", release_info)
        self.assertNotIn("contents: write", create)

    def test_creation_job_builds_validates_and_uploads_seven_day_artifact(self) -> None:
        create = self.job("create-extension-artifacts", "publish-extensions")
        self.assertIn("mise run \"extension:artifact:${SHORT_ID}\"", create)
        self.assertIn("--validate-short-id \"$SHORT_ID\"", create)
        self.assertIn("uses: actions/upload-artifact@v7", create)
        self.assertIn("name: extension-${{ matrix.short_id }}", create)
        self.assertIn("retention-days: 7", create)
        self.assertIn("if-no-files-found: error", create)

    def test_publish_job_downloads_exact_artifacts_and_never_builds(self) -> None:
        publish = self.job("publish-extensions", None)
        self.assertIn(
            "needs: [release-info, create-extension-artifacts]", publish
        )
        self.assertIn("permissions:\n      contents: write", publish)
        self.assertIn("uses: actions/download-artifact@v7", publish)
        self.assertIn("pattern: extension-*", publish)
        self.assertIn("select_extension_assets.py", publish)
        self.assertNotIn("cargo build", publish)
        self.assertNotIn("cargo test", publish)
        self.assertNotIn("mise run extension:artifact", publish)

    def test_publish_job_checks_existing_bytes_and_never_clobbers(self) -> None:
        publish = self.job("publish-extensions", None)
        self.assertIn("gh release download", publish)
        self.assertIn("--existing-assets", publish)
        self.assertIn("--prepared-assets", publish)
        self.assertIn("gh release create", publish)
        self.assertIn("gh release upload", publish)
        self.assertNotIn("--clobber", publish)
        self.assertIn('${RUNNER_TEMP}/extension-release/upload', publish)
        self.assertIn('${RUNNER_TEMP}/extension-release/assets.txt', publish)
        self.assertNotIn("--prepared-assets .release/", publish)
        self.assertLess(
            publish.index("select_extension_assets.py"),
            publish.index("gh release create"),
        )

    def test_empty_existing_release_is_a_valid_state(self) -> None:
        publish = self.job("publish-extensions", None)
        self.assertIn("gh api", publish)
        self.assertIn("--jq '.[].name'", publish)
        self.assertIn('if [ -s "$asset_list" ]; then', publish)
        self.assertIn("gh release download", publish)
        self.assertIn("(HTTP 404)", publish)
        self.assertIn("cat \"$release_error\" >&2", publish)
        self.assertLess(
            publish.index('if [ -s "$asset_list" ]; then'),
            publish.index("gh release download"),
        )

    def test_all_asset_lookups_use_paginated_release_id_endpoint(self) -> None:
        publish = self.job("publish-extensions", None)
        self.assertIn('release_id="$(cat', publish)
        self.assertIn('releases/${release_id}/assets', publish)
        self.assertGreaterEqual(publish.count("--paginate"), 2)
        self.assertNotIn("--jq '.assets | length'", publish)
        self.assertNotIn(".assets[] | select", publish)

    def test_final_check_compares_every_expected_asset_and_rechecks_tag(self) -> None:
        publish = self.job("publish-extensions", None)
        self.assertIn("--expected-manifest", publish)
        self.assertIn("Final verification of every expected asset", publish)
        final = publish.split("      - name: Final verification of every expected asset\n", 1)[1]
        self.assertIn("verify_remote_tag_commit", final)
        self.assertIn("--require-all-existing", final)
        self.assertIn("missing or duplicated", final)
        self.assertIn("extension-release/expected-assets.txt", final)

    def test_mutation_rechecks_release_and_retries_upload_without_clobber(self) -> None:
        publish = self.job("publish-extensions", None)
        mutation = publish.split(
            "      - name: Create release and upload only new assets\n", 1
        )[1]
        self.assertIn("refresh_release_state", mutation)
        self.assertIn("verify_remote_tag_commit\n            gh release create", mutation)
        self.assertIn("verify_remote_tag_commit\n              if ! gh release upload", mutation)
        self.assertIn("compare_published_asset", mutation)
        self.assertIn("upload raced with a different asset", mutation)
        self.assertGreaterEqual(mutation.count("verify_remote_tag_commit"), 3)
        self.assertIn("compare_status=$?", mutation)
        self.assertIn("cannot inspect published asset", mutation)
        self.assertIn("return 2", mutation)
        self.assertIn("case \"$compare_status\" in", mutation)
        self.assertNotIn("--clobber", mutation)

    def test_fresh_release_refreshes_id_then_looks_up_and_uploads_asset(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            result, log = self.run_script_for_step(
                "Create release and upload only new assets",
                Path(temporary).resolve(),
                fail_refresh=False,
            )

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(2, log.count("releases/tags/v0.2.0"), log)
        self.assertIn("release create v0.2.0", log)
        self.assertIn("releases/123/assets --paginate", log)
        self.assertIn("release upload v0.2.0", log)

    def test_fresh_release_stops_when_post_create_state_refresh_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            result, log = self.run_script_for_step(
                "Create release and upload only new assets",
                Path(temporary).resolve(),
                fail_refresh=True,
            )

        self.assertNotEqual(0, result.returncode)
        self.assertIn("API unavailable", result.stderr)
        self.assertIn("release create v0.2.0", log)
        self.assertNotIn("releases/123/assets", log)
        self.assertNotIn("release upload", log)

    def test_ci_python_workflow_tests_are_read_only(self) -> None:
        self.assertIn("permissions:\n  contents: read", self.ci_workflow)
        self.assertIn("test-release-workflow:", self.ci_workflow)
        ci_job = self.ci_workflow.split("  test-release-workflow:\n", 1)[1]
        self.assertIn("permissions:\n      contents: read", ci_job)
        self.assertIn("python3 -m unittest discover -s tests/ci -v", ci_job)
        self.assertNotIn("contents: write", ci_job)
        self.assertNotIn("secrets.", ci_job)


if __name__ == "__main__":
    unittest.main()
