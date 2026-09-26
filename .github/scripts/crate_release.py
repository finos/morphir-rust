#!/usr/bin/env python3
"""Resolve a crates/<name>/v<semver> tag into the crate to publish to crates.io."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import os
from pathlib import Path
import re
import subprocess
import sys
from typing import Any, Mapping

from extension_release import (
    COMMIT_PATTERN,
    SEMVER,
    ReleaseError,
    load_toml,
    validate_semver,
)


class CrateReleaseError(ReleaseError):
    """Report an invalid crate release without a traceback."""


CRATE_NAME = r"[a-z0-9]+(?:[-_][a-z0-9]+)*"
CRATE_TAG_PATTERN = re.compile(rf"^crates/(?P<crate>{CRATE_NAME})/v(?P<version>{SEMVER})$")


@dataclass(frozen=True)
class CrateRelease:
    """The crate and version named by one crate tag."""

    tag: str
    crate: str
    version: str


def parse_tag(tag: str) -> tuple[str, str]:
    """Split a crate tag into its crate name and SemVer version."""
    match = CRATE_TAG_PATTERN.fullmatch(tag)
    if match is None:
        raise CrateReleaseError(
            f"invalid crate tag {tag}; expected crates/<crate>/v<semver>"
        )
    try:
        version = validate_semver(match.group("version"), "tag")
    except ReleaseError as error:
        raise CrateReleaseError(str(error)) from error
    require_pre_1_0(version)
    return match.group("crate"), version


def require_pre_1_0(version: str) -> None:
    """Refuse 1.x and later: crates stay on 0.x.y until Morphir itself reaches 1.0.

    See docs/contributors/publishing-crates.md. Lift this rule when Morphir 1.0.0
    is released.
    """
    if not version.startswith("0."):
        raise CrateReleaseError(
            f"version {version} is 1.0 or above; crates stay on 0.x.y before Morphir 1.0 "
            "(docs/contributors/publishing-crates.md)"
        )


def read_toml(path: Path) -> dict[str, Any]:
    """Read a TOML file, reporting failures as crate release errors."""
    try:
        return load_toml(path)
    except ReleaseError as error:
        raise CrateReleaseError(str(error)) from error


def crate_version(root: Path, crate: str) -> str:
    """Return the version the crate's manifest declares, following workspace inheritance."""
    manifest_path = root / "crates" / crate / "Cargo.toml"
    if not manifest_path.is_file():
        raise CrateReleaseError(f"no crate at crates/{crate}")
    package = read_toml(manifest_path).get("package")
    if not isinstance(package, Mapping):
        raise CrateReleaseError(f"crates/{crate}/Cargo.toml has no [package] table")
    if package.get("name") != crate:
        raise CrateReleaseError(
            f"crates/{crate}/Cargo.toml package name does not match {crate}"
        )
    if package.get("publish") is False:
        raise CrateReleaseError(f"crate {crate} sets publish = false")
    version = package.get("version")
    if isinstance(version, Mapping) and version.get("workspace") is True:
        workspace = read_toml(root / "Cargo.toml").get("workspace")
        shared = workspace.get("package") if isinstance(workspace, Mapping) else None
        version = shared.get("version") if isinstance(shared, Mapping) else None
    if not isinstance(version, str):
        raise CrateReleaseError(f"crates/{crate}/Cargo.toml has no version")
    try:
        return validate_semver(version, f"crate {crate}")
    except ReleaseError as error:
        raise CrateReleaseError(str(error)) from error


def resolve_release(root: Path, tag: str) -> CrateRelease:
    """Resolve a tag and check that it names the crate's current version."""
    crate, version = parse_tag(tag)
    expected = crate_version(root.resolve(), crate)
    if version != expected:
        raise CrateReleaseError(
            f"tag version {version} does not match {crate} version {expected}"
        )
    return CrateRelease(tag, crate, version)


def require_commit_on_main(root: Path, commit: str, main_ref: str) -> None:
    """Fail unless the commit is reachable from the main branch ref."""
    if COMMIT_PATTERN.fullmatch(commit) is None:
        raise CrateReleaseError(f"invalid peeled Git commit: {commit}")
    try:
        result = subprocess.run(
            ["git", "-C", os.fspath(root), "merge-base", "--is-ancestor", commit, main_ref],
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as error:
        raise CrateReleaseError(f"cannot run git: {error}") from error
    if result.returncode == 1:
        raise CrateReleaseError(f"commit {commit} is not on main ({main_ref})")
    if result.returncode != 0:
        raise CrateReleaseError(
            f"cannot check commit {commit} against {main_ref}: {result.stderr.strip()}"
        )


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
        raise CrateReleaseError(f"cannot write GitHub output {destination}: {error}") from error


def parse_args() -> argparse.Namespace:
    """Parse command-line arguments."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--main-ref", default="origin/main")
    parser.add_argument("--root", type=Path, default=Path.cwd())
    return parser.parse_args()


def main() -> int:
    """Resolve the tag, check its commit and emit the workflow fields."""
    args = parse_args()
    try:
        release = resolve_release(args.root, args.tag)
        require_commit_on_main(args.root, args.commit, args.main_ref)
        github_outputs(
            {
                "tag": release.tag,
                "crate": release.crate,
                "version": release.version,
                "commit": args.commit,
            }
        )
    except ReleaseError as error:
        print(f"crate release error: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
