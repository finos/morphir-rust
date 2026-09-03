"""Discover and validate exact extension release bundles."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import stat
from typing import Any, Mapping

from .model import (
    AssetError, AssetSelection, ReleaseAsset, SHA256_PATTERN, extension_release,
)

def regular_files(directory: Path, label: str) -> list[Path]:
    """Return direct regular files and reject all other entry types."""
    try:
        entries = list(directory.iterdir())
    except OSError as error:
        raise AssetError(f"cannot inspect {label} {directory}: {error}") from error
    files = []
    for entry in entries:
        try:
            status = entry.lstat()
        except OSError as error:
            raise AssetError(f"cannot inspect {label} entry {entry}: {error}") from error
        if stat.S_ISLNK(status.st_mode) or not stat.S_ISREG(status.st_mode):
            raise AssetError(f"{label} entry must be a regular file: {entry}")
        files.append(entry)
    return files


def descriptor_paths(bundles: Path) -> list[Path]:
    """Find one direct release descriptor per downloaded artifact directory."""
    try:
        root_status = bundles.lstat()
    except OSError as error:
        raise AssetError(f"cannot inspect bundles directory {bundles}: {error}") from error
    if stat.S_ISLNK(root_status.st_mode) or not stat.S_ISDIR(root_status.st_mode):
        raise AssetError(f"bundles path must be a directory: {bundles}")

    descriptors = []
    for current, directories, files in os.walk(bundles, followlinks=False):
        current_path = Path(current)
        for name in list(directories):
            child = current_path / name
            try:
                status = child.lstat()
            except OSError as error:
                raise AssetError(f"cannot inspect bundle directory {child}: {error}") from error
            if stat.S_ISLNK(status.st_mode) or not stat.S_ISDIR(status.st_mode):
                raise AssetError(f"bundle directory must not be a symbolic link: {child}")
        for name in files:
            child = current_path / name
            try:
                status = child.lstat()
            except OSError as error:
                raise AssetError(f"cannot inspect bundle entry {child}: {error}") from error
            if stat.S_ISLNK(status.st_mode) or not stat.S_ISREG(status.st_mode):
                raise AssetError(f"bundle entry must be a regular file: {child}")
            if name == "release.json":
                descriptors.append(child)
    if not descriptors:
        raise AssetError(f"no release.json descriptors found under {bundles}")
    return sorted(descriptors)


def read_descriptor(path: Path) -> dict[str, Any]:
    """Read one descriptor as a JSON object."""
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise AssetError(f"cannot read descriptor {path}: {error}") from error
    if not isinstance(value, dict):
        raise AssetError(f"release descriptor {path} must be a JSON object")
    return value


def expected_descriptor(
    short_id: str,
    extension: Mapping[str, Any],
    version: str,
    artifact_name: str,
    digest: str,
    expected_commit: str,
) -> dict[str, Any]:
    """Build the tag- and registry-derived descriptor fields."""
    expected: dict[str, Any] = {
        "schemaVersion": 1,
        "shortId": short_id,
        "extensionId": extension.get("extension_id"),
        "package": extension.get("package"),
        "version": version,
        "mepVersions": extension.get("mep_versions"),
        "runtime": "wasm",
        "targets": extension.get("targets"),
        "irVersions": extension.get("ir_versions"),
        "artifact": artifact_name,
        "sha256": digest,
    }
    if "name" in extension:
        expected["name"] = extension.get("name")
    expected["gitCommit"] = expected_commit
    return expected


def validate_bundle(
    descriptor_path: Path,
    release: Any,
    registry: Mapping[str, Any],
    package_versions: Mapping[str, str],
    expected_commit: str,
) -> tuple[str, list[ReleaseAsset]]:
    """Validate one complete three-file bundle against its selected release."""
    descriptor = read_descriptor(descriptor_path)
    short_id = descriptor.get("shortId")
    if not isinstance(short_id, str) or short_id not in release.short_ids:
        raise AssetError(
            f"descriptor {descriptor_path} shortId is not selected by tag {release.tag}"
        )
    extensions = extension_release.extension_entries(registry)
    extension = extensions.get(short_id)
    if not isinstance(extension, Mapping):
        raise AssetError(f"registry entry for {short_id} must be a table")
    package = extension_release.registered_package(extension, short_id)
    version = package_versions.get(package)
    if version is None:
        raise AssetError(f"missing package version for {package}")
    artifact_base = extension.get("artifact")
    if not isinstance(artifact_base, str) or not re.fullmatch(
        extension_release.SHORT_ID, artifact_base
    ):
        raise AssetError(f"extension {short_id} has an invalid artifact name")
    artifact_name = f"{artifact_base}-{version}.wasm"
    artifact_path = descriptor_path.parent / artifact_name
    checksum_path = descriptor_path.parent / f"{artifact_name}.sha256"
    expected_names = {artifact_name, f"{artifact_name}.sha256", "release.json"}
    files = regular_files(descriptor_path.parent, "bundle")
    actual_names = {path.name for path in files}
    if actual_names != expected_names or len(files) != len(expected_names):
        raise AssetError(
            f"bundle entries for {short_id} do not match expected files: "
            f"{sorted(expected_names)}"
        )
    try:
        artifact_bytes = artifact_path.read_bytes()
    except OSError as error:
        raise AssetError(f"cannot read extension artifact {artifact_path}: {error}") from error
    digest = hashlib.sha256(artifact_bytes).hexdigest()
    descriptor_digest = descriptor.get("sha256")
    if not isinstance(descriptor_digest, str) or not SHA256_PATTERN.fullmatch(
        descriptor_digest
    ):
        raise AssetError(f"descriptor SHA-256 is invalid for {short_id}")
    if descriptor_digest != digest:
        raise AssetError(f"artifact SHA-256 does not match descriptor for {short_id}")
    try:
        checksum_bytes = checksum_path.read_bytes()
    except OSError as error:
        raise AssetError(f"cannot read checksum {checksum_path}: {error}") from error
    expected_checksum = f"{digest}  {artifact_name}\n".encode("utf-8")
    if checksum_bytes != expected_checksum:
        raise AssetError(f"checksum file does not match artifact for {short_id}")

    expected = expected_descriptor(
        short_id, extension, version, artifact_name, digest, expected_commit
    )
    allowed_keys = set(expected)
    unexpected = set(descriptor) - allowed_keys
    missing = set(expected) - set(descriptor)
    if unexpected or missing:
        raise AssetError(
            f"descriptor fields for {short_id} are invalid; "
            f"missing={sorted(missing)} unexpected={sorted(unexpected)}"
        )
    for key, expected_value in expected.items():
        if descriptor.get(key) != expected_value:
            raise AssetError(
                f"descriptor {key} for {short_id} does not match tag or registry"
            )
    descriptor_asset_name = f"{artifact_name.removesuffix('.wasm')}.release.json"
    return short_id, [
        ReleaseAsset(artifact_path, artifact_name),
        ReleaseAsset(checksum_path, checksum_path.name),
        ReleaseAsset(descriptor_path, descriptor_asset_name),
    ]


def existing_asset_map(directory: Path) -> dict[str, Path]:
    """Index downloaded existing release assets by unique basename."""
    if not directory.exists():
        return {}
    try:
        status = directory.lstat()
    except OSError as error:
        raise AssetError(f"cannot inspect existing assets {directory}: {error}") from error
    if stat.S_ISLNK(status.st_mode) or not stat.S_ISDIR(status.st_mode):
        raise AssetError(f"existing assets path must be a directory: {directory}")
    indexed = {}
    for current, directories, files in os.walk(directory, followlinks=False):
        current_path = Path(current)
        for name in directories:
            child = current_path / name
            if child.is_symlink():
                raise AssetError(f"existing assets contain symbolic link: {child}")
        for name in files:
            child = current_path / name
            try:
                child_status = child.lstat()
            except OSError as error:
                raise AssetError(f"cannot inspect existing asset {child}: {error}") from error
            if stat.S_ISLNK(child_status.st_mode) or not stat.S_ISREG(
                child_status.st_mode
            ):
                raise AssetError(f"existing asset must be a regular file: {child}")
            if name in indexed:
                raise AssetError(f"duplicate existing release asset name: {name}")
            indexed[name] = child
    return indexed


def select_assets(
    tag: str,
    expected_commit: str,
    root: Path,
    bundles: Path,
    existing_assets: Path,
    *,
    require_all_existing: bool = False,
) -> AssetSelection:
    """Validate every selected bundle and refuse changed same-name assets."""
    try:
        expected_commit = extension_release.validate_commit(expected_commit)
        registry, workspace_version, package_versions = (
            extension_release.repository_release_data(root)
        )
        release = extension_release.resolve_release(
            tag, registry, workspace_version, package_versions
        )
    except extension_release.ReleaseError as error:
        raise AssetError(str(error)) from error

    selected_bundles: dict[str, list[ReleaseAsset]] = {}
    for descriptor_path in descriptor_paths(bundles):
        short_id, assets = validate_bundle(
            descriptor_path, release, registry, package_versions, expected_commit
        )
        if short_id in selected_bundles:
            raise AssetError(f"more than one bundle found for extension {short_id}")
        selected_bundles[short_id] = assets
    missing = sorted(set(release.short_ids) - set(selected_bundles))
    extra = sorted(set(selected_bundles) - set(release.short_ids))
    if missing or extra:
        raise AssetError(f"bundle selection mismatch; missing={missing} extra={extra}")

    existing = existing_asset_map(existing_assets)
    uploads = []
    skipped = []
    expected = []
    selected_names: dict[str, Path] = {}
    for short_id in sorted(selected_bundles):
        for asset in selected_bundles[short_id]:
            previous = selected_names.get(asset.name)
            if previous is not None:
                raise AssetError(
                    f"selected bundles contain duplicate release asset name {asset.name}: "
                    f"{previous} and {asset.source}"
                )
            selected_names[asset.name] = asset.source
            expected_asset = ReleaseAsset(asset.source.resolve(), asset.name)
            expected.append(expected_asset)
            published = existing.get(asset.name)
            if published is None:
                uploads.append(expected_asset)
                continue
            try:
                same_bytes = published.read_bytes() == asset.source.read_bytes()
            except OSError as error:
                raise AssetError(f"cannot compare existing asset {published}: {error}") from error
            if not same_bytes:
                raise AssetError(
                    f"refusing to replace existing release asset with different bytes: "
                    f"{asset.name}"
                )
            skipped.append(asset.name)
    if require_all_existing and uploads:
        missing_names = [asset.name for asset in uploads]
        raise AssetError(f"missing expected release assets: {missing_names}")
    return AssetSelection(uploads, skipped, expected)


def validate_matrix_bundle(
    tag: str,
    expected_commit: str,
    short_id: str,
    root: Path,
    bundles: Path,
) -> None:
    """Validate exactly one creation-job bundle for a selected matrix entry."""
    try:
        expected_commit = extension_release.validate_commit(expected_commit)
        registry, workspace_version, package_versions = (
            extension_release.repository_release_data(root)
        )
        release = extension_release.resolve_release(
            tag, registry, workspace_version, package_versions
        )
    except extension_release.ReleaseError as error:
        raise AssetError(str(error)) from error
    if short_id not in release.short_ids:
        raise AssetError(f"extension {short_id} is not selected by tag {tag}")
    descriptors = descriptor_paths(bundles)
    if len(descriptors) != 1:
        raise AssetError(
            f"matrix entry {short_id} must contain exactly one release descriptor"
        )
    actual_short_id, _ = validate_bundle(
        descriptors[0], release, registry, package_versions, expected_commit
    )
    if actual_short_id != short_id:
        raise AssetError(
            f"matrix entry {short_id} contains descriptor for {actual_short_id}"
        )
