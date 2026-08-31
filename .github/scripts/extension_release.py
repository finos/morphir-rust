#!/usr/bin/env python3
"""Resolve an exact workspace or extension release tag into a build matrix."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import sys
import tomllib
from typing import Any, Mapping


class ReleaseError(Exception):
    """Report invalid release input without a traceback."""


SHORT_ID = r"[a-z0-9]+(?:-[a-z0-9]+)*"
SEMVER = (
    r"(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)"
    r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
    r"(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
)
WORKSPACE_TAG_PATTERN = re.compile(rf"^v(?P<version>{SEMVER})$")
EXTENSION_TAG_PATTERN = re.compile(
    rf"^extension/(?P<short_id>{SHORT_ID})/v(?P<version>{SEMVER})$"
)
SEMVER_PATTERN = re.compile(rf"^{SEMVER}$")
COMMIT_PATTERN = re.compile(r"^[0-9a-f]{40}(?:[0-9a-f]{24})?$")


@dataclass(frozen=True)
class Release:
    """The extensions selected by one exact release tag."""

    tag: str
    kind: str
    version: str
    short_ids: list[str]


def validate_semver(version: str, label: str) -> str:
    """Return a valid SemVer string, rejecting numeric prerelease zero padding."""
    match = SEMVER_PATTERN.fullmatch(version)
    if match is None:
        raise ReleaseError(f"invalid {label} version: {version}")
    prerelease = match.group(4)
    if prerelease is not None and any(
        identifier.isdigit()
        and len(identifier) > 1
        and identifier.startswith("0")
        for identifier in prerelease.split(".")
    ):
        raise ReleaseError(f"invalid {label} version: {version}")
    return version


def validate_commit(commit: str) -> str:
    """Return a full lowercase SHA-1 or SHA-256 Git object ID."""
    if COMMIT_PATTERN.fullmatch(commit) is None:
        raise ReleaseError(f"invalid peeled Git commit: {commit}")
    return commit


def extension_entries(registry: Mapping[str, Any]) -> Mapping[str, Any]:
    """Return the registry extension table after basic shape validation."""
    extensions = registry.get("extensions")
    if not isinstance(extensions, Mapping):
        raise ReleaseError("extension registry has no [extensions] table")
    return extensions


def registered_package(extension: Any, short_id: str) -> str:
    """Return the package name for a valid registry entry."""
    if not isinstance(extension, Mapping):
        raise ReleaseError(f"extension registry entry {short_id} must be a table")
    package = extension.get("package")
    if not isinstance(package, str) or not re.fullmatch(SHORT_ID, package):
        raise ReleaseError(f"extension {short_id} has an invalid package name")
    return package


def resolve_release(
    tag: str,
    registry: Mapping[str, Any],
    workspace_version: str,
    package_versions: Mapping[str, str],
) -> Release:
    """Resolve a tag after comparing it with the authoritative Cargo versions."""
    extensions = extension_entries(registry)
    workspace_match = WORKSPACE_TAG_PATTERN.fullmatch(tag)
    if workspace_match is not None:
        version = validate_semver(workspace_match.group("version"), "tag")
        expected = validate_semver(workspace_version, "workspace")
        if version != expected:
            raise ReleaseError(
                f"workspace tag version {version} does not match workspace version {expected}"
            )
        selected = []
        for short_id, extension in extensions.items():
            if not isinstance(short_id, str) or not re.fullmatch(SHORT_ID, short_id):
                raise ReleaseError(f"invalid extension short ID in registry: {short_id}")
            if not isinstance(extension, Mapping):
                raise ReleaseError(f"extension registry entry {short_id} must be a table")
            release_with_workspace = extension.get("release_with_workspace")
            if not isinstance(release_with_workspace, bool):
                raise ReleaseError(
                    f"extension {short_id} release_with_workspace must be a boolean"
                )
            if release_with_workspace:
                package = registered_package(extension, short_id)
                if package not in package_versions:
                    raise ReleaseError(f"missing Cargo version for package {package}")
                validate_semver(package_versions[package], f"package {package}")
                selected.append(short_id)
        if not selected:
            raise ReleaseError("workspace tag selects no extensions")
        return Release(tag, "workspace", version, sorted(selected))

    extension_match = EXTENSION_TAG_PATTERN.fullmatch(tag)
    if extension_match is None:
        raise ReleaseError(
            "invalid release tag; expected v<semver> or "
            "extension/<short-id>/v<semver>"
        )
    short_id = extension_match.group("short_id")
    extension = extensions.get(short_id)
    if extension is None:
        raise ReleaseError(f"unknown extension short ID in tag: {short_id}")
    package = registered_package(extension, short_id)
    expected = package_versions.get(package)
    if expected is None:
        raise ReleaseError(f"missing Cargo version for package {package}")
    version = validate_semver(extension_match.group("version"), "tag")
    expected = validate_semver(expected, f"package {package}")
    if version != expected:
        raise ReleaseError(
            f"extension tag version {version} does not match package version {expected}"
        )
    return Release(tag, "extension", version, [short_id])


def compact_matrix(
    release: Release,
    registry: Mapping[str, Any],
    package_versions: Mapping[str, str],
) -> str:
    """Serialize a stable GitHub Actions matrix without insignificant whitespace."""
    extensions = extension_entries(registry)
    include = []
    for short_id in sorted(release.short_ids):
        extension = extensions.get(short_id)
        package = registered_package(extension, short_id)
        version = package_versions.get(package)
        if version is None:
            raise ReleaseError(f"missing Cargo version for package {package}")
        include.append(
            {
                "short_id": short_id,
                "version": validate_semver(version, f"package {package}"),
            }
        )
    return json.dumps({"include": include}, separators=(",", ":"))


def load_toml(path: Path) -> dict[str, Any]:
    """Read one TOML document with a release-specific error."""
    try:
        with path.open("rb") as source:
            value = tomllib.load(source)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise ReleaseError(f"cannot read {path}: {error}") from error
    if not isinstance(value, dict):
        raise ReleaseError(f"TOML root in {path} must be a table")
    return value


def manifest_version(path: Path, table: str, expected_name: str | None = None) -> str:
    """Read a version from a Cargo manifest table."""
    manifest = load_toml(path)
    value: Any = manifest
    for component in table.split("."):
        value = value.get(component) if isinstance(value, Mapping) else None
    if not isinstance(value, Mapping):
        raise ReleaseError(f"Cargo manifest {path} has no [{table}] table")
    if expected_name is not None and value.get("name") != expected_name:
        raise ReleaseError(
            f"Cargo manifest {path} package name does not match {expected_name}"
        )
    version = value.get("version")
    if not isinstance(version, str):
        raise ReleaseError(f"Cargo manifest {path} has no explicit version in [{table}]")
    return validate_semver(version, f"Cargo manifest {path}")


def repository_release_data(
    root: Path,
) -> tuple[dict[str, Any], str, dict[str, str]]:
    """Load the registry plus workspace and registered package versions."""
    root = root.resolve()
    registry = load_toml(root / ".github" / "extensions.toml")
    workspace_version = manifest_version(root / "Cargo.toml", "workspace.package")
    package_versions = {}
    for short_id, extension in extension_entries(registry).items():
        if not isinstance(short_id, str):
            raise ReleaseError("extension registry short IDs must be strings")
        package = registered_package(extension, short_id)
        package_versions[package] = manifest_version(
            root / "crates" / package / "Cargo.toml", "package", package
        )
    return registry, workspace_version, package_versions


def github_outputs(fields: Mapping[str, str]) -> None:
    """Write single-line GitHub outputs, or stdout outside Actions."""
    destination = os.environ.get("GITHUB_OUTPUT")
    contents = "".join(f"{key}={value}\n" for key, value in fields.items())
    if destination is None:
        sys.stdout.write(contents)
        return
    try:
        with Path(destination).open("a", encoding="utf-8", newline="\n") as output:
            output.write(contents)
    except OSError as error:
        raise ReleaseError(f"cannot write GitHub output {destination}: {error}") from error


def parse_args() -> argparse.Namespace:
    """Parse command-line arguments."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    return parser.parse_args()


def main() -> int:
    """Resolve the requested tag and emit the workflow fields."""
    args = parse_args()
    try:
        registry, workspace_version, package_versions = repository_release_data(args.root)
        release = resolve_release(
            args.tag, registry, workspace_version, package_versions
        )
        commit = validate_commit(args.commit)
        github_outputs(
            {
                "tag": release.tag,
                "kind": release.kind,
                "version": release.version,
                "commit": commit,
                "matrix": compact_matrix(release, registry, package_versions),
            }
        )
    except ReleaseError as error:
        print(f"extension release error: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
