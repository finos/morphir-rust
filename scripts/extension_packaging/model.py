"""Registry validation and release descriptor construction."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re
import tomllib
from typing import Any

from .errors import PackageError


IDENTIFIER_PATTERN = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
SEMVER_PATTERN = re.compile(
    r"^(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)"
    r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
    r"(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$"
)


def load_toml(path: Path) -> dict[str, Any]:
    try:
        with path.open("rb") as source:
            return tomllib.load(source)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise PackageError(f"cannot read {path}: {error}") from error


def registered_extension(root: Path, short_id: str) -> dict[str, Any]:
    require_identifier(short_id, "short ID")
    registry = load_toml(root / ".github" / "extensions.toml")
    extensions = registry.get("extensions", {})
    extension = extensions.get(short_id) if isinstance(extensions, dict) else None
    if not isinstance(extension, dict):
        raise PackageError(f"unknown extension short ID: {short_id}")
    return extension


def package_version(root: Path, registered_package: str) -> str:
    crates = root / "crates"
    manifest_path = crates / registered_package / "Cargo.toml"
    try:
        resolved_crates = crates.resolve(strict=True)
        resolved_manifest = manifest_path.resolve(strict=True)
    except OSError as error:
        raise PackageError(f"cannot resolve Cargo manifest {manifest_path}: {error}") from error
    try:
        resolved_manifest.relative_to(resolved_crates)
    except ValueError as error:
        raise PackageError(
            f"Cargo manifest escapes crates directory: {manifest_path}"
        ) from error

    manifest = load_toml(resolved_manifest)
    package = manifest.get("package")
    if not isinstance(package, dict):
        raise PackageError(f"Cargo manifest for {registered_package} has no [package] table")

    cargo_name = package.get("name")
    if cargo_name != registered_package:
        raise PackageError(
            f"registry package {registered_package} does not match Cargo package {cargo_name}"
        )

    version = package.get("version")
    if not isinstance(version, str) or not version:
        raise PackageError(f"Cargo package {registered_package} has no explicit version")
    validate_semver(version)
    return version


def require_string(extension: dict[str, Any], key: str) -> str:
    value = extension.get(key)
    if not isinstance(value, str) or not value:
        raise PackageError(f"extension registry field {key} must be a non-empty string")
    return value


def require_identifier(value: object, label: str) -> str:
    if not isinstance(value, str) or not IDENTIFIER_PATTERN.fullmatch(value):
        raise PackageError(
            f"invalid extension {label}: expected lowercase ASCII letters and digits "
            "separated by single hyphens"
        )
    return value


def validate_semver(version: str) -> None:
    match = SEMVER_PATTERN.fullmatch(version)
    if match is None:
        raise PackageError(f"invalid Cargo SemVer: {version}")
    prerelease = match.group(4)
    if prerelease is not None and any(
        identifier.isdigit()
        and len(identifier) > 1
        and identifier.startswith("0")
        for identifier in prerelease.split(".")
    ):
        raise PackageError(f"invalid Cargo SemVer: {version}")


def require_string_list(extension: dict[str, Any], key: str) -> list[str]:
    value = extension.get(key)
    if not isinstance(value, list) or not value or not all(
        isinstance(item, str) and item for item in value
    ):
        raise PackageError(f"extension registry field {key} must be a non-empty string list")
    return value


def descriptor_bytes(
    short_id: str,
    extension: dict[str, Any],
    version: str,
    artifact_name: str,
    digest: str,
    git_commit: str | None,
) -> bytes:
    descriptor = {
        "schemaVersion": 1,
        "shortId": short_id,
        "extensionId": require_string(extension, "extension_id"),
        "package": require_string(extension, "package"),
        "version": version,
        "mepVersions": require_string_list(extension, "mep_versions"),
        "runtime": "wasm",
        "targets": require_string_list(extension, "targets"),
        "irVersions": require_string_list(extension, "ir_versions"),
        "artifact": artifact_name,
        "sha256": digest,
    }
    if git_commit is not None:
        descriptor["gitCommit"] = git_commit
    return (json.dumps(descriptor, indent=2) + "\n").encode("utf-8")


def expected_bundle(
    short_id: str,
    extension: dict[str, Any],
    version: str,
    wasm_bytes: bytes,
    git_commit: str | None,
) -> dict[str, bytes]:
    artifact_base = require_identifier(extension.get("artifact"), "artifact")
    artifact_name = f"{artifact_base}-{version}.wasm"
    digest = hashlib.sha256(wasm_bytes).hexdigest()
    return {
        artifact_name: wasm_bytes,
        f"{artifact_name}.sha256": f"{digest}  {artifact_name}\n".encode("utf-8"),
        "release.json": descriptor_bytes(
            short_id,
            extension,
            version,
            artifact_name,
            digest,
            git_commit,
        ),
    }
