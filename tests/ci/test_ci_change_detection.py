"""Tests for the conservative CI change classifier."""

from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
CLASSIFIER_SCRIPT = REPOSITORY_ROOT / ".github" / "scripts" / "classify_ci_changes.py"


def load_classifier():
    """Import the repository classifier script as a module."""
    if not CLASSIFIER_SCRIPT.exists():
        raise AssertionError(f"classifier script is missing: {CLASSIFIER_SCRIPT}")
    spec = importlib.util.spec_from_file_location(
        "classify_ci_changes_under_test", CLASSIFIER_SCRIPT
    )
    if spec is None or spec.loader is None:
        raise AssertionError(f"cannot load {CLASSIFIER_SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class CiChangeDetectionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.classifier = load_classifier()

    def test_safe_exact_paths_are_metadata(self) -> None:
        for path in (
            "README.md",
            "CHANGELOG.md",
            "CONTRIBUTING.md",
            "MAINTAINERS.md",
            "LICENSE",
            "LICENSE.spdx",
            "NOTICE",
            "AGENTS.md",
            "renovate.json",
        ):
            with self.subTest(path=path):
                self.assertTrue(self.classifier.is_metadata_path(path))

    def test_safe_prefix_paths_are_metadata(self) -> None:
        for path in (
            ".beads/issues.jsonl",
            "docs/getting-started.md",
            ".github/ISSUE_TEMPLATE/bug.md",
            ".github/PULL_REQUEST_TEMPLATE/change.md",
        ):
            with self.subTest(path=path):
                self.assertTrue(self.classifier.is_metadata_path(path))

    def test_unsafe_paths_are_not_metadata(self) -> None:
        for path in (
            ".gitignore",
            "Cargo.toml",
            "Cargo.lock",
            "crates/morphir-core/src/lib.rs",
            ".github/workflows/ci.yml",
            ".github/scripts/classify_ci_changes.py",
            "mise.toml",
            "unknown.txt",
        ):
            with self.subTest(path=path):
                self.assertFalse(self.classifier.is_metadata_path(path))

    def test_requires_expensive_ci_for_empty_or_unsafe_changes(self) -> None:
        self.assertTrue(self.classifier.requires_expensive_ci([]))
        self.assertFalse(
            self.classifier.requires_expensive_ci(
                ["README.md", "docs/getting-started.md"]
            )
        )
        self.assertTrue(
            self.classifier.requires_expensive_ci(["README.md", "Cargo.toml"])
        )

    def test_zero_sha_accepts_only_full_zero_sha1_or_sha256(self) -> None:
        self.assertTrue(self.classifier.is_zero_sha("0" * 40))
        self.assertTrue(self.classifier.is_zero_sha("0" * 64))
        for value in ("0" * 39, "0" * 41, "0" * 63, "0" * 65, "0" * 39 + "1"):
            with self.subTest(value=value):
                self.assertFalse(self.classifier.is_zero_sha(value))

    def test_changed_paths_preserves_filenames_containing_spaces(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary)
            self._init_repository(repository)
            readme = repository / "README.md"
            readme.write_text("base\n", encoding="utf-8")
            base = self._commit(repository, "base")
            readme.write_text("head\n", encoding="utf-8")
            spaced = repository / "notes with space.txt"
            spaced.write_text("notes\n", encoding="utf-8")
            head = self._commit(repository, "head")

            self.assertEqual(
                ("README.md", "notes with space.txt"),
                self.classifier.changed_paths(repository, base, head),
            )

    def test_unsafe_rename_into_docs_still_requires_expensive_ci(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary)
            self._init_repository(repository)
            unsafe_source = repository / "crates" / "example" / "src" / "lib.rs"
            unsafe_source.parent.mkdir(parents=True)
            unsafe_source.write_text("unsafe\n", encoding="utf-8")
            base = self._commit(repository, "base")
            destination = repository / "docs" / "lib.rs"
            destination.parent.mkdir()
            unsafe_source.rename(destination)
            head = self._commit(repository, "rename into docs")

            paths = self.classifier.changed_paths(repository, base, head)

            self.assertTrue(self.classifier.requires_expensive_ci(paths))

    def test_cli_writes_false_for_readme_only_commit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary)
            self._init_repository(repository)
            readme = repository / "README.md"
            readme.write_text("base\n", encoding="utf-8")
            base = self._commit(repository, "base")
            readme.write_text("head\n", encoding="utf-8")
            head = self._commit(repository, "head")
            output = repository / "github-output"
            result = self._run_cli(repository, base, head, output)

            self.assertEqual(0, result.returncode, result.stderr)
            self.assertEqual("", result.stdout)
            self.assertEqual("expensive=false\n", output.read_text(encoding="utf-8"))
            self.assertEqual("", result.stderr)

    def test_cli_fails_safe_for_all_zero_base(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary)
            self._init_repository(repository)
            (repository / "README.md").write_text("head\n", encoding="utf-8")
            head = self._commit(repository, "head")
            result = self._run_cli(repository, "0" * 40, head)

            self.assertEqual(0, result.returncode)
            self.assertEqual("expensive=true\n", result.stdout)
            self.assertIn("warning", result.stderr.lower())

    def test_cli_fails_safe_for_unknown_base_object(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary)
            self._init_repository(repository)
            (repository / "README.md").write_text("head\n", encoding="utf-8")
            head = self._commit(repository, "head")
            result = self._run_cli(repository, "f" * 40, head)

            self.assertEqual(0, result.returncode)
            self.assertEqual("expensive=true\n", result.stdout)
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
            environment = {
                **os.environ,
                "GIT_CONFIG_GLOBAL": str(global_config),
                "GIT_CONFIG_NOSYSTEM": "1",
            }
            with mock.patch.dict(os.environ, environment, clear=True):
                repository = root / "repository"
                repository.mkdir()
                self._init_repository(repository)
                (repository / "README.md").write_text("base\n", encoding="utf-8")

                base = self._commit(repository, "base")

            self.assertRegex(base, r"^[0-9a-f]{40}$")

    @staticmethod
    def _init_repository(repository: Path) -> None:
        subprocess.run(
            CiChangeDetectionTests._git_command("init", "-q", str(repository)),
            check=True,
            env=CiChangeDetectionTests._git_environment(),
        )
        subprocess.run(
            CiChangeDetectionTests._git_command(
                "config", "user.email", "ci@example.invalid"
            ),
            cwd=repository,
            check=True,
            env=CiChangeDetectionTests._git_environment(),
        )
        subprocess.run(
            CiChangeDetectionTests._git_command("config", "user.name", "CI Test"),
            cwd=repository,
            check=True,
            env=CiChangeDetectionTests._git_environment(),
        )

    @staticmethod
    def _commit(repository: Path, message: str) -> str:
        subprocess.run(
            CiChangeDetectionTests._git_command("add", "."),
            cwd=repository,
            check=True,
            env=CiChangeDetectionTests._git_environment(),
        )
        subprocess.run(
            CiChangeDetectionTests._git_command("commit", "-qm", message),
            cwd=repository,
            check=True,
            env=CiChangeDetectionTests._git_environment(),
        )
        return subprocess.check_output(
            CiChangeDetectionTests._git_command("rev-parse", "HEAD"),
            cwd=repository,
            text=True,
            env=CiChangeDetectionTests._git_environment(),
        ).strip()

    @staticmethod
    def _git_command(*arguments: str) -> list[str]:
        return [
            "git",
            "-c",
            "commit.gpgSign=false",
            "-c",
            f"core.hooksPath={os.devnull}",
            *arguments,
        ]

    @staticmethod
    def _git_environment() -> dict[str, str]:
        environment = os.environ.copy()
        environment["GIT_CONFIG_GLOBAL"] = os.devnull
        environment["GIT_CONFIG_NOSYSTEM"] = "1"
        return environment

    @staticmethod
    def _run_cli(
        repository: Path, base: str, head: str, output: Path | None = None
    ) -> subprocess.CompletedProcess[str]:
        environment = os.environ.copy()
        if output is not None:
            environment["GITHUB_OUTPUT"] = str(output)
        else:
            environment.pop("GITHUB_OUTPUT", None)
        return subprocess.run(
            [
                sys.executable,
                str(CLASSIFIER_SCRIPT),
                "--root",
                str(repository),
                "--base",
                base,
                "--head",
                head,
            ],
            check=False,
            capture_output=True,
            text=True,
            env=environment,
        )


if __name__ == "__main__":
    unittest.main()
