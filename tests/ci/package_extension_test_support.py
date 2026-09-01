"""Shared fixtures for extension packaging and artifact task tests."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import textwrap
import tomllib
import unittest


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPOSITORY_ROOT / "scripts"))

from extension_packaging.errors import PackageError  # noqa: E402
from extension_packaging.model import descriptor_bytes  # noqa: E402
from extension_packaging.paths import (  # noqa: E402
    clean_extension_staging,
    clean_head_snapshot,
    validate_extension_staging,
)

EXTENSIONS_TOML = REPOSITORY_ROOT / ".github" / "extensions.toml"
PACKAGER = REPOSITORY_ROOT / "scripts" / "package_extension.py"
PACKAGER_MODULES = REPOSITORY_ROOT / "scripts" / "extension_packaging"
AVRO_ARTIFACT_TASK = (
    REPOSITORY_ROOT / ".mise" / "tasks" / "extension" / "artifact" / "avro"
)

AVRO_REGISTRY = """\
[extensions.avro]
package = "morphir-avro-extension"
artifact = "morphir-avro-extension"
extension_id = "morphir-avro"
mep_versions = ["0.1"]
targets = ["avro"]
ir_versions = ["3", "4"]
release_with_workspace = true
"""


def sha256(path: Path) -> str:
    """Return the lowercase SHA-256 hex digest for a file."""
    return hashlib.sha256(path.read_bytes()).hexdigest()


class PackageFixture:
    """Build a small repository that can invoke the real packager script."""

    def __init__(
        self,
        root: Path,
        *,
        registry: str = AVRO_REGISTRY,
        cargo_package: str = "morphir-avro-extension",
        cargo_version: str = "0.1.0",
    ) -> None:
        self.root = root.resolve()
        self.wasm = self.root / "input" / "guest.wasm"
        self.output = self.root / "stage"
        (self.root / "scripts").mkdir(parents=True)
        (self.root / ".github").mkdir()
        (self.root / "crates" / "morphir-avro-extension").mkdir(parents=True)
        self.wasm.parent.mkdir()

        shutil.copy2(PACKAGER, self.root / "scripts" / "package_extension.py")
        shutil.copytree(
            PACKAGER_MODULES,
            self.root / "scripts" / "extension_packaging",
            ignore=shutil.ignore_patterns("__pycache__", "*.pyc"),
        )
        (self.root / ".github" / "extensions.toml").write_text(
            registry, encoding="utf-8", newline="\n"
        )
        (self.root / "crates" / "morphir-avro-extension" / "Cargo.toml").write_text(
            f'[package]\nname = "{cargo_package}"\nversion = "{cargo_version}"\n',
            encoding="utf-8",
            newline="\n",
        )
        self.wasm.write_bytes(b"\x00asm\x01\x00\x00\x00fixture")

    def package(
        self,
        short_id: str = "avro",
        *,
        wasm: Path | None = None,
        git_commit: str | None = None,
    ) -> subprocess.CompletedProcess[str]:
        command = [
            sys.executable,
            str(self.root / "scripts" / "package_extension.py"),
            "--short-id",
            short_id,
            "--wasm",
            str(wasm or self.wasm),
            "--output",
            str(self.output),
        ]
        if git_commit is not None:
            command.extend(["--git-commit", git_commit])
        return subprocess.run(
            command,
            cwd=self.root,
            check=False,
            capture_output=True,
            text=True,
        )

    def transfer(
        self, source: Path, *, output: Path | None = None
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(self.root / "scripts" / "package_extension.py"),
                "--transfer-bundle",
                str(source),
                "--output",
                str(output or self.output),
            ],
            cwd=self.root,
            check=False,
            capture_output=True,
            text=True,
        )


class ArtifactTaskFixture:
    """Run the artifact task in a local Git sandbox with stubbed build tools."""

    def __init__(self, root: Path) -> None:
        self.root = root.resolve()
        self.log = self.root.parent / "artifact-task.log"
        self.snapshot_marker = self.root.parent / "archived-task-ran"
        PackageFixture(self.root)
        task = self.root / ".mise" / "tasks" / "extension" / "artifact" / "avro"
        task.parent.mkdir(parents=True)
        shutil.copy2(AVRO_ARTIFACT_TASK, task)
        task.write_text(
            task.read_text(encoding="utf-8").replace(
                "set -eu\n",
                """\
set -eu

if [ "${MORPHIR_AVRO_SNAPSHOT_MODE:-}" = "1" ] && [ -n "${ARTIFACT_SNAPSHOT_MARKER:-}" ]; then
    printf '%s\n' "$0" > "$ARTIFACT_SNAPSHOT_MARKER"
fi
""",
                1,
            ),
            encoding="utf-8",
            newline="\n",
        )
        self.task = task

        (self.root / "Cargo.toml").write_text(
            '[workspace]\nmembers = ["crates/morphir-avro-extension"]\n',
            encoding="utf-8",
            newline="\n",
        )
        (self.root / "Cargo.lock").write_text(
            "# fixture lockfile\n", encoding="utf-8", newline="\n"
        )
        (self.root / "mise.toml").write_text(
            "[tools]\n", encoding="utf-8", newline="\n"
        )
        avro_idl_task = self.root / ".mise/tasks/test/avro-idl"
        avro_idl_task.parent.mkdir(parents=True, exist_ok=True)
        avro_idl_task.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8", newline="\n")
        avro_idl_task.chmod(0o755)
        avro_crate = self.root / "crates/morphir-avro-extension"
        (avro_crate / "src").mkdir()
        (avro_crate / "src/lib.rs").write_text(
            "// fixture source\n", encoding="utf-8", newline="\n"
        )
        (avro_crate / "tests").mkdir()
        (avro_crate / "tests/guest.rs").write_text(
            "// fixture test\n", encoding="utf-8", newline="\n"
        )
        packaging_tests = (
            "tests/ci/test_package_extension.py",
            "tests/ci/package_extension_test_support.py",
            "tests/ci/test_package_extension_packaging.py",
            "tests/ci/test_package_extension_task.py",
        )
        for relative in packaging_tests:
            packaging_test = self.root / relative
            packaging_test.parent.mkdir(parents=True, exist_ok=True)
            packaging_test.write_text(
                "# fixture packaging test\n", encoding="utf-8", newline="\n"
            )
        installed_guest_test = (
            self.root
            / "crates/morphir-daemon/tests/installed_wasm_extension.rs"
        )
        installed_guest_test.parent.mkdir(parents=True)
        installed_guest_test.write_text(
            "// fixture installed guest test\n", encoding="utf-8", newline="\n"
        )

        (self.root / ".gitignore").write_text(
            ".morphir/\ntarget/\ncrates/morphir-avro-extension/ignored-source.rs\n",
            encoding="utf-8",
            newline="\n",
        )
        (self.root / "tracked.txt").write_text("clean\n", encoding="utf-8")
        tools = self.root / "test-bin"
        tools.mkdir()
        self._write_tool(
            "cargo",
            """
            printf '%s | %s\n' "$(pwd -P)" "cargo $*" >> "$ARTIFACT_TASK_LOG"
            if [ -f crates/morphir-avro-extension/ignored-source.rs ]; then
                printf '%s\n' "ignored-source-present" >> "$ARTIFACT_TASK_LOG"
            fi
            case "$*" in
                'build --locked --release -p morphir-avro-extension --target wasm32-unknown-unknown')
                    mkdir -p target/wasm32-unknown-unknown/release
                    printf '\\000asm\\001\\000\\000\\000' > target/wasm32-unknown-unknown/release/morphir_avro_extension.wasm
                    ;;
            esac
            """,
        )
        self._write_tool(
            "mise",
            'printf \'%s | %s\\n\' "$(pwd -P)" "mise $*" >> "$ARTIFACT_TASK_LOG"\n',
        )
        self._write_tool(
            "wasm-tools",
            'test -f "$2"\nprintf \'%s | %s\\n\' "$(pwd -P)" "wasm-tools $*" >> "$ARTIFACT_TASK_LOG"\n',
        )
        self._write_tool("java", 'echo \'openjdk version "17.0.12"\' >&2\n')

        self._git("init", "-q")
        self._git("config", "user.name", "Task Fixture")
        self._git("config", "user.email", "task-fixture@example.invalid")
        self._git("add", ".")
        self._git("-c", "commit.gpgsign=false", "commit", "-qm", "fixture")

    def _write_tool(self, name: str, body: str) -> None:
        path = self.root / "test-bin" / name
        path.write_text(
            "#!/bin/sh\nset -eu\n" + textwrap.dedent(body).lstrip(),
            encoding="utf-8",
            newline="\n",
        )
        path.chmod(0o755)

    def _git(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", *args],
            cwd=self.root,
            check=True,
            capture_output=True,
            text=True,
        )

    def run(
        self,
        extra_environment: dict[str, str] | None = None,
        working_directory: Path | None = None,
    ) -> subprocess.CompletedProcess[str]:
        environment = os.environ.copy()
        environment.pop("PYTHONPYCACHEPREFIX", None)
        environment["PYTHONDONTWRITEBYTECODE"] = "1"
        environment["PATH"] = f"{self.root / 'test-bin'}{os.pathsep}{environment['PATH']}"
        environment["ARTIFACT_TASK_LOG"] = str(self.log)
        environment["ARTIFACT_SNAPSHOT_MARKER"] = str(self.snapshot_marker)
        environment.update(extra_environment or {})
        return subprocess.run(
            [str(self.task)],
            cwd=working_directory or self.root,
            env=environment,
            check=False,
            capture_output=True,
            text=True,
        )

    def descriptor(self) -> dict[str, object]:
        return json.loads(
            (
                self.root / ".morphir/build/extensions/avro/release.json"
            ).read_text(encoding="utf-8")
        )

    def head(self) -> str:
        return self._git("rev-parse", "HEAD").stdout.strip()

    def log_lines(self) -> list[str]:
        return self.log.read_text(encoding="utf-8").splitlines()

    def archived_task_path(self) -> Path:
        return Path(self.snapshot_marker.read_text(encoding="utf-8").strip())

    def snapshot_source(self) -> Path:
        return self.archived_task_path().parents[4]
