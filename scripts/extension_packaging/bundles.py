"""Bundle verification and atomic publication."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import tempfile

from .errors import PackageError
from .paths import lstat_optional, reject_symlink_components


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
