"""Shared fixtures for extension release workflow tests."""

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
