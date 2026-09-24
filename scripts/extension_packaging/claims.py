"""Read guest capability claims and check declared release metadata."""

from __future__ import annotations

import json
from pathlib import Path
import subprocess
import tempfile
from typing import Any

from .errors import PackageError
from .model import (
    frontend_incremental, frontend_languages, require_string,
    require_string_list, validate_semver, workspace_discovery,
)


CLAIMS_TIMEOUT_SECONDS = 1800
CLAIMS_VERSION = "0.1.0-draft.2"


def read_claims(root: Path, wasm_bytes: bytes) -> dict[str, Any]:
    """Describe a private snapshot of the exact bytes destined for the bundle."""
    try:
        with tempfile.TemporaryDirectory(prefix="morphir-extension-claims-") as directory:
            wasm = Path(directory) / "guest.wasm"
            wasm.write_bytes(wasm_bytes)
            result = subprocess.run(
                ["cargo", "run", "--quiet", "--locked", "-p", "morphir-host-native",
                 "--bin", "extension-claims", "--", str(wasm)],
                cwd=root, check=True, stdout=subprocess.PIPE, text=True, encoding="utf-8",
                # A guest whose describe never answers must not stall a release job. The first
                # run may also compile the tool, so the bound is generous.
                timeout=CLAIMS_TIMEOUT_SECONDS,
            )
        return json.loads(result.stdout)
    except (
        OSError, UnicodeError, subprocess.CalledProcessError, subprocess.TimeoutExpired,
        json.JSONDecodeError,
    ) as error:
        raise PackageError(f"cannot read WASM capability claims: {error}") from error


def check_claims(short_id: str, extension: dict[str, Any], version: str, claims: object) -> None:
    """Refuse drift in registry-owned fields without rewriting guest metadata."""
    languages = frontend_languages(extension) if "languages" in extension else None
    incremental = frontend_incremental(short_id, extension, languages is not None)
    workspace = workspace_discovery(short_id, extension, languages is not None)
    targets = extension.get("targets", [])
    if targets != [] or languages is None:
        targets = require_string_list(extension, "targets")
    ir_versions = require_string_list(extension, "ir_versions")
    kinds = (["frontend"] if languages else []) + (["backend"] if targets else []) + (["workspace"] if workspace else [])

    def agrees(path: str, expected: object, *, unordered: bool = False) -> None:
        actual = claims
        for key in path.split("."):
            actual = actual.get(key) if isinstance(actual, dict) else None
        if unordered and isinstance(actual, list) and isinstance(expected, list):
            # Compare sets while refusing duplicate entries and malformed values.
            actual = sorted(json.dumps(item, sort_keys=True) for item in actual)
            expected = sorted(json.dumps(item, sort_keys=True) for item in expected)
        if type(actual) is not type(expected) or actual != expected:
            raise PackageError(f"extension {short_id} claims {path} does not match registry or Cargo version")

    # The reader compares the draft exactly but ignores SemVer build metadata, as Cargo does.
    claims_version = claims.get("claimsVersion") if isinstance(claims, dict) else None
    if not isinstance(claims_version, str) or claims_version.split("+", 1)[0] != CLAIMS_VERSION:
        raise PackageError(f"extension {short_id} claims claimsVersion must be {CLAIMS_VERSION}")
    agrees("extension.id", require_string(extension, "extension_id"))
    agrees("extension.version", version)
    if "name" in extension:
        agrees("extension.name", require_string(extension, "name"))
    agrees("protocolVersions", require_string_list(extension, "mep_versions"), unordered=True)
    agrees("extension.types", kinds, unordered=True)
    capabilities = claims.get("capabilities") if isinstance(claims, dict) else None
    if not isinstance(capabilities, dict):
        raise PackageError(f"extension {short_id} claims capabilities must be an object")
    for kind in ("frontend", "backend", "workspace"):
        if (kind in capabilities) != (kind in kinds):
            raise PackageError(f"extension {short_id} claims capability kinds disagree: {kind}")
        if kind in kinds and not isinstance(capabilities[kind], dict):
            raise PackageError(f"extension {short_id} claims capabilities.{kind} must be an object")
    validate_capabilities(capabilities)
    if languages:
        # Only the declared language identity and suffixes are registry-owned.
        # Unknown optional language members stay in the shipped claims.
        actual_languages = capabilities["frontend"]["languages"]
        def language_key(language):
            return language["id"], sorted(language["fileExtensions"])
        if sorted(map(language_key, actual_languages)) != sorted(map(language_key, languages)):
            raise PackageError(f"extension {short_id} claims capabilities.frontend.languages does not match registry")
        agrees("capabilities.frontend.irVersions", ir_versions, unordered=True)
        agrees("capabilities.frontend.compile", True)
        actual_incremental = capabilities["frontend"]["incremental"]
        if type(actual_incremental) is not bool or actual_incremental != incremental:
            raise PackageError(f"extension {short_id} claims capabilities.frontend.incremental does not match registry")
    if targets:
        agrees("capabilities.backend.targets", targets, unordered=True)
        agrees("capabilities.backend.irVersions", ir_versions, unordered=True)
        agrees("capabilities.backend.generate", True)
    if workspace:
        agrees("capabilities.workspace.discover", True)


def validate_capabilities(capabilities: dict[str, Any]) -> None:
    """Check known capability objects as the Rust publication reader does.

    Unknown optional members remain intact. Required fields mirror the SDK's
    FrontendCapability, BackendCapability and WorkspaceCapability wire types.
    """
    def strings(value: object) -> bool:
        return isinstance(value, list) and all(isinstance(item, str) for item in value)

    def require(value: object, valid: bool, path: str) -> None:
        if not valid:
            raise PackageError(f"invalid claims capabilities.{path}: {value!r}")

    fields = {
        "frontend": {"languages": list, "irVersions": strings, "compile": bool,
                     "incremental": bool, "fragments": bool},
        "backend": {"targets": strings, "irVersions": strings, "generate": bool},
        "workspace": {"protocolVersions": strings, "discover": bool},
    }
    for kind, members in fields.items():
        if kind not in capabilities:
            continue
        capability = capabilities[kind]
        require(capability, isinstance(capability, dict), kind)
        for member, expected in members.items():
            value = capability.get(member)
            valid = type(value) is expected if isinstance(expected, type) else expected(value)
            require(value, valid, f"{kind}.{member}")
    if "frontend" in capabilities:
        frontend = capabilities["frontend"]
        if "multiDocument" in frontend:
            require(frontend["multiDocument"], type(frontend["multiDocument"]) is bool,
                    "frontend.multiDocument")
        for language in frontend["languages"]:
            require(language, isinstance(language, dict), "frontend.languages")
            require(language.get("id"), isinstance(language.get("id"), str), "frontend.languages.id")
            require(language.get("fileExtensions"), strings(language.get("fileExtensions")),
                    "frontend.languages.fileExtensions")
    if "workspace" in capabilities:
        for version in capabilities["workspace"]["protocolVersions"]:
            try:
                validate_semver(version)
            except PackageError as error:
                raise PackageError("invalid claims capabilities.workspace.protocolVersions: expected SemVer") from error
