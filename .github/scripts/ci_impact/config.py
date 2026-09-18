"""Load and validate .github/ci-impact.toml."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import re
import tomllib
from typing import Any, Mapping, Sequence


@dataclass(frozen=True)
class JobRule:
    """A CI job and the crates or paths that turn it on."""

    name: str
    crates: frozenset[str]
    paths: tuple[str, ...]


@dataclass(frozen=True)
class ImpactConfig:
    global_paths: tuple[str, ...]
    safe_exact: frozenset[str]
    safe_prefixes: tuple[str, ...]
    extension_crates: frozenset[str]
    rust_paths: tuple[str, ...]
    jobs: tuple[JobRule, ...]
    extension_paths: tuple[str, ...] = ()


def _strings(value: Any, label: str) -> tuple[str, ...]:
    if value is None:
        return ()
    if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
        raise ValueError(f"{label} must be a list of strings")
    return tuple(value)


def _table(data: Mapping[str, Any], key: str, allowed: Sequence[str]) -> Mapping[str, Any]:
    table = data.get(key, {})
    if not isinstance(table, dict):
        raise ValueError(f"[{key}] must be a table")
    unknown = sorted(set(table) - set(allowed))
    if unknown:
        raise ValueError(f"[{key}] has unknown keys: {', '.join(unknown)}")
    return table


def parse_config(data: Mapping[str, Any]) -> ImpactConfig:
    """Build an ImpactConfig from decoded TOML, rejecting unknown keys."""
    unknown = sorted(set(data) - {"global", "safe", "extensions", "rust", "jobs"})
    if unknown:
        raise ValueError(f"unknown top-level keys: {', '.join(unknown)}")
    global_table = _table(data, "global", ["paths"])
    safe_table = _table(data, "safe", ["exact", "prefixes"])
    extensions_table = _table(data, "extensions", ["crates", "paths"])
    rust_table = _table(data, "rust", ["paths"])
    jobs_table = data.get("jobs", {})
    if not isinstance(jobs_table, dict):
        raise ValueError("[jobs] must be a table")
    jobs = []
    for name, rule in jobs_table.items():
        if not isinstance(rule, dict):
            raise ValueError(f"[jobs.{name}] must be a table")
        unknown_rule = sorted(set(rule) - {"crates", "paths"})
        if unknown_rule:
            raise ValueError(f"[jobs.{name}] has unknown keys: {', '.join(unknown_rule)}")
        jobs.append(
            JobRule(
                name=name,
                crates=frozenset(_strings(rule.get("crates"), f"jobs.{name}.crates")),
                paths=_strings(rule.get("paths"), f"jobs.{name}.paths"),
            )
        )
    return ImpactConfig(
        global_paths=_strings(global_table.get("paths"), "global.paths"),
        safe_exact=frozenset(_strings(safe_table.get("exact"), "safe.exact")),
        safe_prefixes=_strings(safe_table.get("prefixes"), "safe.prefixes"),
        extension_crates=frozenset(_strings(extensions_table.get("crates"), "extensions.crates")),
        extension_paths=_strings(extensions_table.get("paths"), "extensions.paths"),
        rust_paths=_strings(rust_table.get("paths"), "rust.paths"),
        jobs=tuple(jobs),
    )


def load_config(path: Path) -> ImpactConfig:
    with path.open("rb") as source:
        return parse_config(tomllib.load(source))


def _glob_to_regex(pattern: str) -> re.Pattern[str]:
    parts: list[str] = []
    index = 0
    while index < len(pattern):
        char = pattern[index]
        if pattern.startswith("**/", index):
            parts.append("(?:.*/)?")
            index += 3
            continue
        if pattern.startswith("**", index):
            parts.append(".*")
            index += 2
            continue
        if char == "*":
            parts.append("[^/]*")
        elif char == "?":
            parts.append("[^/]")
        else:
            parts.append(re.escape(char))
        index += 1
    return re.compile("^" + "".join(parts) + "$")


def matches_pattern(pattern: str, path: str) -> bool:
    """Match a repository-relative path against a glob with ** support."""
    return _glob_to_regex(pattern).match(path) is not None
