"""Native identity coverage shares the existing Windows candidate category."""

from __future__ import annotations

import unittest

from ci_impact_test_support import *
from test_ci_workflow_definition import extract_job_blocks


COMMON_SELECTED = "contains(fromJSON(needs.changes.outputs.crates), 'morphir-common')"
IDENTITY_SELECTED = "needs.changes.outputs.all == 'true' || " + COMMON_SELECTED
PROVIDER_SELECTED = (
    "needs.changes.outputs.all == 'true' || "
    "needs.changes.outputs.job_kit_conformance == 'true'"
)
WINDOWS_SELECTED = PROVIDER_SELECTED


class WindowsSourceIdentityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = (REPOSITORY_ROOT / ".github/workflows/ci.yml").read_text()
        cls.jobs = extract_job_blocks(cls.workflow)
        cls.job = cls.jobs["provider-windows-standard-user"]
        cls.impact = config.load_config(REPOSITORY_ROOT / ".github/ci-impact.toml")
        # The actual adapter dependency is what selects this existing job for
        # common changes. Metadata only; no Cargo build or parallel target use.
        cls.workspace = graph.load_workspace(REPOSITORY_ROOT)

    def test_existing_job_selects_common_or_provider_without_new_matrix(self) -> None:
        self.assertIn(
            "    if: ${{ !cancelled() && needs.changes.result == 'success' && ("
            + WINDOWS_SELECTED + ") }}",
            self.job,
        )
        for runner, target in (("windows-2025", "x86_64-pc-windows-msvc"), ("windows-11-arm", "aarch64-pc-windows-msvc")):
            self.assertIn("os: " + runner, self.job)
            self.assertIn("target: " + target, self.job)
        self.assertEqual(1, self.job.count("matrix:"))
        self.assertIn("      fail-fast: false", self.job)

    def test_native_identity_and_real_observation_steps_use_same_architecture(self) -> None:
        target_step = self.job.split("      - name: Install native source observation target\n", 1)[1].split("      - name:", 1)[0]
        identity_step = self.job.split("      - name: Test native common unit controls\n", 1)[1].split("      - name:", 1)[0]
        observed_step = self.job.split("      - name: Test native observed read routes\n", 1)[1].split("      - name:", 1)[0]
        for step in (target_step, identity_step, observed_step):
            self.assertIn("        if: ${{ " + IDENTITY_SELECTED + " }}", step)
        self.assertIn("rustup target add ${{ matrix.target }}", target_step)
        self.assertIn("cargo test --locked --target ${{ matrix.target }} -p morphir-common --lib\n", identity_step)
        self.assertIn("cargo test --locked --target ${{ matrix.target }} -p morphir-common -p morphir-devkit", observed_step)
        for test in ("source_read_observation", "source_read_scope_adversarial", "source_read_late_registration", "source_read_alias_route", "source_read_content_file", "file_identity_public"):
            self.assertIn("--test " + test, observed_step)
        self.assertNotIn("continue-on-error", self.job)

    def test_provider_work_and_evidence_keep_their_existing_job_policy(self) -> None:
        provider_step = self.job.split("      - name: Build and execute under a separate standard-user account\n", 1)[1].split("      - name:", 1)[0]
        upload_step = self.job.split("      - name: Upload ordinary-user candidate evidence\n", 1)[1]
        self.assertNotIn("        if:", provider_step)
        self.assertIn("        if: always()", upload_step)

    def test_native_root_acquisition_controls_run_under_the_same_common_scope(self) -> None:
        marker = "      - name: Test native common unit controls\n"
        self.assertIn(marker, self.job)
        step = self.job.split(marker, 1)[1].split("      - name:", 1)[0]
        self.assertIn("        if: ${{ " + IDENTITY_SELECTED + " }}", step)
        self.assertIn("cargo test --locked --target ${{ matrix.target }} -p morphir-common --lib\n", step)
        # The complete unit suite covers private full identities, root readers,
        # Windows UTF-16 name controls and future helpers without filter gaps.
        self.assertNotIn("--lib identity_tests", self.job)
        self.assertNotIn("--lib root_read_tests", self.job)
        self.assertNotIn("continue-on-error", step)

    def test_actual_source_tests_fixtures_and_dependencies_select_existing_windows_job(self) -> None:
        for path in (
            "crates/morphir-common/src/config/observation/identity.rs",
            "crates/morphir-common/src/file_identity.rs",
            "crates/morphir-common/src/config/observation/identity/identity_tests.rs",
            "crates/morphir-common/src/config/observation/root_read.rs",
            "crates/morphir-common/src/config/observation/root_read_tests.rs",
            "crates/morphir-common/tests/source_read_content_file.rs",
            "crates/morphir-common/tests/file_identity_public.rs",
            "crates/morphir-common/tests/source_observation_fixtures/primary.json",
            "crates/morphir-common/Cargo.toml",
            "crates/morphir-core/src/lib.rs",
        ):
            with self.subTest(path=path):
                plan = classify.plan_changes([path], self.impact, self.workspace, ())
                self.assertFalse(plan.all)
                self.assertIn("morphir-common", plan.crates)
                self.assertTrue(plan.jobs["kit-conformance"])
        for path in ("README.md", "docs/example.md", "crates/morphir-python-binding/src/lib.rs", "crates/morphir-devkit/src/lib.rs"):
            with self.subTest(path=path):
                plan = classify.plan_changes([path], self.impact, self.workspace, ())
                self.assertFalse(plan.all)
                self.assertNotIn("morphir-common", plan.crates)

    def test_schedule_full_and_conservative_fallback_select_both_paths(self) -> None:
        for paths in (("Cargo.lock",), (".github/workflows/ci.yml",), ("unclaimed.file",), ()):
            with self.subTest(paths=paths):
                plan = classify.plan_changes(paths, self.impact, self.workspace, ())
                self.assertTrue(plan.all)
        self.assertTrue(classify.full_plan(self.impact, self.workspace, (), "schedule/manual full").all)
        self.assertIn('[ "$EVENT_NAME" = "schedule" ]', self.jobs["changes"])
        self.assertIn('[ "$DISPATCH_FULL" = "true" ]', self.jobs["changes"])

    def test_aggregate_rejects_any_non_success_for_selected_windows_work(self) -> None:
        aggregate = self.jobs["ci-ok"]
        self.assertIn("      - provider-windows-standard-user\n", aggregate)
        self.assertIn("    if: ${{ always() }}", aggregate)
        step = aggregate.split("      - name: Require selected Windows candidate evidence\n", 1)[1].split("      - name:", 1)[0]
        self.assertIn(
            "        if: ${{ needs.changes.result == 'success' && (" + WINDOWS_SELECTED
            + ") && needs.provider-windows-standard-user.result != 'success' }}",
            step,
        )
        self.assertIn("exit 1", step)
        # Direct non-success comparison includes unexpected skips and missing
        # results; intentional unselected skips do not enter this step.
        self.assertIn("contains(needs.*.result, 'failure')", aggregate)
        self.assertIn("contains(needs.*.result, 'cancelled')", aggregate)


if __name__ == "__main__":
    unittest.main()
