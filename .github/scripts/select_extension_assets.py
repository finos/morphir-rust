#!/usr/bin/env python3
"""Validate downloaded extension bundles and select assets safe to upload."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import sys
import tempfile
from types import ModuleType
from typing import Any, Mapping


class AssetError(Exception):
    """Report an invalid or conflicting release asset without a traceback."""


SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")


@dataclass(frozen=True)
class ReleaseAsset:
    """An unchanged bundle file and its deterministic published asset name."""

    source: Path
    name: str


@dataclass(frozen=True)
class AssetSelection:
    """Validated new assets and byte-identical assets already published."""

    uploads: list[ReleaseAsset]
    skipped: list[str]
    expected: list[ReleaseAsset]


def load_release_module() -> ModuleType:
    """Load the sibling tag-routing helper without changing sys.path."""
    path = Path(__file__).with_name("extension_release.py")
    spec = importlib.util.spec_from_file_location("extension_release", path)
    if spec is None or spec.loader is None:
        raise AssetError(f"cannot load release routing helper {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules.setdefault("extension_release", module)
    try:
        spec.loader.exec_module(module)
    except OSError as error:
        raise AssetError(f"cannot load release routing helper {path}: {error}") from error
    return module


extension_release = load_release_module()


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
    return {
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
        "gitCommit": expected_commit,
    }


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


def absolute_path(path: Path) -> Path:
    """Make a path absolute without resolving symbolic links."""
    return Path(os.path.abspath(os.fspath(path)))


def reject_symlink_components(path: Path, label: str) -> Path:
    """Reject symbolic links and non-directory ancestors in an output path."""
    absolute = absolute_path(path)
    current = Path(absolute.anchor)
    for component in absolute.parts[1:]:
        current /= component
        try:
            status = current.lstat()
        except FileNotFoundError:
            continue
        except OSError as error:
            raise AssetError(f"cannot inspect {label} component {current}: {error}") from error
        if stat.S_ISLNK(status.st_mode):
            raise AssetError(f"{label} contains symbolic link: {current}")
        if current != absolute and not stat.S_ISDIR(status.st_mode):
            raise AssetError(f"{label} ancestor is not a directory: {current}")
    return absolute


def require_absent_output(path: Path, label: str) -> Path:
    """Validate output components and require the final path not to exist."""
    absolute = reject_symlink_components(path, label)
    try:
        absolute.lstat()
    except FileNotFoundError:
        return absolute
    except OSError as error:
        raise AssetError(f"cannot inspect {label} {absolute}: {error}") from error
    raise AssetError(f"{label} already exists: {absolute}")


def create_directory_chain(path: Path, label: str) -> None:
    """Create missing output ancestors while rechecking every component."""
    current = Path(path.anchor)
    for component in path.parts[1:]:
        current /= component
        try:
            current.mkdir()
        except FileExistsError:
            pass
        except OSError as error:
            raise AssetError(f"cannot create {label} directory {current}: {error}") from error
        try:
            status = current.lstat()
        except OSError as error:
            raise AssetError(f"cannot inspect {label} directory {current}: {error}") from error
        if stat.S_ISLNK(status.st_mode):
            raise AssetError(f"{label} contains symbolic link: {current}")
        if not stat.S_ISDIR(status.st_mode):
            raise AssetError(f"{label} component is not a directory: {current}")


def materialize_assets(directory: Path, uploads: list[ReleaseAsset]) -> list[Path]:
    """Copy validated source bytes to their collision-free publication names."""
    try:
        directory.mkdir()
    except OSError as error:
        raise AssetError(f"cannot create prepared assets directory {directory}: {error}") from error
    prepared = []
    for asset in uploads:
        destination = directory / asset.name
        try:
            contents = asset.source.read_bytes()
            with destination.open("xb") as output:
                output.write(contents)
        except OSError as error:
            raise AssetError(
                f"cannot prepare release asset {destination} from {asset.source}: {error}"
            ) from error
        prepared.append(destination.resolve())
    return prepared


def write_manifest_atomic(path: Path, uploads: list[Path]) -> None:
    """Publish a complete regular manifest atomically without overwriting."""
    contents = "".join(f"{asset}\n" for asset in uploads)
    temporary_name: str | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w",
            encoding="utf-8",
            newline="\n",
            dir=path.parent,
            prefix=f".{path.name}.",
            delete=False,
        ) as output:
            temporary_name = output.name
            output.write(contents)
            output.flush()
            os.fsync(output.fileno())
        os.link(temporary_name, path)
        path_status = path.lstat()
        if not stat.S_ISREG(path_status.st_mode):
            raise AssetError(f"asset manifest is not a regular file: {path}")
    except OSError as error:
        raise AssetError(f"cannot write asset manifest {path}: {error}") from error
    finally:
        if temporary_name is not None:
            try:
                Path(temporary_name).unlink()
            except FileNotFoundError:
                pass
            except OSError as error:
                raise AssetError(
                    f"cannot clean temporary asset manifest {temporary_name}: {error}"
                ) from error


def prepare_publication(
    directory: Path,
    manifest: Path,
    expected_manifest: Path,
    selection: AssetSelection,
) -> list[Path]:
    """Prepare every exact asset and atomically publish upload and expected lists."""
    directory = require_absent_output(directory, "prepared assets directory")
    manifest = require_absent_output(manifest, "asset manifest")
    expected_manifest = require_absent_output(
        expected_manifest, "expected asset manifest"
    )
    create_directory_chain(directory.parent, "prepared assets")
    create_directory_chain(manifest.parent, "asset manifest")
    create_directory_chain(expected_manifest.parent, "expected asset manifest")
    expected_paths = materialize_assets(directory, selection.expected)
    prepared_by_name = {path.name: path for path in expected_paths}
    upload_paths = [prepared_by_name[asset.name] for asset in selection.uploads]
    write_manifest_atomic(manifest, upload_paths)
    write_manifest_atomic(expected_manifest, expected_paths)
    return upload_paths


def parse_args() -> argparse.Namespace:
    """Parse command-line arguments."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--bundles", type=Path, required=True)
    parser.add_argument("--existing-assets", type=Path)
    parser.add_argument("--output-manifest", type=Path)
    parser.add_argument("--expected-manifest", type=Path)
    parser.add_argument("--prepared-assets", type=Path)
    parser.add_argument("--require-all-existing", action="store_true")
    parser.add_argument("--validate-short-id")
    args = parser.parse_args()
    if args.validate_short_id is not None:
        if any(
            value is not None
            for value in (
                args.existing_assets,
                args.output_manifest,
                args.expected_manifest,
                args.prepared_assets,
            )
        ) or args.require_all_existing:
            parser.error(
                "--validate-short-id cannot be combined with publication inputs"
            )
    elif args.require_all_existing:
        if args.existing_assets is None:
            parser.error("--require-all-existing requires --existing-assets")
        if any(
            value is not None
            for value in (
                args.output_manifest,
                args.expected_manifest,
                args.prepared_assets,
            )
        ):
            parser.error("final verification cannot create publication outputs")
    elif any(
        value is None
        for value in (
            args.existing_assets,
            args.output_manifest,
            args.expected_manifest,
            args.prepared_assets,
        )
    ):
        parser.error("publication selection requires all output manifests")
    return args


def main() -> int:
    """Validate bundles and write only paths that are safe to upload."""
    args = parse_args()
    try:
        if args.validate_short_id is not None:
            validate_matrix_bundle(
                args.tag,
                args.commit,
                args.validate_short_id,
                args.root,
                args.bundles,
            )
            return 0
        selection = select_assets(
            args.tag,
            args.commit,
            args.root,
            args.bundles,
            args.existing_assets,
            require_all_existing=args.require_all_existing,
        )
        if not args.require_all_existing:
            prepare_publication(
                args.prepared_assets,
                args.output_manifest,
                args.expected_manifest,
                selection,
            )
    except AssetError as error:
        print(f"extension asset error: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
