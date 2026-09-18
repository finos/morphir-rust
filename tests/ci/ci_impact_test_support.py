"""Shared imports and helpers for ci_impact tests."""

from __future__ import annotations

import os
from pathlib import Path
import sys

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SCRIPTS_DIRECTORY = REPOSITORY_ROOT / ".github" / "scripts"
if os.fspath(SCRIPTS_DIRECTORY) not in sys.path:
    sys.path.insert(0, os.fspath(SCRIPTS_DIRECTORY))

from ci_impact import config, graph  # noqa: E402


def fake_metadata() -> dict:
    """A small workspace shaped like morphir-rust for graph tests.

    core <- sdk <- {daemon, gleam, python}
    core <- projection <- avro
    daemon dev-depends on avro (installed wasm tests)
    ext-example is a member but not a default member
    """

    def package(name: str, deps: list[tuple[str, str]]) -> dict:
        return {
            "name": name,
            "id": f"path+file:///ws/crates/{name}#{name}@0.1.0",
            "manifest_path": f"/ws/crates/{name}/Cargo.toml",
            "dependencies": [
                {"name": dep, "kind": kind if kind != "normal" else None}
                for dep, kind in deps
            ],
        }

    packages = [
        package("morphir-core", []),
        package("morphir-extension-sdk", [("morphir-core", "normal"), ("serde", "normal")]),
        package("morphir-projection", [("morphir-core", "normal")]),
        package("morphir-daemon", [("morphir-extension-sdk", "normal"), ("morphir-avro-extension", "dev")]),
        package("morphir-gleam-binding", [("morphir-extension-sdk", "normal")]),
        package("morphir-python-binding", [("morphir-extension-sdk", "normal")]),
        package("morphir-avro-extension", [("morphir-projection", "normal"), ("morphir-extension-sdk", "build")]),
        package("morphir-ext-example", [("morphir-extension-sdk", "normal")]),
    ]
    ids = [item["id"] for item in packages]
    return {
        "packages": packages,
        "workspace_members": ids,
        "workspace_default_members": [i for i in ids if "ext-example" not in i],
    }


def fake_config_data() -> dict:
    return {
        "global": {"paths": ["Cargo.toml", "Cargo.lock", "mise.toml", ".github/workflows/ci.yml"]},
        "safe": {
            "exact": ["README.md", "LICENSE"],
            "prefixes": [".beads/", "docs/"],
        },
        "extensions": {"crates": ["morphir-daemon"]},
        "jobs": {
            "kit-conformance": {"crates": ["morphir-projection"], "paths": [".config/mck-driver-version"]},
            "test-extism": {"crates": ["morphir-daemon", "morphir-avro-extension"], "paths": [".mise/tasks/test/extism"]},
            "lint-shell": {"paths": ["**/*.sh", ".mise/tasks/**"]},
            "docs-generated": {"paths": ["docs/**", "CHANGELOG.md"]},
        },
    }


def fake_extensions() -> tuple[classify.Extension, ...]:
    return (
        classify.Extension(short_id="avro", package="morphir-avro-extension"),
        classify.Extension(short_id="python", package="morphir-python-binding"),
    )
