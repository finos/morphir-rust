"""Materialize validated assets and publish atomic manifests."""

from __future__ import annotations

import os
from pathlib import Path
import stat
import tempfile

from .model import AssetError, AssetSelection, ReleaseAsset

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
