"""Tests for deterministic extension release bundle construction."""

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
EXTENSIONS_TOML = REPOSITORY_ROOT / ".github" / "extensions.toml"
PACKAGER = REPOSITORY_ROOT / "scripts" / "package_extension.py"
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
        packaging_test = self.root / "tests/ci/test_package_extension.py"
        packaging_test.parent.mkdir(parents=True)
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


class PackageExtensionTests(unittest.TestCase):
    def test_avro_registry_entry_is_independently_versioned(self) -> None:
        registry = tomllib.loads(EXTENSIONS_TOML.read_text(encoding="utf-8"))
        avro = registry["extensions"]["avro"]

        self.assertEqual("morphir-avro-extension", avro["package"])
        self.assertEqual("morphir-avro-extension", avro["artifact"])
        self.assertEqual("morphir-avro", avro["extension_id"])
        self.assertEqual(["0.1"], avro["mep_versions"])
        self.assertEqual(["avro"], avro["targets"])
        self.assertEqual(["3", "4"], avro["ir_versions"])
        self.assertTrue(avro["release_with_workspace"])

    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.fixture = PackageFixture(
            Path(self.temporary_directory.name) / "repository"
        )

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def test_release_descriptor_uses_crate_version_and_wasm_digest(self) -> None:
        result = self.fixture.package()
        self.assertEqual(0, result.returncode, result.stderr)

        descriptor = json.loads(
            (self.fixture.output / "release.json").read_text(encoding="utf-8")
        )
        artifact = self.fixture.output / descriptor["artifact"]
        self.assertEqual(1, descriptor["schemaVersion"])
        self.assertEqual("avro", descriptor["shortId"])
        self.assertEqual("morphir-avro", descriptor["extensionId"])
        self.assertEqual("morphir-avro-extension", descriptor["package"])
        self.assertEqual("0.1.0", descriptor["version"])
        self.assertEqual(["0.1"], descriptor["mepVersions"])
        self.assertEqual("wasm", descriptor["runtime"])
        self.assertEqual(["avro"], descriptor["targets"])
        self.assertEqual(["3", "4"], descriptor["irVersions"])
        self.assertEqual(sha256(artifact), descriptor["sha256"])
        self.assertEqual(self.fixture.wasm.read_bytes(), artifact.read_bytes())

    def test_rejects_unknown_short_id(self) -> None:
        result = self.fixture.package("unknown")

        self.assertNotEqual(0, result.returncode)
        self.assertIn("unknown extension short ID: unknown", result.stderr)
        self.assertFalse(self.fixture.output.exists())

    def test_rejects_missing_wasm(self) -> None:
        missing = self.fixture.root / "input" / "missing.wasm"
        result = self.fixture.package(wasm=missing)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(f"WASM file does not exist: {missing}", result.stderr)
        self.assertFalse(self.fixture.output.exists())

    def test_rejects_registry_and_cargo_package_mismatch(self) -> None:
        self.temporary_directory.cleanup()
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.fixture = PackageFixture(
            Path(self.temporary_directory.name) / "repository",
            cargo_package="another-package",
        )

        result = self.fixture.package()

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "registry package morphir-avro-extension does not match Cargo package another-package",
            result.stderr,
        )
        self.assertFalse(self.fixture.output.exists())

    def test_refuses_to_overwrite_a_different_staged_artifact(self) -> None:
        first = self.fixture.package()
        self.assertEqual(0, first.returncode, first.stderr)
        artifact = self.fixture.output / "morphir-avro-extension-0.1.0.wasm"
        artifact.write_bytes(b"different staged bytes")

        second = self.fixture.package()

        self.assertNotEqual(0, second.returncode)
        self.assertIn("refusing to overwrite different staged bundle", second.stderr)
        self.assertEqual(b"different staged bytes", artifact.read_bytes())

    def test_repeating_an_identical_package_is_idempotent(self) -> None:
        first = self.fixture.package(git_commit="abc123")
        self.assertEqual(0, first.returncode, first.stderr)
        before = {
            path.name: path.read_bytes() for path in self.fixture.output.iterdir()
        }

        second = self.fixture.package(git_commit="abc123")

        self.assertEqual(0, second.returncode, second.stderr)
        after = {path.name: path.read_bytes() for path in self.fixture.output.iterdir()}
        self.assertEqual(before, after)

    def test_checksum_is_basename_only_with_final_newline(self) -> None:
        result = self.fixture.package()
        self.assertEqual(0, result.returncode, result.stderr)
        artifact_name = "morphir-avro-extension-0.1.0.wasm"
        digest = sha256(self.fixture.output / artifact_name)

        checksum = (self.fixture.output / f"{artifact_name}.sha256").read_bytes()

        self.assertEqual(f"{digest}  {artifact_name}\n".encode(), checksum)
        self.assertNotIn(str(self.fixture.output).encode(), checksum)

    def test_descriptor_has_one_final_newline_and_deterministic_field_order(self) -> None:
        result = self.fixture.package()
        self.assertEqual(0, result.returncode, result.stderr)

        raw_descriptor = (self.fixture.output / "release.json").read_bytes()
        descriptor = json.loads(raw_descriptor)

        self.assertTrue(raw_descriptor.endswith(b"\n"))
        self.assertFalse(raw_descriptor.endswith(b"\n\n"))
        self.assertEqual(
            [
                "schemaVersion",
                "shortId",
                "extensionId",
                "package",
                "version",
                "mepVersions",
                "runtime",
                "targets",
                "irVersions",
                "artifact",
                "sha256",
            ],
            list(descriptor),
        )

    def test_git_commit_is_optional_and_last_when_supplied(self) -> None:
        without_commit = self.fixture.package()
        self.assertEqual(0, without_commit.returncode, without_commit.stderr)
        descriptor = json.loads(
            (self.fixture.output / "release.json").read_text(encoding="utf-8")
        )
        self.assertNotIn("gitCommit", descriptor)

        shutil.rmtree(self.fixture.output)
        with_commit = self.fixture.package(git_commit="0123456789abcdef")
        self.assertEqual(0, with_commit.returncode, with_commit.stderr)
        descriptor = json.loads(
            (self.fixture.output / "release.json").read_text(encoding="utf-8")
        )
        self.assertEqual("0123456789abcdef", descriptor["gitCommit"])
        self.assertEqual("gitCommit", list(descriptor)[-1])

    def test_bundle_contains_only_wasm_checksum_and_descriptor(self) -> None:
        result = self.fixture.package()
        self.assertEqual(0, result.returncode, result.stderr)

        self.assertEqual(
            [
                "morphir-avro-extension-0.1.0.wasm",
                "morphir-avro-extension-0.1.0.wasm.sha256",
                "release.json",
            ],
            sorted(path.name for path in self.fixture.output.iterdir()),
        )

    def test_transfer_verifies_and_atomically_copies_the_exact_bundle(self) -> None:
        source = self.fixture.root / "snapshot-bundle"
        self.fixture.output = source
        packaged = self.fixture.package(git_commit="0123456789abcdef")
        self.assertEqual(0, packaged.returncode, packaged.stderr)
        expected = {path.name: path.read_bytes() for path in source.iterdir()}
        final = self.fixture.root / "final-bundle"

        transferred = self.fixture.transfer(source, output=final)

        self.assertEqual(0, transferred.returncode, transferred.stderr)
        self.assertEqual(
            expected, {path.name: path.read_bytes() for path in final.iterdir()}
        )

    def test_transfer_rejects_a_bundle_with_a_checksum_mismatch(self) -> None:
        source = self.fixture.root / "snapshot-bundle"
        self.fixture.output = source
        packaged = self.fixture.package(git_commit="0123456789abcdef")
        self.assertEqual(0, packaged.returncode, packaged.stderr)
        artifact = source / "morphir-avro-extension-0.1.0.wasm"
        artifact.write_bytes(b"tampered")
        final = self.fixture.root / "final-bundle"

        transferred = self.fixture.transfer(source, output=final)

        self.assertNotEqual(0, transferred.returncode)
        self.assertIn("bundle checksum mismatch", transferred.stderr)
        self.assertNotIn("Traceback", transferred.stderr)
        self.assertFalse(final.exists())

    def test_rejects_non_portable_short_ids_before_registry_lookup(self) -> None:
        for short_id in ("../avro", r"..\avro", "Avro", "avro_thing", "avro--idl"):
            with self.subTest(short_id=short_id):
                result = self.fixture.package(short_id)
                self.assertNotEqual(0, result.returncode)
                self.assertIn("invalid extension short ID", result.stderr)
                self.assertNotIn("Traceback", result.stderr)

    def test_rejects_non_portable_registry_package_and_artifact_ids(self) -> None:
        for field, invalid in (("package", "../escape"), ("artifact", r"bad\name")):
            with self.subTest(field=field):
                self.temporary_directory.cleanup()
                self.temporary_directory = tempfile.TemporaryDirectory()
                registry = AVRO_REGISTRY.replace(
                    f'{field} = "morphir-avro-extension"',
                    f'{field} = "{invalid.replace(chr(92), chr(92) * 2)}"',
                )
                self.fixture = PackageFixture(
                    Path(self.temporary_directory.name) / "repository",
                    registry=registry,
                )

                result = self.fixture.package()

                self.assertNotEqual(0, result.returncode)
                self.assertIn(f"invalid extension {field}", result.stderr)
                self.assertNotIn("Traceback", result.stderr)

    def test_rejects_non_semver_cargo_version(self) -> None:
        self.temporary_directory.cleanup()
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.fixture = PackageFixture(
            Path(self.temporary_directory.name) / "repository", cargo_version="1.2"
        )

        result = self.fixture.package()

        self.assertNotEqual(0, result.returncode)
        self.assertIn("invalid Cargo SemVer: 1.2", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    def test_accepts_cargo_semver_with_prerelease_and_build_metadata(self) -> None:
        self.temporary_directory.cleanup()
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.fixture = PackageFixture(
            Path(self.temporary_directory.name) / "repository",
            cargo_version="1.2.3-rc.1+build.7",
        )

        result = self.fixture.package()

        self.assertEqual(0, result.returncode, result.stderr)
        descriptor = json.loads(
            (self.fixture.output / "release.json").read_text(encoding="utf-8")
        )
        self.assertEqual("1.2.3-rc.1+build.7", descriptor["version"])

    def test_rejects_a_crate_manifest_that_resolves_outside_crates(self) -> None:
        crate = self.fixture.root / "crates" / "morphir-avro-extension"
        external_crate = self.fixture.root.parent / "external-crate"
        shutil.rmtree(crate)
        external_crate.mkdir()
        (external_crate / "Cargo.toml").write_text(
            '[package]\nname = "morphir-avro-extension"\nversion = "0.1.0"\n',
            encoding="utf-8",
            newline="\n",
        )
        crate.symlink_to(external_crate, target_is_directory=True)

        result = self.fixture.package()

        self.assertNotEqual(0, result.returncode)
        self.assertIn("Cargo manifest escapes crates directory", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    def test_rejects_an_output_directory_symlink_even_when_bundle_matches(self) -> None:
        external = self.fixture.root.parent / "external-bundle"
        self.fixture.output = external
        first = self.fixture.package()
        self.assertEqual(0, first.returncode, first.stderr)
        artifact = external / "morphir-avro-extension-0.1.0.wasm"
        expected_bytes = artifact.read_bytes()
        symlink = self.fixture.root / "stage"
        symlink.symlink_to(external, target_is_directory=True)
        self.fixture.output = symlink

        result = self.fixture.package()

        self.assertNotEqual(0, result.returncode)
        self.assertIn("symbolic link", result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        self.assertEqual(expected_bytes, artifact.read_bytes())

    def test_rejects_a_symlink_in_an_output_ancestor(self) -> None:
        external = self.fixture.root.parent / "external-parent"
        self.fixture.output = external / "stage"
        first = self.fixture.package()
        self.assertEqual(0, first.returncode, first.stderr)
        (self.fixture.root / "linked").symlink_to(external, target_is_directory=True)
        self.fixture.output = self.fixture.root / "linked" / "stage"

        result = self.fixture.package()

        self.assertNotEqual(0, result.returncode)
        self.assertIn("symbolic link", result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        self.assertTrue((external / "stage/release.json").is_file())

    def test_output_parent_io_error_has_no_traceback(self) -> None:
        blocked_parent = self.fixture.root / "blocked"
        blocked_parent.write_text("not a directory", encoding="utf-8")
        self.fixture.output = blocked_parent / "stage"

        result = self.fixture.package()

        self.assertNotEqual(0, result.returncode)
        self.assertIn("output ancestor is not a directory", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    @unittest.skipIf(os.name == "nt", "POSIX permission behavior")
    def test_unreadable_wasm_io_error_has_no_traceback(self) -> None:
        self.fixture.wasm.chmod(0)
        try:
            result = self.fixture.package()
        finally:
            self.fixture.wasm.chmod(0o600)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("cannot read WASM file", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    @unittest.skipIf(os.name == "nt", "POSIX permission behavior")
    def test_unreadable_existing_bundle_io_error_has_no_traceback(self) -> None:
        first = self.fixture.package()
        self.assertEqual(0, first.returncode, first.stderr)
        artifact = self.fixture.output / "morphir-avro-extension-0.1.0.wasm"
        artifact.chmod(0)
        try:
            result = self.fixture.package()
        finally:
            artifact.chmod(0o600)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("cannot read staged bundle entry", result.stderr)
        self.assertNotIn("Traceback", result.stderr)


class AvroArtifactTaskTests(unittest.TestCase):
    def test_task_is_executable_and_runs_the_required_pipeline_in_order(self) -> None:
        mode = AVRO_ARTIFACT_TASK.stat().st_mode
        script = AVRO_ARTIFACT_TASK.read_text(encoding="utf-8")
        commands = [
            "cargo test --locked -p morphir-avro-extension",
            "cargo build --locked --release -p morphir-avro-extension --target wasm32-unknown-unknown",
            "cargo test --locked -p morphir-daemon --test installed_wasm_extension -- --ignored",
            "mise run test:avro-idl",
            "wasm-tools validate target/wasm32-unknown-unknown/release/morphir_avro_extension.wasm",
            "set -- scripts/package_extension.py",
        ]

        self.assertTrue(mode & stat.S_IXUSR)
        self.assertIn("set -eu", script)
        positions = [script.index(command) for command in commands]
        self.assertEqual(sorted(positions), positions)

    def test_task_clears_only_the_explicit_avro_staging_directory(self) -> None:
        script = AVRO_ARTIFACT_TASK.read_text(encoding="utf-8")

        self.assertIn(
            'STAGING_DIR="$REPO_ROOT/.morphir/build/extensions/avro"', script
        )
        self.assertNotIn("rm -rf", script)
        self.assertIn("--clean-avro-staging", script)
        self.assertNotIn(".morphir/build/extensions/*", script)

    def test_task_packages_without_tagging_releasing_or_publishing(self) -> None:
        script = AVRO_ARTIFACT_TASK.read_text(encoding="utf-8")

        self.assertIn('--output "$output"', script)
        self.assertIn('run_pipeline "$REPO_ROOT" "$STAGING_DIR" ""', script)
        self.assertIn("MORPHIR_AVRO_SNAPSHOT_MODE=1", script)
        self.assertIn('--transfer-bundle "$SNAPSHOT_BUNDLE"', script)
        self.assertNotIn("git tag", script)
        self.assertNotIn("gh release", script)
        self.assertNotIn("cargo publish", script)
        self.assertNotIn("mise run extension:artifact:avro", script)

    def test_symlinked_staging_ancestor_is_rejected_without_external_deletion(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory) / "repository"
            fixture = ArtifactTaskFixture(root)
            external = root.parent / "external"
            avro = external / "avro"
            avro.mkdir(parents=True)
            sentinel = avro / "survives.txt"
            sentinel.write_text("keep\n", encoding="utf-8")
            (root / ".morphir/build").mkdir(parents=True)
            (root / ".morphir/build/extensions").symlink_to(
                external, target_is_directory=True
            )

            result = fixture.run()

            self.assertNotEqual(0, result.returncode)
            self.assertIn("symbolic link", result.stderr)
            self.assertEqual("keep\n", sentinel.read_text(encoding="utf-8"))
            self.assertFalse(fixture.log.exists())

    @unittest.skipIf(os.name == "nt", "POSIX permission behavior")
    def test_staging_cleanup_io_error_has_no_traceback(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            staging = fixture.root / ".morphir/build/extensions/avro"
            staging.mkdir(parents=True)
            (staging / "blocked").write_text("bytes", encoding="utf-8")
            staging.chmod(0)
            try:
                result = fixture.run()
            finally:
                if staging.exists():
                    staging.chmod(0o700)

            self.assertNotEqual(0, result.returncode)
            self.assertIn("cannot clean Avro staging directory", result.stderr)
            self.assertNotIn("Traceback", result.stderr)
            self.assertFalse(fixture.log.exists())

    def test_clean_tree_records_the_head_commit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")

            result = fixture.run()

            self.assertEqual(0, result.returncode, result.stderr)
            self.assertEqual(fixture.head(), fixture.descriptor()["gitCommit"])
            snapshot_source = fixture.snapshot_source()
            self.assertFalse(snapshot_source.is_relative_to(fixture.root))
            self.assertTrue(fixture.archived_task_path().is_relative_to(snapshot_source))
            self.assertTrue(
                all(line.startswith(f"{snapshot_source} | ") for line in fixture.log_lines())
            )
            self.assertFalse(snapshot_source.parent.exists())

    def test_ignored_source_absent_from_head_cannot_affect_provenance_build(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            ignored = fixture.root / "crates/morphir-avro-extension/ignored-source.rs"
            ignored.write_text("ambient-only\n", encoding="utf-8")
            self.assertEqual(
                "",
                fixture._git(
                    "status", "--porcelain", "--untracked-files=normal"
                ).stdout,
            )

            result = fixture.run()

            self.assertEqual(0, result.returncode, result.stderr)
            self.assertEqual(fixture.head(), fixture.descriptor()["gitCommit"])
            self.assertNotIn("ignored-source-present", fixture.log_lines())
            self.assertFalse(fixture.snapshot_source().parent.exists())

    def test_task_resolves_its_repository_when_called_from_outside(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")

            result = fixture.run(working_directory=fixture.root.parent)

            self.assertEqual(0, result.returncode, result.stderr)
            self.assertEqual(fixture.head(), fixture.descriptor()["gitCommit"])

    def test_dirty_tree_omits_the_head_commit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            (fixture.root / "tracked.txt").write_text("dirty\n", encoding="utf-8")

            result = fixture.run()

            self.assertEqual(0, result.returncode, result.stderr)
            self.assertNotIn("gitCommit", fixture.descriptor())

    def test_snapshot_mode_cannot_claim_an_ambient_worktree(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            (fixture.root / "tracked.txt").write_text("dirty\n", encoding="utf-8")

            result = fixture.run(
                {
                    "MORPHIR_AVRO_SNAPSHOT_MODE": "1",
                    "MORPHIR_AVRO_HEAD_COMMIT": fixture.head(),
                }
            )

            self.assertNotEqual(0, result.returncode)
            self.assertIn("snapshot source must not be inside a Git worktree", result.stderr)
            self.assertFalse(
                (fixture.root / ".morphir/build/extensions/avro/release.json").exists()
            )

    def test_plain_untracked_input_builds_locally_without_git_commit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            untracked = fixture.root / "crates/morphir-avro-extension/local-input.rs"
            untracked.write_text("local-only\n", encoding="utf-8")

            result = fixture.run()

            self.assertEqual(0, result.returncode, result.stderr)
            self.assertNotIn("gitCommit", fixture.descriptor())
            self.assertTrue(
                all(line.startswith(f"{fixture.root} | ") for line in fixture.log_lines())
            )
            self.assertFalse(fixture.snapshot_marker.exists())

    def test_clean_status_fails_when_head_lacks_a_packaging_input(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            fixture._git("rm", "--cached", "scripts/package_extension.py")
            with (fixture.root / ".gitignore").open("a", encoding="utf-8") as ignore:
                ignore.write("scripts/package_extension.py\n")
            fixture._git("add", ".gitignore")
            fixture._git(
                "-c", "commit.gpgsign=false", "commit", "-qm", "ignore packager"
            )
            self.assertEqual(
                "",
                fixture._git(
                    "status", "--porcelain", "--untracked-files=normal"
                ).stdout,
            )

            result = fixture.run()

            self.assertNotEqual(0, result.returncode)
            self.assertFalse(
                (fixture.root / ".morphir/build/extensions/avro/release.json").exists()
            )
            self.assertFalse(fixture.snapshot_source().parent.exists())

    def test_clean_status_fails_when_head_lacks_the_artifact_task(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            fixture._git("rm", "--cached", ".mise/tasks/extension/artifact/avro")
            with (fixture.root / ".gitignore").open("a", encoding="utf-8") as ignore:
                ignore.write(".mise/tasks/extension/artifact/avro\n")
            fixture._git("add", ".gitignore")
            fixture._git(
                "-c", "commit.gpgsign=false", "commit", "-qm", "ignore task"
            )

            result = fixture.run()

            self.assertNotEqual(0, result.returncode)
            self.assertFalse(
                (fixture.root / ".morphir/build/extensions/avro/release.json").exists()
            )

    def test_clean_status_fails_when_head_lacks_root_cargo_inputs(self) -> None:
        for missing in ("Cargo.toml", "Cargo.lock"):
            with (
                self.subTest(missing=missing),
                tempfile.TemporaryDirectory() as temporary_directory,
            ):
                fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
                fixture._git("rm", missing)
                fixture._git(
                    "-c", "commit.gpgsign=false", "commit", "-qm", "remove cargo input"
                )

                result = fixture.run()

                self.assertNotEqual(0, result.returncode)
                self.assertIn("required provenance input is missing", result.stderr)
                self.assertFalse(
                    (fixture.root / ".morphir/build/extensions/avro/release.json").exists()
                )

    def test_clean_status_fails_when_head_lacks_mise_inputs(self) -> None:
        for missing in ("mise.toml", ".mise/tasks/test/avro-idl"):
            with (
                self.subTest(missing=missing),
                tempfile.TemporaryDirectory() as temporary_directory,
            ):
                fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
                fixture._git("rm", missing)
                fixture._git(
                    "-c", "commit.gpgsign=false", "commit", "-qm", "remove mise input"
                )

                result = fixture.run()

                self.assertNotEqual(0, result.returncode)
                self.assertIn("required provenance input is missing", result.stderr)
                self.assertFalse(
                    (fixture.root / ".morphir/build/extensions/avro/release.json").exists()
                )

    def test_clean_status_fails_when_head_lacks_source_or_test_inputs(self) -> None:
        for missing in (
            "crates/morphir-avro-extension/src/lib.rs",
            "crates/morphir-avro-extension/tests/guest.rs",
            "tests/ci/test_package_extension.py",
        ):
            with (
                self.subTest(missing=missing),
                tempfile.TemporaryDirectory() as temporary_directory,
            ):
                fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
                fixture._git("rm", missing)
                fixture._git(
                    "-c", "commit.gpgsign=false", "commit", "-qm", "remove provenance input"
                )

                result = fixture.run()

                self.assertNotEqual(0, result.returncode)
                self.assertIn("required provenance input is missing", result.stderr)
                self.assertFalse(
                    (fixture.root / ".morphir/build/extensions/avro/release.json").exists()
                )

    def test_python_older_than_3_11_fails_before_build(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            fixture._write_tool(
                "python3",
                'if [ "${1:-}" = "-c" ]; then exit 1; fi\nexec "$REAL_PYTHON" "$@"\n',
            )
            result = fixture.run({"REAL_PYTHON": sys.executable})

            self.assertNotEqual(0, result.returncode)
            self.assertIn("Python 3.11 or newer is required", result.stderr)
            self.assertFalse(fixture.log.exists())

    def test_java_older_than_11_fails_before_build(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            fixture._write_tool("java", 'echo \'java version "1.8.0_402"\' >&2\n')

            result = fixture.run()

            self.assertNotEqual(0, result.returncode)
            self.assertIn("Java 11 or newer is required", result.stderr)
            self.assertFalse(fixture.log.exists())


if __name__ == "__main__":
    unittest.main()
