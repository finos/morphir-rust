"""Workspace dependency graph derived from `cargo metadata --no-deps`."""

from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path
import shutil
import subprocess
from typing import Any, Iterable, Mapping


@dataclass(frozen=True)
class Workspace:
    members: frozenset[str]
    default_members: frozenset[str]
    dependents: Mapping[str, frozenset[str]]


def workspace_from_metadata(metadata: Mapping[str, Any]) -> Workspace:
    """Build the reverse dependency map over workspace members only.

    Dev and build dependencies count as edges: a change to a crate must
    re-run the tests of every crate that links it in any way.
    """
    packages = metadata["packages"]
    name_by_id = {package["id"]: package["name"] for package in packages}
    members = frozenset(name_by_id[item] for item in metadata["workspace_members"])
    default_members = frozenset(
        name_by_id[item] for item in metadata.get("workspace_default_members", metadata["workspace_members"])
    )
    dependents: dict[str, set[str]] = {name: set() for name in members}
    for package in packages:
        if package["name"] not in members:
            continue
        for dependency in package["dependencies"]:
            target = dependency["name"]
            if target in members:
                dependents[target].add(package["name"])
    return Workspace(
        members=members,
        default_members=default_members,
        dependents={name: frozenset(values) for name, values in dependents.items()},
    )


def load_workspace(root: Path) -> Workspace:
    # shutil.which honours PATHEXT on Windows, so a cargo.cmd stand-in on PATH
    # is found by tests; subprocess alone would only look for cargo.exe.
    cargo = shutil.which("cargo") or "cargo"
    result = subprocess.run(
        [cargo, "metadata", "--format-version", "1", "--no-deps", "--offline"],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )
    return workspace_from_metadata(json.loads(result.stdout))


def affected_closure(workspace: Workspace, changed: Iterable[str]) -> frozenset[str]:
    """Return the changed crates plus everything that transitively depends on them."""
    affected: set[str] = set()
    pending = list(changed)
    while pending:
        crate = pending.pop()
        if crate in affected:
            continue
        affected.add(crate)
        pending.extend(workspace.dependents.get(crate, frozenset()))
    return frozenset(affected)
