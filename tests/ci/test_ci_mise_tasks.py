"""Tests that CI-facing mise tasks accept package scoping."""

from __future__ import annotations

from pathlib import Path
import unittest

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
TASKS = REPOSITORY_ROOT / ".mise" / "tasks"


class MiseTaskScopingTests(unittest.TestCase):
    def test_lint_rust_forwards_extra_cargo_arguments(self) -> None:
        script = (TASKS / "check" / "lint" / "rust").read_text(encoding="utf-8")
        self.assertIn('cargo clippy "$@" --all-targets --all-features -- -D warnings', script)

    def test_coverage_tasks_honour_ci_packages(self) -> None:
        for name in ("coverage", "coverage-report"):
            with self.subTest(task=name):
                script = (TASKS / "check" / name).read_text(encoding="utf-8")
                self.assertIn("CI_PACKAGES", script)
                self.assertIn("in_ci_packages", script)

    def test_ci_impact_task_exists_and_calls_the_classifier(self) -> None:
        script = (TASKS / "ci" / "impact").read_text(encoding="utf-8")
        self.assertIn("classify_ci_changes.py", script)
        self.assertIn("--format text", script)
        self.assertIn("git merge-base", script)


if __name__ == "__main__":
    unittest.main()
