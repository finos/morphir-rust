"""Tests for the CI workflow's conservative change-classification wiring."""

from __future__ import annotations

import re
from pathlib import Path
import unittest


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
CI_WORKFLOW = REPOSITORY_ROOT / ".github" / "workflows" / "ci.yml"


def extract_job_blocks(workflow: str) -> dict[str, str]:
    """Extract top-level job blocks without parsing the full YAML document."""
    jobs = workflow.split("jobs:\n", 1)[1]
    matches = list(re.finditer(r"(?m)^  (?P<name>[A-Za-z0-9_-]+):\n", jobs))
    return {
        match.group("name"): jobs[
            match.start() : (
                matches[index + 1].start() if index + 1 < len(matches) else len(jobs)
            )
        ]
        for index, match in enumerate(matches)
    }


class CiWorkflowDefinitionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = CI_WORKFLOW.read_text(encoding="utf-8")
        cls.jobs = extract_job_blocks(cls.workflow)

    def test_changes_job_classifies_changed_paths_with_full_history(self) -> None:
        self.assertIn("changes", self.jobs)
        self.assertLess(list(self.jobs).index("changes"), list(self.jobs).index("lint-rust"))
        changes = self.jobs["changes"]

        self.assertIn("    outputs:\n      expensive: ${{ steps.classify.outputs.expensive }}", changes)
        self.assertIn("      - uses: actions/checkout@v7", changes)
        self.assertIn("          fetch-depth: 0", changes)
        self.assertIn(
            "          BASE_SHA: ${{ github.event.pull_request.base.sha || github.event.before }}",
            changes,
        )
        self.assertIn("          HEAD_SHA: ${{ github.sha }}", changes)
        self.assertIn("          python3 .github/scripts/classify_ci_changes.py", changes)
        self.assertIn("            --base \"$BASE_SHA\"", changes)
        self.assertIn("            --head \"$HEAD_SHA\"", changes)

    def test_lint_rust_is_the_root_of_the_expensive_job_chain(self) -> None:
        lint_rust = self.jobs["lint-rust"]

        self.assertIn("    needs: changes", lint_rust)
        self.assertIn(
            "    if: ${{ !cancelled() && (needs.changes.result != 'success' || needs.changes.outputs.expensive != 'false') }}",
            lint_rust,
        )

        for job_name in (
            "build-wasm",
            "workspace-wasm",
            "test-native-extension",
            "test-daemon-extension",
            "test-unit",
            "docs",
            "coverage",
        ):
            with self.subTest(job=job_name):
                job = self.jobs[job_name]
                self.assertIn("    needs: [lint-rust]", job)
                self.assertIn(
                    "    if: ${{ !cancelled() && needs.lint-rust.result == 'success' }}",
                    job,
                )

    def test_every_job_declares_a_ci_classification(self) -> None:
        expected_jobs = {
            "changes",
            "test-release-workflow",
            "lint-shell",
            "lint-yaml",
            "lint-rust",
            "build-wasm",
            "workspace-wasm",
            "test-native-extension",
            "test-daemon-extension",
            "test-unit",
            "docs",
            "coverage",
        }

        self.assertEqual(expected_jobs, set(self.jobs))

    def test_release_shell_and_yaml_checks_are_always_independent(self) -> None:
        for job_name in ("test-release-workflow", "lint-shell", "lint-yaml"):
            with self.subTest(job=job_name):
                job = self.jobs[job_name]
                self.assertNotIn("changes", job)
                self.assertNotIn("lint-rust", job)


if __name__ == "__main__":
    unittest.main()
