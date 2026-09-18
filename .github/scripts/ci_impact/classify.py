"""Turn a list of changed paths into a CI plan."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import tomllib
from typing import Mapping, Sequence

from .config import ImpactConfig, matches_pattern
from .graph import Workspace, affected_closure


@dataclass(frozen=True)
class Extension:
    short_id: str
    package: str


@dataclass(frozen=True)
class Plan:
    all: bool
    crates: tuple[str, ...]
    cargo_packages: str
    rust: bool
    jobs: Mapping[str, bool]
    extensions: tuple[Extension, ...]
    reasons: tuple[str, ...]


def load_extensions(path: Path) -> tuple[Extension, ...]:
    with path.open("rb") as source:
        registry = tomllib.load(source)
    entries = registry.get("extensions", {})
    return tuple(
        Extension(short_id=short_id, package=entry["package"])
        for short_id, entry in sorted(entries.items())
    )


def crate_for_path(path: str) -> str | None:
    """Return the crate name for paths under crates/<name>/, else None."""
    parts = path.split("/")
    if len(parts) >= 3 and parts[0] == "crates" and parts[1]:
        return parts[1]
    return None


def _is_safe(path: str, config: ImpactConfig) -> bool:
    return path in config.safe_exact or path.startswith(config.safe_prefixes)


def _cargo_packages(crates: Sequence[str], workspace: Workspace) -> str:
    return " ".join(f"-p {crate}" for crate in crates if crate in workspace.default_members)


def full_plan(
    config: ImpactConfig,
    workspace: Workspace,
    extensions: Sequence[Extension],
    reason: str,
) -> Plan:
    return Plan(
        all=True,
        crates=tuple(sorted(workspace.members)),
        cargo_packages="",
        rust=True,
        jobs={job.name: True for job in config.jobs},
        extensions=tuple(extensions),
        reasons=(reason,),
    )


def plan_changes(
    paths: Sequence[str],
    config: ImpactConfig,
    workspace: Workspace,
    extensions: Sequence[Extension],
) -> Plan:
    if not paths:
        return full_plan(config, workspace, extensions, "no changed paths were reported")

    changed_crates: set[str] = set()
    for path in paths:
        if any(matches_pattern(pattern, path) for pattern in config.global_paths):
            return full_plan(config, workspace, extensions, f"global path changed: {path}")
        crate = crate_for_path(path)
        if crate is not None:
            if crate not in workspace.members:
                return full_plan(config, workspace, extensions, f"path is not in a workspace crate: {path}")
            changed_crates.add(crate)
            continue
        if _is_safe(path, config):
            continue
        claimed = any(
            matches_pattern(pattern, path) for job in config.jobs for pattern in job.paths
        )
        if not claimed:
            return full_plan(config, workspace, extensions, f"unclaimed path changed: {path}")

    affected = affected_closure(workspace, changed_crates)
    crates = tuple(sorted(affected))
    jobs = {
        job.name: bool(job.crates & affected)
        or any(matches_pattern(pattern, path) for pattern in job.paths for path in paths)
        for job in config.jobs
    }
    rebuild_all_extensions = bool(config.extension_crates & affected)
    matrix = tuple(
        extension
        for extension in extensions
        if rebuild_all_extensions or extension.package in affected
    )
    reasons = tuple(f"crate changed: {crate}" for crate in sorted(changed_crates))
    return Plan(
        all=False,
        crates=crates,
        cargo_packages=_cargo_packages(crates, workspace),
        rust=bool(crates),
        jobs=jobs,
        extensions=matrix,
        reasons=reasons,
    )
