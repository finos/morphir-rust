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


GATED_JOBS = {
    "lint-rust": "job_rust",
    "test-unit": "job_rust",
    "docs": "job_rust",
    "coverage": "job_rust",
    "kit-conformance": "job_kit_conformance",
    "provider-windows-standard-user": "job_kit_conformance",
    "workspace-wasm": "job_workspace_wasm",
    "test-native-extension": "job_test_native_extension",
    "test-daemon-extension": "job_test_daemon_extension",
    "test-extism": "job_test_extism",
    "example-wasm": "job_example_wasm",
    "lint-shell": "job_lint_shell",
    "lint-yaml": "job_lint_yaml",
    "docs-generated": "job_docs_generated",
    "test-release-workflow": "job_test_release_workflow",
}

RUST_CACHE_GROUPS = {
    "native": (
        "lint-rust",
        "test-unit",
        "docs",
        "kit-conformance",
        "provider-windows-standard-user",
        "workspace-wasm",
        "test-native-extension",
        "test-daemon-extension",
    ),
    "wasm": ("test-extism", "example-wasm", "extension-bundle"),
    "coverage": ("coverage",),
}

QUICK_JOBS = ("changes", "test-release-workflow", "lint-shell", "lint-yaml", "docs-generated", "ci-ok")


def gate(output: str) -> str:
    return (
        "    if: ${{ !cancelled() && needs.changes.result == 'success' && "
        f"(needs.changes.outputs.all == 'true' || needs.changes.outputs.{output} == 'true') }}}}"
    )


class CiWorkflowDefinitionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = CI_WORKFLOW.read_text(encoding="utf-8")
        cls.jobs = extract_job_blocks(cls.workflow)

    def test_workflow_triggers_include_schedule_and_manual_full_runs(self) -> None:
        header = self.workflow.split("jobs:\n", 1)[0]
        self.assertIn("  schedule:\n    - cron:", header)
        self.assertIn("  workflow_dispatch:\n    inputs:\n      full:", header)
        self.assertIn('  pull_request:\n    branches: ["main"]\n', header)
        self.assertNotIn("labeled", header)

    def test_concurrency_cancels_only_off_main(self) -> None:
        header = self.workflow.split("jobs:\n", 1)[0]
        self.assertIn("  group: ${{ github.workflow }}-${{ github.head_ref || github.ref }}", header)
        self.assertIn("  cancel-in-progress: ${{ github.ref != 'refs/heads/main' }}", header)

    def test_every_job_has_a_timeout(self) -> None:
        for job_name, job in self.jobs.items():
            with self.subTest(job=job_name):
                minutes = 15 if job_name in QUICK_JOBS else 45
                self.assertIn(f"    timeout-minutes: {minutes}\n", job)

    def test_changes_job_classifies_changed_paths_with_full_history(self) -> None:
        self.assertEqual("changes", list(self.jobs)[0])
        changes = self.jobs["changes"]
        for output in ("all", "crates", "cargo_packages", "extensions", *GATED_JOBS.values()):
            with self.subTest(output=output):
                self.assertIn(f"      {output}: ${{{{ steps.classify.outputs.{output} }}}}", changes)
        self.assertIn("      pull-requests: read", changes)
        self.assertIn("          fetch-depth: 0", changes)
        self.assertNotIn("jdx/mise-action", changes)
        self.assertIn("          BASE_SHA: ${{ github.event.pull_request.base.sha || github.event.before }}", changes)
        self.assertIn("          HEAD_SHA: ${{ github.sha }}", changes)
        self.assertIn("          EVENT_NAME: ${{ github.event_name }}", changes)
        self.assertIn("          DISPATCH_FULL: ${{ github.event.inputs.full }}", changes)
        self.assertIn("          PR_NUMBER: ${{ github.event.pull_request.number }}", changes)
        self.assertIn('[ "$EVENT_NAME" = "schedule" ]', changes)
        self.assertIn('[ "$DISPATCH_FULL" = "true" ]', changes)
        self.assertIn("gh pr view", changes)
        self.assertIn("--json labels", changes)
        self.assertIn("ci:full", changes)
        self.assertIn("          python3 .github/scripts/classify_ci_changes.py", changes)
        self.assertIn("            --base \"$BASE_SHA\"", changes)
        self.assertIn("            --head \"$HEAD_SHA\"", changes)

    def test_every_gated_job_depends_on_changes_and_uses_its_output(self) -> None:
        for job_name, output in GATED_JOBS.items():
            with self.subTest(job=job_name):
                job = self.jobs[job_name]
                self.assertIn("    needs: changes\n", job)
                self.assertIn(gate(output), job)

    def test_rust_jobs_share_caches_by_profile_and_save_only_on_main(self) -> None:
        self.assertNotIn("actions/cache@", self.workflow)
        for shared_key, job_names in RUST_CACHE_GROUPS.items():
            for job_name in job_names:
                with self.subTest(job=job_name):
                    job = self.jobs[job_name]
                    self.assertIn("        uses: Swatinem/rust-cache@v2\n", job)
                    self.assertIn(f"          shared-key: {shared_key}\n", job)
                    self.assertIn("          save-if: ${{ github.ref == 'refs/heads/main' }}\n", job)
                    self.assertIn("          cache-on-failure: true\n", job)
        cached_jobs = {name for names in RUST_CACHE_GROUPS.values() for name in names}
        for job_name in set(self.jobs) - cached_jobs:
            with self.subTest(job=job_name):
                self.assertNotIn("rust-cache", self.jobs[job_name])

    def test_extension_bundle_job_uses_the_matrix_from_changes(self) -> None:
        job = self.jobs["extension-bundle"]
        self.assertIn("    needs: changes\n", job)
        self.assertIn(
            "    if: ${{ !cancelled() && needs.changes.result == 'success' && needs.changes.outputs.extensions != '[]' }}",
            job,
        )
        self.assertIn("      fail-fast: false\n", job)
        self.assertIn("        include: ${{ fromJSON(needs.changes.outputs.extensions) }}", job)
        self.assertIn('        run: mise run "extension:artifact:${{ matrix.id }}"', job)
        self.assertIn("          name: morphir-${{ matrix.id }}-extension-bundle", job)
        self.assertIn("          path: .morphir/build/extensions/${{ matrix.id }}/*", job)

    def test_extension_bundle_job_proves_the_bundle_through_the_released_cli(self) -> None:
        job = self.jobs["extension-bundle"]
        self.assertIn('        run: mise run test:cli-release "${{ matrix.id }}"', job)
        # The bundle has to exist before the released CLI can publish it.
        self.assertLess(
            job.index('mise run "extension:artifact:${{ matrix.id }}"'),
            job.index("mise run test:cli-release"),
        )
        # morphir-elm-native is built into the CLI, so finos/morphir checks it; every other
        # bundle, the Rust one included, goes through the released CLI.
        self.assertIn("        if: matrix.id != 'elm-native'\n", job)
        self.assertNotIn("matrix.id != 'rust'", job)
        # The check uses a released CLI, never a checkout of finos/morphir.
        self.assertNotIn("repository: finos/morphir\n", job)
        version = (REPOSITORY_ROOT / ".config" / "morphir-cli-version").read_text(encoding="utf-8")
        self.assertRegex(version, r"^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$")
        task = (REPOSITORY_ROOT / ".mise" / "tasks" / "test" / "cli-release").read_text(encoding="utf-8")
        self.assertIn("scripts/released-cli.ts", task)
        installer = (REPOSITORY_ROOT / "scripts/released-cli.ts").read_text(encoding="utf-8")
        self.assertIn("https://github.com/finos/morphir/releases/download/v${options.version}", installer)
        self.assertIn(".sha256", installer)

    def test_a_relative_cli_override_survives_the_change_into_the_project(self) -> None:
        """MORPHIR_CLI=target/debug/morphir must still resolve after the task enters its project."""
        task = REPOSITORY_ROOT / ".mise" / "tasks" / "test" / "cli-release"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "bin").mkdir()
            cli = root / "bin" / "morphir"
            cli.write_text("#!/bin/sh\necho \"morphir 0.0.0 $(pwd -P)\"\nexit 7\n", encoding="utf-8")
            cli.chmod(0o755)
            bundle = root / "bundle"
            bundle.mkdir()
            (bundle / "stub.release.json").write_text("{}", encoding="utf-8")
            result = subprocess.run(
                ["sh", str(task), "avro"],
                cwd=REPOSITORY_ROOT,
                env={
                    **os.environ,
                    "MORPHIR_CLI": os.path.relpath(cli, REPOSITORY_ROOT),
                    "MORPHIR_EXTENSION_BUNDLE": str(bundle),
                },
                capture_output=True,
                text=True,
                check=False,
            )
        # The stub CLI exits 7 when it runs. A lost relative path exits 127 instead.
        self.assertEqual(7, result.returncode, result.stderr)
        self.assertNotIn("No such file", result.stderr)

    def test_released_cli_inputs_rebuild_every_bundle(self) -> None:
        impact = (REPOSITORY_ROOT / ".github" / "ci-impact.toml").read_text(encoding="utf-8")
        extensions = impact.split("[extensions]\n", 1)[1].split("\n[", 1)[0]
        self.assertIn('".config/morphir-cli-version"', extensions)
        self.assertIn('".mise/tasks/test/cli-release"', extensions)

    def test_rust_jobs_scope_cargo_to_affected_packages(self) -> None:
        self.assertIn("        run: mise run check:fmt", self.jobs["lint-rust"])
        self.assertIn("        run: mise run check:lint:rust -- ${{ needs.changes.outputs.cargo_packages }}", self.jobs["lint-rust"])
        self.assertIn("        run: mise run test:unit -- ${{ needs.changes.outputs.cargo_packages }}", self.jobs["test-unit"])
        self.assertIn("        run: mise run test:integration -- ${{ needs.changes.outputs.cargo_packages }}", self.jobs["test-unit"])
        self.assertIn("        run: cargo doc --no-deps ${{ needs.changes.outputs.cargo_packages }}", self.jobs["docs"])
        self.assertIn("          CI_PACKAGES: ${{ needs.changes.outputs.crates }}", self.jobs["coverage"])

    def test_every_job_is_expected(self) -> None:
        self.assertEqual({"changes", "extension-bundle", "ci-ok", *GATED_JOBS}, set(self.jobs))

    def test_ci_ok_aggregates_every_other_job(self) -> None:
        ci_ok = self.jobs["ci-ok"]
        self.assertEqual("ci-ok", list(self.jobs)[-1])
        self.assertIn("    if: ${{ always() }}", ci_ok)
        for job_name in ("changes", "extension-bundle", *GATED_JOBS):
            with self.subTest(job=job_name):
                self.assertIn(f"      - {job_name}\n", ci_ok)
        self.assertIn("needs.changes.result != 'success'", ci_ok)
        self.assertIn("contains(needs.*.result, 'failure')", ci_ok)
        self.assertIn("contains(needs.*.result, 'cancelled')", ci_ok)

    def test_generated_docs_are_checked_independently(self) -> None:
        docs_generated = self.jobs["docs-generated"]
        self.assertIn("        uses: jdx/mise-action@v4\n        with:\n          install: false", docs_generated)
        self.assertIn("        run: mise run --skip-tools docs:generate", docs_generated)
        self.assertIn(
            "          if ! docs_status=\"$(git status --porcelain --untracked-files=all -- docs/)\"; then",
            docs_generated,
        )
        self.assertIn('            echo "::error::Unable to inspect generated documentation."', docs_generated)
        self.assertIn("            git diff -- docs/", docs_generated)

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

if __name__ == "__main__":
    unittest.main()
