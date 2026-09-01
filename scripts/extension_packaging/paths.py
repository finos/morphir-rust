"""Path hardening and repository cleanup operations."""

from __future__ import annotations

import os
from pathlib import Path
import re
import shutil
import stat

from .errors import PackageError
from .model import require_identifier


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


def validate_extension_staging(root: Path, short_id: str) -> None:
    short_id = require_identifier(short_id, "short ID")
    validate_repository_directory(
        root,
        (".morphir", "build", "extensions", short_id),
        f"{short_id} staging directory",
    )


def clean_extension_staging(root: Path, short_id: str) -> None:
    short_id = require_identifier(short_id, "short ID")
    clean_repository_directory(
        root,
        (".morphir", "build", "extensions", short_id),
        f"{short_id} staging directory",
    )


def clean_head_snapshot(root: Path, snapshot: Path, short_id: str) -> None:
    short_id = require_identifier(short_id, "short ID")
    snapshot = absolute_path(snapshot)
    expected = re.compile(rf"morphir-{re.escape(short_id)}-head\.[A-Za-z0-9]+")
    if not expected.fullmatch(snapshot.name):
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
