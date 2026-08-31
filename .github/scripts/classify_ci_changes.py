#!/usr/bin/env python3
"""Classify whether a change set can skip expensive CI jobs."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import subprocess
import sys
from typing import Sequence


SAFE_EXACT_PATHS = (
    "README.md",
    "CHANGELOG.md",
    "CONTRIBUTING.md",
    "MAINTAINERS.md",
    "LICENSE",
    "LICENSE.spdx",
    "NOTICE",
    "AGENTS.md",
    "renovate.json",
)
SAFE_PREFIXES = (
    ".beads/",
    "docs/",
    ".github/ISSUE_TEMPLATE/",
    ".github/PULL_REQUEST_TEMPLATE/",
)


def is_metadata_path(path: str) -> bool:
    """Return whether one repository-relative path is explicitly safe."""
    return path in SAFE_EXACT_PATHS or path.startswith(SAFE_PREFIXES)


def requires_expensive_ci(paths: Sequence[str]) -> bool:
    """Return whether the change set must run the full CI suite."""
    return not paths or any(not is_metadata_path(path) for path in paths)


def is_zero_sha(value: str) -> bool:
    """Return whether value is a full SHA-1 or SHA-256 all-zero object ID."""
    return value in ("0" * 40, "0" * 64)


def changed_paths(root: Path, base: str, head: str) -> tuple[str, ...]:
    """Return changed repository-relative paths between two Git objects."""
    result = subprocess.run(
        [
            "git",
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            "--diff-filter=ACDMRTUXB",
            base,
            head,
            "--",
        ],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )
    return tuple(path for path in result.stdout.split("\0") if path)


def _emit(expensive: bool) -> None:
    """Emit exactly one GitHub output field or the equivalent stdout value."""
    value = f"expensive={'true' if expensive else 'false'}\n"
    output_path = os.environ.get("GITHUB_OUTPUT")
    if output_path:
        with open(output_path, "a", encoding="utf-8") as output:
            output.write(value)
    else:
        sys.stdout.write(value)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path("."))
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    return parser


def main() -> int:
    args = _parser().parse_args()
    try:
        if is_zero_sha(args.base):
            raise ValueError("base is an all-zero Git object ID")
        paths = changed_paths(args.root, args.base, args.head)
        expensive = requires_expensive_ci(paths)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"warning: unable to classify CI changes: {error}", file=sys.stderr)
        expensive = True
    _emit(expensive)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
