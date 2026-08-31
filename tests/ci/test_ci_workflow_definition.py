"""Tests for the CI workflow's conservative change-classification wiring."""

from __future__ import annotations

import re
import os
from pathlib import Path
import subprocess
import tempfile
import textwrap
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
            "docs-generated",
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

    def test_generated_docs_are_checked_independently(self) -> None:
        docs_generated = self.jobs["docs-generated"]

        self.assertNotIn("    needs:", docs_generated)
        self.assertNotIn("\n    if:", docs_generated)
        self.assertIn("      - name: Checkout repository", docs_generated)
        self.assertIn("        uses: actions/checkout@v7", docs_generated)
        self.assertIn(
            "        uses: jdx/mise-action@v4\n        with:\n          install: false",
            docs_generated,
        )
        self.assertIn("        run: mise run --skip-tools docs:generate", docs_generated)
        self.assertIn(
            "          if ! docs_status=\"$(git status --porcelain --untracked-files=all -- docs/)\"; then",
            docs_generated,
        )
        self.assertIn(
            '            echo "::error::Unable to inspect generated documentation."',
            docs_generated,
        )
        self.assertIn("          if [ -n \"$docs_status\" ]; then", docs_generated)
        self.assertIn(
            "            git status --short --untracked-files=all -- docs/",
            docs_generated,
        )
        self.assertIn("            git diff -- docs/", docs_generated)
        self.assertIn("            exit 1", docs_generated)

    def test_generated_docs_drift_check_fails_when_git_inspection_fails(self) -> None:
        docs_generated = self.jobs["docs-generated"]
        marker = "      - name: Check docs are up to date\n"
        drift_step = docs_generated.split(marker, 1)[1]
        drift_script = textwrap.dedent(drift_step.split("        run: |\n", 1)[1])

        with tempfile.TemporaryDirectory() as temporary_directory:
            temporary_root = Path(temporary_directory)
            fake_bin = temporary_root / "bin"
            fake_bin.mkdir()
            fake_git = fake_bin / "git"
            fake_git.write_text(
                "#!/bin/sh\necho 'fatal: unable to inspect repository' >&2\nexit 1\n",
                encoding="utf-8",
            )
            fake_git.chmod(0o755)

            result = subprocess.run(
                ["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", drift_script],
                check=False,
                cwd=temporary_root,
                capture_output=True,
                text=True,
                env={**os.environ, "PATH": f"{fake_bin}{os.pathsep}{os.environ['PATH']}"},
            )

            self.assertNotEqual(0, result.returncode)
            self.assertIn(
                "Unable to inspect generated documentation.",
                result.stdout + result.stderr,
            )

    def test_deleted_generated_docs_are_detected_as_untracked(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            temporary_root = Path(temporary_directory)
            clone = temporary_root / "repository"
            global_config = temporary_root / "gitconfig"
            hooks = temporary_root / "hooks"
            hooks.mkdir()
            global_config.write_text("[commit]\n\tgpgSign = false\n", encoding="utf-8")
            git_environment = {
                **os.environ,
                "GIT_CONFIG_GLOBAL": str(global_config),
                "GIT_CONFIG_NOSYSTEM": "1",
                "GIT_CONFIG_SYSTEM": os.devnull,
            }

            subprocess.run(
                ["git", "clone", "--local", str(REPOSITORY_ROOT), str(clone)],
                check=True,
                capture_output=True,
                text=True,
                env=git_environment,
            )
            for key, value in (
                ("user.email", "ci@example.invalid"),
                ("user.name", "CI Test"),
                ("core.hooksPath", str(hooks)),
                ("commit.gpgSign", "false"),
            ):
                subprocess.run(
                    ["git", "config", key, value],
                    check=True,
                    cwd=clone,
                    env=git_environment,
                )

            subprocess.run(
                ["git", "rm", "docs/llms.txt"],
                check=True,
                cwd=clone,
                env=git_environment,
            )
            subprocess.run(
                ["git", "commit", "-qm", "delete generated docs"],
                check=True,
                cwd=clone,
                env=git_environment,
            )
            for task in ("releases", "llms-txt"):
                subprocess.run(
                    ["sh", str(clone / ".mise" / "tasks" / "docs" / task)],
                    check=True,
                    cwd=clone,
                    env=git_environment,
                )

            generated = clone / "docs" / "llms.txt"
            self.assertTrue(generated.is_file())
            status = subprocess.run(
                ["git", "status", "--porcelain", "--untracked-files=all", "--", "docs/"],
                check=True,
                cwd=clone,
                capture_output=True,
                text=True,
                env=git_environment,
            )
            self.assertIn("?? docs/llms.txt", status.stdout.splitlines())

    def test_expensive_docs_job_only_builds_rust_documentation(self) -> None:
        docs = self.jobs["docs"]

        self.assertIn("    needs: [lint-rust]", docs)
        self.assertIn(
            "    if: ${{ !cancelled() && needs.lint-rust.result == 'success' }}",
            docs,
        )
        self.assertIn("        run: cargo doc --no-deps", docs)
        self.assertNotIn("docs:generate", docs)
        self.assertNotIn("git diff --quiet docs/", docs)

    def test_release_shell_and_yaml_checks_are_always_independent(self) -> None:
        for job_name in ("test-release-workflow", "lint-shell", "lint-yaml"):
            with self.subTest(job=job_name):
                job = self.jobs[job_name]
                self.assertNotIn("changes", job)
                self.assertNotIn("lint-rust", job)


if __name__ == "__main__":
    unittest.main()
