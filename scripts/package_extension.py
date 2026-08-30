#!/usr/bin/env python3
"""Build a deterministic release bundle for a registered WASM extension."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import sys
import tempfile
import tomllib
from typing import Any


class PackageError(Exception):
    """Report invalid inputs without a Python traceback."""


IDENTIFIER_PATTERN = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
SEMVER_PATTERN = re.compile(
    r"^(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)"
    r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
    r"(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$"
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clean-avro-staging", action="store_true")
    parser.add_argument("--validate-avro-staging", action="store_true")
    parser.add_argument("--clean-head-snapshot", type=Path)
    parser.add_argument("--transfer-bundle", type=Path)
    parser.add_argument("--short-id")
    parser.add_argument("--wasm", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--git-commit")
    args = parser.parse_args()
    operation_count = sum(
        (
            args.clean_avro_staging,
            args.validate_avro_staging,
            args.clean_head_snapshot is not None,
            args.transfer_bundle is not None,
        )
    )
    if operation_count > 1:
        parser.error("only one cleanup or transfer mode may be selected")
    if (
        args.clean_avro_staging
        or args.validate_avro_staging
        or args.clean_head_snapshot is not None
    ):
        if any(
            value is not None
            for value in (
                args.short_id,
                args.wasm,
                args.output,
                args.git_commit,
                args.transfer_bundle,
            )
        ):
            parser.error("cleanup modes cannot be combined with other arguments")
    elif args.transfer_bundle is not None:
        if args.output is None:
            parser.error("the following arguments are required: --output")
        if any(value is not None for value in (args.short_id, args.wasm, args.git_commit)):
            parser.error("transfer mode cannot be combined with packaging arguments")
    else:
        missing = [
            option
            for option, value in (
                ("--short-id", args.short_id),
                ("--wasm", args.wasm),
                ("--output", args.output),
            )
            if value is None
        ]
        if missing:
            parser.error(f"the following arguments are required: {', '.join(missing)}")
    return args


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


def absolute_path(path: Path) -> Path:
    return Path(os.path.abspath(os.fspath(path)))


def lstat_optional(path: Path, label: str) -> os.stat_result | None:
    try:
        return path.lstat()
    except FileNotFoundError:
        return None
    except OSError as error:
        raise PackageError(f"cannot inspect {label} {path}: {error}") from error


def reject_symlink_components(path: Path, label: str) -> Path:
    absolute = absolute_path(path)
    current = Path(absolute.anchor)
    components = absolute.parts[1:]
    for index, component in enumerate(components):
        current /= component
        status = lstat_optional(current, f"{label} path component")
        if status is None:
            continue
        if stat.S_ISLNK(status.st_mode):
            raise PackageError(f"{label} contains symbolic link: {current}")
        if index < len(components) - 1 and not stat.S_ISDIR(status.st_mode):
            raise PackageError(f"{label} ancestor is not a directory: {current}")
    return absolute


def bundle_matches(output: Path, expected: dict[str, bytes]) -> bool:
    try:
        entries = list(output.iterdir())
    except OSError as error:
        raise PackageError(f"cannot inspect output directory {output}: {error}") from error

    if {entry.name for entry in entries} != set(expected):
        return False

    for entry in entries:
        status = lstat_optional(entry, "staged bundle entry")
        if status is None or not stat.S_ISREG(status.st_mode):
            return False
        try:
            contents = entry.read_bytes()
        except OSError as error:
            raise PackageError(
                f"cannot read staged bundle entry {entry}: {error}"
            ) from error
        if contents != expected[entry.name]:
            return False
    return True


def remove_temporary_bundle(temporary: Path) -> None:
    try:
        shutil.rmtree(temporary)
    except OSError as error:
        raise PackageError(
            f"cannot clean temporary bundle directory {temporary}: {error}"
        ) from error


def clean_repository_directory(
    root: Path, relative_components: tuple[str, ...], label: str
) -> None:
    current = root
    for index, component in enumerate(relative_components):
        current /= component
        try:
            status = current.lstat()
        except FileNotFoundError:
            return
        except OSError as error:
            raise PackageError(
                f"cannot clean {label}; cannot inspect {current}: {error}"
            ) from error
        if stat.S_ISLNK(status.st_mode):
            raise PackageError(
                f"cannot clean {label}; component is a symbolic link: {current}"
            )
        if not stat.S_ISDIR(status.st_mode):
            kind = "target" if index == len(relative_components) - 1 else "ancestor"
            raise PackageError(
                f"cannot clean {label}; {kind} is not a directory: {current}"
            )

    try:
        shutil.rmtree(current)
    except OSError as error:
        raise PackageError(f"cannot clean {label} {current}: {error}") from error


def validate_repository_directory(
    root: Path, relative_components: tuple[str, ...], label: str
) -> None:
    current = root
    for index, component in enumerate(relative_components):
        current /= component
        try:
            status = current.lstat()
        except FileNotFoundError:
            return
        except OSError as error:
            raise PackageError(
                f"cannot clean {label}; cannot inspect {current}: {error}"
            ) from error
        if stat.S_ISLNK(status.st_mode):
            raise PackageError(
                f"cannot clean {label}; component is a symbolic link: {current}"
            )
        if not stat.S_ISDIR(status.st_mode):
            kind = "target" if index == len(relative_components) - 1 else "ancestor"
            raise PackageError(
                f"cannot clean {label}; {kind} is not a directory: {current}"
            )
    try:
        for entry in current.iterdir():
            entry.lstat()
    except OSError as error:
        raise PackageError(f"cannot clean {label} {current}: {error}") from error


def validate_avro_staging(root: Path) -> None:
    validate_repository_directory(
        root,
        (".morphir", "build", "extensions", "avro"),
        "Avro staging directory",
    )


def clean_avro_staging(root: Path) -> None:
    clean_repository_directory(
        root,
        (".morphir", "build", "extensions", "avro"),
        "Avro staging directory",
    )


def clean_head_snapshot(root: Path, snapshot: Path) -> None:
    snapshot = absolute_path(snapshot)
    if not re.fullmatch(r"morphir-avro-head\.[A-Za-z0-9]+", snapshot.name):
        raise PackageError(f"refusing to clean unexpected HEAD snapshot path: {snapshot}")

    try:
        resolved_root = root.resolve(strict=True)
        resolved_snapshot = snapshot.resolve(strict=False)
    except OSError as error:
        raise PackageError(f"cannot resolve HEAD snapshot path {snapshot}: {error}") from error
    if resolved_snapshot == resolved_root or resolved_snapshot.is_relative_to(resolved_root):
        raise PackageError(f"HEAD snapshot must be outside repository: {snapshot}")

    status = lstat_optional(snapshot, "HEAD snapshot")
    if status is None:
        return
    if stat.S_ISLNK(status.st_mode):
        raise PackageError(f"refusing to clean symbolic-link HEAD snapshot: {snapshot}")
    if not stat.S_ISDIR(status.st_mode):
        raise PackageError(f"HEAD snapshot is not a directory: {snapshot}")
    try:
        shutil.rmtree(snapshot)
    except OSError as error:
        raise PackageError(f"cannot clean HEAD snapshot {snapshot}: {error}") from error


def verified_bundle(source: Path) -> dict[str, bytes]:
    source = reject_symlink_components(source, "source bundle")
    status = lstat_optional(source, "source bundle")
    if status is None or not stat.S_ISDIR(status.st_mode):
        raise PackageError(f"source bundle is not a directory: {source}")
    try:
        entries = list(source.iterdir())
    except OSError as error:
        raise PackageError(f"cannot inspect source bundle {source}: {error}") from error
    if len(entries) != 3 or "release.json" not in {entry.name for entry in entries}:
        raise PackageError(f"source bundle must contain exactly three release files: {source}")

    contents: dict[str, bytes] = {}
    for entry in entries:
        entry_status = lstat_optional(entry, "source bundle entry")
        if entry_status is None or not stat.S_ISREG(entry_status.st_mode):
            raise PackageError(f"source bundle entry is not a regular file: {entry}")
        try:
            contents[entry.name] = entry.read_bytes()
        except OSError as error:
            raise PackageError(f"cannot read source bundle entry {entry}: {error}") from error

    try:
        descriptor = json.loads(contents["release.json"].decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise PackageError(f"cannot parse source bundle release.json: {error}") from error
    if not isinstance(descriptor, dict):
        raise PackageError("source bundle release.json must contain a JSON object")
    artifact = descriptor.get("artifact")
    digest = descriptor.get("sha256")
    if (
        not isinstance(artifact, str)
        or not artifact
        or Path(artifact).name != artifact
        or "/" in artifact
        or "\\" in artifact
    ):
        raise PackageError("source bundle release.json has an invalid artifact name")
    checksum_name = f"{artifact}.sha256"
    if set(contents) != {artifact, checksum_name, "release.json"}:
        raise PackageError("source bundle files do not match release.json")
    actual_digest = hashlib.sha256(contents[artifact]).hexdigest()
    if not isinstance(digest, str) or digest != actual_digest:
        raise PackageError("bundle checksum mismatch")
    expected_checksum = f"{actual_digest}  {artifact}\n".encode("utf-8")
    if contents[checksum_name] != expected_checksum:
        raise PackageError("bundle checksum file mismatch")
    return contents


def write_bundle(output: Path, expected: dict[str, bytes]) -> None:
    output = reject_symlink_components(output, "output")
    output_status = lstat_optional(output, "output")
    if output_status is not None:
        if not stat.S_ISDIR(output_status.st_mode) or not bundle_matches(output, expected):
            raise PackageError(f"refusing to overwrite different staged bundle: {output}")
        return

    try:
        output.parent.mkdir(parents=True, exist_ok=True)
    except OSError as error:
        raise PackageError(f"cannot prepare bundle parent {output.parent}: {error}") from error
    reject_symlink_components(output.parent, "output")

    try:
        temporary = Path(tempfile.mkdtemp(prefix=f".{output.name}.", dir=output.parent))
    except OSError as error:
        raise PackageError(f"cannot create temporary bundle beside {output}: {error}") from error

    try:
        for name, contents in expected.items():
            (temporary / name).write_bytes(contents)
        reject_symlink_components(output, "output")
        if lstat_optional(output, "output") is not None:
            raise PackageError(f"refusing to overwrite newly staged bundle: {output}")
        os.replace(temporary, output)
    except PackageError:
        remove_temporary_bundle(temporary)
        raise
    except OSError as error:
        try:
            remove_temporary_bundle(temporary)
        except PackageError as cleanup_error:
            raise PackageError(
                f"cannot write bundle {output}: {error}; {cleanup_error}"
            ) from cleanup_error
        raise PackageError(f"cannot write bundle {output}: {error}") from error


def package(args: argparse.Namespace) -> None:
    root = Path(__file__).resolve().parents[1]
    extension = registered_extension(root, args.short_id)
    registered_package = require_identifier(extension.get("package"), "package")

    try:
        wasm_status = args.wasm.stat()
    except FileNotFoundError as error:
        raise PackageError(f"WASM file does not exist: {args.wasm}") from error
    except OSError as error:
        raise PackageError(f"cannot inspect WASM file {args.wasm}: {error}") from error
    if not stat.S_ISREG(wasm_status.st_mode):
        raise PackageError(f"WASM file does not exist: {args.wasm}")

    version = package_version(root, registered_package)
    try:
        wasm_bytes = args.wasm.read_bytes()
    except OSError as error:
        raise PackageError(f"cannot read WASM file {args.wasm}: {error}") from error
    expected = expected_bundle(
        args.short_id,
        extension,
        version,
        wasm_bytes,
        args.git_commit,
    )
    write_bundle(args.output, expected)


def transfer_bundle(source: Path, output: Path) -> None:
    write_bundle(output, verified_bundle(source))


def main() -> int:
    try:
        args = parse_args()
        root = Path(__file__).resolve().parents[1]
        if args.clean_avro_staging:
            clean_avro_staging(root)
        elif args.validate_avro_staging:
            validate_avro_staging(root)
        elif args.clean_head_snapshot is not None:
            clean_head_snapshot(root, args.clean_head_snapshot)
        elif args.transfer_bundle is not None:
            transfer_bundle(args.transfer_bundle, args.output)
        else:
            package(args)
    except PackageError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
