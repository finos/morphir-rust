"""Shared release-asset types and release-routing integration."""

from __future__ import annotations

from dataclasses import dataclass
import importlib.util
from pathlib import Path
import re
import sys
from types import ModuleType

class AssetError(Exception):
    """Report an invalid or conflicting release asset without a traceback."""


SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")


@dataclass(frozen=True)
class ReleaseAsset:
    """An unchanged bundle file and its deterministic published asset name."""

    source: Path
    name: str


@dataclass(frozen=True)
class AssetSelection:
    """Validated new assets and byte-identical assets already published."""

    uploads: list[ReleaseAsset]
    skipped: list[str]
    expected: list[ReleaseAsset]


def load_release_module() -> ModuleType:
    """Load the sibling tag-routing helper without changing sys.path."""
    path = Path(__file__).resolve().parent.parent / "extension_release.py"
    spec = importlib.util.spec_from_file_location("extension_release", path)
    if spec is None or spec.loader is None:
        raise AssetError(f"cannot load release routing helper {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules.setdefault("extension_release", module)
    try:
        spec.loader.exec_module(module)
    except OSError as error:
        raise AssetError(f"cannot load release routing helper {path}: {error}") from error
    return module


extension_release = load_release_module()
