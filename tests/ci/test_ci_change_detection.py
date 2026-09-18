"""End-to-end tests for the CI change classifier CLI."""

from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from ci_impact_test_support import *

CLASSIFIER_SCRIPT = REPOSITORY_ROOT / ".github" / "scripts" / "classify_ci_changes.py"


def _git_environment() -> dict[str, str]:
    environment = os.environ.copy()
    environment["GIT_CONFIG_GLOBAL"] = os.devnull
    environment["GIT_CONFIG_NOSYSTEM"] = "1"
    return environment


def _git(*arguments: str) -> list[str]:
    return ["git", "-c", "commit.gpgSign=false", "-c", f"core.hooksPath={os.devnull}", *arguments]


def _init_repository(repository: Path) -> None:
    subprocess.run(_git("init", "-q", str(repository)), check=True, env=_git_environment())
    for key, value in (("user.email", "ci@example.invalid"), ("user.name", "CI Test")):
        subprocess.run(_git("config", key, value), cwd=repository, check=True, env=_git_environment())


def _commit(repository: Path, message: str) -> str:
    subprocess.run(_git("add", "."), cwd=repository, check=True, env=_git_environment())
    subprocess.run(_git("commit", "-qm", message), cwd=repository, check=True, env=_git_environment())
    return subprocess.check_output(
        _git("rev-parse", "HEAD"), cwd=repository, text=True, env=_git_environment()
    ).strip()


def _seed_repository(repository: Path) -> None:
    """Create a fake repository with a config, an extensions registry, and a fake cargo."""
    (repository / ".github").mkdir()
    (repository / ".github" / "ci-impact.toml").write_text(
        "[global]\npaths = ['Cargo.lock']\n"
        "[safe]\nexact = ['README.md']\nprefixes = ['docs/']\n"
        "[jobs.docs-generated]\npaths = ['docs/**']\n"
        "[jobs.kit-conformance]\ncrates = ['morphir-projection']\n",
        encoding="utf-8",
    )
    (repository / ".github" / "extensions.toml").write_text(
        "[extensions.python]\npackage = 'morphir-python-binding'\n", encoding="utf-8"
    )
    (repository / "crates" / "morphir-python-binding" / "src").mkdir(parents=True)
    (repository / "crates" / "morphir-python-binding" / "src" / "lib.rs").write_text("", encoding="utf-8")
    (repository / "README.md").write_text("base\n", encoding="utf-8")
    (repository / "Cargo.lock").write_text("base\n", encoding="utf-8")


def _fake_cargo(directory: Path) -> Path:
    """Write a cargo stand-in that prints the fake metadata."""
    directory.mkdir(exist_ok=True)
    metadata = json.dumps(fake_metadata())
    shim = directory / "cargo_metadata.py"
    shim.write_text(f"import sys\nsys.stdout.write({metadata!r})\n", encoding="utf-8")
    if os.name == "nt":
        script = directory / "cargo.cmd"
        script.write_text(f'@echo off\r\n"{sys.executable}" "{shim}"\r\n', encoding="utf-8")
    else:
        script = directory / "cargo"
        script.write_text(f'#!/bin/sh\nexec "{sys.executable}" "{shim}"\n', encoding="utf-8")
        script.chmod(0o755)
    return directory


def _run_cli(repository: Path, *arguments: str, output: Path | None = None, fake_cargo: Path | None = None):
    environment = os.environ.copy()
    if output is not None:
        environment["GITHUB_OUTPUT"] = str(output)
    else:
        environment.pop("GITHUB_OUTPUT", None)
    if fake_cargo is not None:
        environment["PATH"] = f"{fake_cargo}{os.pathsep}{environment['PATH']}"
    return subprocess.run(
        [sys.executable, str(CLASSIFIER_SCRIPT), "--root", str(repository), *arguments],
        check=False,
        capture_output=True,
        text=True,
        env=environment,
    )


class ClassifierCliTests(unittest.TestCase):
    def test_zero_sha_accepts_only_full_zero_sha1_or_sha256(self) -> None:
        self.assertTrue(cli.is_zero_sha("0" * 40))
        self.assertTrue(cli.is_zero_sha("0" * 64))
        for value in ("0" * 39, "0" * 41, "0" * 63, "0" * 65, "0" * 39 + "1"):
            with self.subTest(value=value):
                self.assertFalse(cli.is_zero_sha(value))

    def test_changed_paths_preserves_filenames_containing_spaces(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary)
            _init_repository(repository)
            readme = repository / "README.md"
            readme.write_text("base\n", encoding="utf-8")
            base = _commit(repository, "base")
            readme.write_text("head\n", encoding="utf-8")
            (repository / "notes with space.txt").write_text("notes\n", encoding="utf-8")
            head = _commit(repository, "head")

            self.assertEqual(
                ("README.md", "notes with space.txt"),
                cli.changed_paths(repository, base, head),
            )

    def test_cli_writes_scoped_outputs_for_a_leaf_crate_change(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "repo"
            repository.mkdir()
            _init_repository(repository)
            _seed_repository(repository)
            base = _commit(repository, "base")
            (repository / "crates" / "morphir-python-binding" / "src" / "lib.rs").write_text("// change\n", encoding="utf-8")
            head = _commit(repository, "head")
            output = Path(temporary) / "github-output"
            fake_cargo = _fake_cargo(Path(temporary) / "bin")

            result = _run_cli(repository, "--base", base, "--head", head, output=output, fake_cargo=fake_cargo)

            self.assertEqual(0, result.returncode, result.stderr)
            lines = dict(line.split("=", 1) for line in output.read_text(encoding="utf-8").splitlines())
            self.assertEqual("false", lines["all"])
            self.assertEqual("-p morphir-python-binding", lines["cargo_packages"])
            self.assertEqual("false", lines["job_kit_conformance"])
            self.assertEqual([{"id": "python", "package": "morphir-python-binding"}], json.loads(lines["extensions"]))

    def test_cli_writes_all_true_for_readme_plus_lockfile(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "repo"
            repository.mkdir()
            _init_repository(repository)
            _seed_repository(repository)
            base = _commit(repository, "base")
            (repository / "Cargo.lock").write_text("head\n", encoding="utf-8")
            head = _commit(repository, "head")
            fake_cargo = _fake_cargo(Path(temporary) / "bin")

            result = _run_cli(repository, "--base", base, "--head", head, fake_cargo=fake_cargo)

            self.assertEqual(0, result.returncode, result.stderr)
            lines = dict(line.split("=", 1) for line in result.stdout.splitlines())
            self.assertEqual("true", lines["all"])
            self.assertEqual("", lines["cargo_packages"])

    def test_cli_full_flag_forces_everything_without_git(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "repo"
            repository.mkdir()
            _seed_repository(repository)
            fake_cargo = _fake_cargo(Path(temporary) / "bin")

            result = _run_cli(repository, "--full", fake_cargo=fake_cargo)

            self.assertEqual(0, result.returncode, result.stderr)
            lines = dict(line.split("=", 1) for line in result.stdout.splitlines())
            self.assertEqual("true", lines["all"])
            self.assertEqual("true", lines["job_docs_generated"])

    def test_cli_text_format_is_readable(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "repo"
            repository.mkdir()
            _seed_repository(repository)
            fake_cargo = _fake_cargo(Path(temporary) / "bin")

            result = _run_cli(repository, "--full", "--format", "text", fake_cargo=fake_cargo)

            self.assertEqual(0, result.returncode, result.stderr)
            self.assertIn("all: true", result.stdout)
            self.assertIn("docs-generated: run", result.stdout)

    def test_cli_text_format_never_writes_github_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "repo"
            repository.mkdir()
            _seed_repository(repository)
            fake_cargo = _fake_cargo(Path(temporary) / "bin")
            output = Path(temporary) / "github-output"

            result = _run_cli(repository, "--full", "--format", "text", output=output, fake_cargo=fake_cargo)

            self.assertEqual(0, result.returncode, result.stderr)
            self.assertIn("all: true", result.stdout)
            self.assertIn("docs-generated: run", result.stdout)
            self.assertTrue(not output.exists() or output.read_text(encoding="utf-8") == "")

    def test_cli_fails_safe_for_all_zero_base(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "repo"
            repository.mkdir()
            _init_repository(repository)
            _seed_repository(repository)
            head = _commit(repository, "head")
            fake_cargo = _fake_cargo(Path(temporary) / "bin")

            result = _run_cli(repository, "--base", "0" * 40, "--head", head, fake_cargo=fake_cargo)

            self.assertEqual(0, result.returncode)
            lines = dict(line.split("=", 1) for line in result.stdout.splitlines())
            self.assertEqual("true", lines["all"])
            self.assertIn("warning", result.stderr.lower())

    def test_cli_fails_safe_when_cargo_metadata_is_unavailable(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "repo"
            repository.mkdir()
            _init_repository(repository)
            _seed_repository(repository)
            base = _commit(repository, "base")
            (repository / "README.md").write_text("head\n", encoding="utf-8")
            head = _commit(repository, "head")
            broken = Path(temporary) / "bin"
            broken.mkdir()
            name = "cargo.cmd" if os.name == "nt" else "cargo"
            (broken / name).write_text("@echo off\r\nexit /b 1\r\n" if os.name == "nt" else "#!/bin/sh\nexit 1\n", encoding="utf-8")
            if os.name != "nt":
                (broken / name).chmod(0o755)

            result = _run_cli(repository, "--base", base, "--head", head, fake_cargo=broken)

            self.assertEqual(0, result.returncode)
            lines = dict(line.split("=", 1) for line in result.stdout.splitlines())
            self.assertEqual("true", lines["all"])
            self.assertIn("warning", result.stderr.lower())

    def test_cli_fails_safe_when_config_is_missing(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "repo"
            repository.mkdir()
            _init_repository(repository)
            (repository / "README.md").write_text("base\n", encoding="utf-8")
            base = _commit(repository, "base")
            (repository / "README.md").write_text("head\n", encoding="utf-8")
            head = _commit(repository, "head")

            result = _run_cli(repository, "--base", base, "--head", head)

            self.assertEqual(0, result.returncode)
            self.assertIn("all=true", result.stdout)
            self.assertIn("warning", result.stderr.lower())

    def test_git_fixture_commits_with_hostile_global_signing_and_hook_config(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            global_config = root / "global.gitconfig"
            hook_directory = root / "hooks"
            hook_directory.mkdir()
            hook = hook_directory / "pre-commit"
            hook.write_text("#!/bin/sh\nexit 42\n", encoding="utf-8")
            hook.chmod(0o755)
            global_config.write_text(
                f"[commit]\n\tgpgSign = true\n[core]\n\thooksPath = {hook_directory}\n",
                encoding="utf-8",
            )
            environment = {**os.environ, "GIT_CONFIG_GLOBAL": str(global_config), "GIT_CONFIG_NOSYSTEM": "1"}
            with mock.patch.dict(os.environ, environment, clear=True):
                repository = root / "repository"
                repository.mkdir()
                _init_repository(repository)
                (repository / "README.md").write_text("base\n", encoding="utf-8")
                base = _commit(repository, "base")
            self.assertRegex(base, r"^[0-9a-f]{40}$")


if __name__ == "__main__":
    unittest.main()
