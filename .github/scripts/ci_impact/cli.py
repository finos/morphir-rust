"""Command-line entry point used by ci.yml and by `mise run ci:impact`."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import subprocess
import sys
from typing import Sequence

from .classify import Plan, full_plan, load_extensions, plan_changes
from .config import load_config
from .graph import load_workspace
from .outputs import render_github, render_text


def is_zero_sha(value: str) -> bool:
    """Return whether value is a full SHA-1 or SHA-256 all-zero object ID."""
    return value in ("0" * 40, "0" * 64)


def changed_paths(root: Path, base: str, head: str) -> tuple[str, ...]:
    """Return changed repository-relative paths between two Git objects."""
    result = subprocess.run(
        ["git", "diff", "--name-only", "-z", "--no-renames", "--diff-filter=ACDMRTUXB", base, head, "--"],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )
    return tuple(path for path in result.stdout.split("\0") if path)


def _emit(text: str) -> None:
    output_path = os.environ.get("GITHUB_OUTPUT")
    if output_path:
        with open(output_path, "a", encoding="utf-8") as output:
            output.write(text)
    else:
        sys.stdout.write(text)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Classify which CI jobs a change set needs.")
    parser.add_argument("--root", type=Path, default=Path("."))
    parser.add_argument("--base")
    parser.add_argument("--head", default="HEAD")
    parser.add_argument("--full", action="store_true", help="force a full run")
    parser.add_argument("--format", choices=("github", "text"), default="github")
    return parser


def _build_plan(args: argparse.Namespace) -> Plan:
    root = args.root
    config = load_config(root / ".github" / "ci-impact.toml")
    workspace = load_workspace(root)
    extensions = load_extensions(root / ".github" / "extensions.toml")
    if args.full:
        return full_plan(config, workspace, extensions, "full run requested")
    if not args.base:
        raise ValueError("--base is required unless --full is given")
    if is_zero_sha(args.base):
        raise ValueError("base is an all-zero Git object ID")
    return plan_changes(changed_paths(root, args.base, args.head), config, workspace, extensions)


def _fallback_plan(root: Path, reason: str) -> Plan:
    """A plan that runs everything even when config or cargo are unavailable."""
    from .config import ImpactConfig
    from .graph import Workspace

    try:
        config = load_config(root / ".github" / "ci-impact.toml")
    except (OSError, ValueError):
        config = ImpactConfig((), frozenset(), (), frozenset(), ())
    try:
        extensions = load_extensions(root / ".github" / "extensions.toml")
    except (OSError, ValueError, KeyError):
        extensions = ()
    return full_plan(config, Workspace(frozenset(), frozenset(), {}), extensions, reason)


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        plan = _build_plan(args)
    except Exception as error:  # noqa: BLE001 - any failure must fail safe
        print(f"warning: unable to classify CI changes: {error}", file=sys.stderr)
        plan = _fallback_plan(args.root, f"classifier error: {error}")
    _emit(render_github(plan) if args.format == "github" else render_text(plan))
    return 0
