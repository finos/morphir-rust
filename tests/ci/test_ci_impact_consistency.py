"""The impact config and the extensions registry must name real crates and jobs."""

from __future__ import annotations

import re
import shutil
import tomllib
import unittest

from ci_impact_test_support import *

CI_WORKFLOW = REPOSITORY_ROOT / ".github" / "workflows" / "ci.yml"


class ImpactConsistencyTests(unittest.TestCase):
    def test_native_mck_setup_uses_portable_paths_and_only_required_tools(self) -> None:
        settings = tomllib.loads((REPOSITORY_ROOT / "mise.toml").read_text())
        self.assertNotIn("PATH", settings["env"])
        self.assertEqual(["{{config_root}}/.gems/bin"], settings["env"]["_"]["path"])
        job = self.workflow.split("  kit-conformance:\n", 1)[1].split("  lint-shell:\n", 1)[0]
        self.assertIn("MISE_ENABLE_TOOLS: rust,bun", job)
        self.assertIn('CARGO_NET_GIT_FETCH_WITH_CLI: "true"', job)
        self.assertIn("git config --global core.longpaths true", job)
        self.assertLess(job.index("core.longpaths true"), job.index("uses: jdx/mise-action"))
        self.assertLess(job.index("core.longpaths true"), job.index("cargo test --locked"))
        self.assertLess(job.index("CARGO_NET_GIT_FETCH_WITH_CLI"), job.index("    steps:"))

    def test_native_mck_inputs_route_to_cross_platform_conformance(self) -> None:
        paths = (REPOSITORY_ROOT / ".github/ci-impact.toml").read_text()
        section = paths.split("[jobs.kit-conformance]", 1)[1].split("[jobs.", 1)[0]
        for path in (".config/mck-cli.json", "vendor/morphir-mck/**", "third_party/morphir-package-tough/**", "scripts/check-mck*", "scripts/released-cli*"):
            self.assertIn(path, section)
        job = self.workflow.split("  kit-conformance:\n", 1)[1].split("  lint-shell:\n", 1)[0]
        for os in ("ubuntu-latest", "macos-15", "windows-2025"):
            self.assertIn(os, job)
        self.assertIn("mise run tools:test", job)
        self.assertIn("mise run check:kit", job)
        self.assertNotIn("mise run check:kit-legacy", job)
        self.assertNotIn("hashFiles('.config/mck-cli.json')", job)
        self.assertNotIn(".dev/out/mck/legacy-report.json", job)
        self.assertIn(".dev/out/mck/report.html", job)
        self.assertIn("mck-report-${{ matrix.os }}", job)

    def test_native_mck_upload_retains_reports_inside_hidden_dev_directory(self) -> None:
        job = self.workflow.split("  kit-conformance:\n", 1)[1].split("  lint-shell:\n", 1)[0]
        upload = job.split("      - name: Upload kit report\n", 1)[1]
        self.assertIn("if: always()", upload)
        self.assertIn("include-hidden-files: true", upload)
        for report in ("report.json", "report.html"):
            self.assertIn(f".dev/out/mck/{report}", upload)

    def test_native_mck_is_default_and_legacy_runner_is_retired(self) -> None:
        settings = tomllib.loads((REPOSITORY_ROOT / "mise.toml").read_text())
        self.assertEqual("bun run scripts/check-mck.ts", settings["tasks"]["check:kit"]["run"])
        self.assertEqual(["check:kit"], settings["tasks"]["check:kit-native"]["depends"])
        self.assertFalse((REPOSITORY_ROOT / ".mise/tasks/check/kit").exists())
        for retired in (".mise/tasks/check/kit-legacy", ".config/mck-driver-version", "crates/morphir-mck-adapter/tests/legacy_report.rs"):
            self.assertFalse((REPOSITORY_ROOT / retired).exists(), retired)

    @unittest.skipUnless(shutil.which("cargo"), "cargo is not installed")
    def test_rust_bundle_uses_selective_ci_routing(self) -> None:
        workspace = graph.load_workspace(REPOSITORY_ROOT)
        all_extensions = tuple(e.short_id for e in self.extensions)
        self.assertIn("rust", all_extensions)
        cases = [
            ("crates/morphir-rust-binding/src/lib.rs", ("rust",)),
            ("crates/morphir-python-binding/src/lib.rs", ("python",)),
            ("crates/morphir-gleam-binding/src/lib.rs", ("gleam",)),
            ("README.md", ()),
            (".mise/tasks/extension/artifact/rust", ("rust",)),
            (".mise/tasks/extension/artifact/python", ("python",)),
            (".mise/tasks/extension/artifact/gleam", ("gleam",)),
            (".github/extensions.toml", all_extensions),
            ("scripts/package_extension.py", all_extensions),
            ("scripts/extension_packaging/model.py", all_extensions),
            ("crates/morphir-daemon/src/lib.rs", all_extensions),
            ("crates/morphir-extension-sdk/src/lib.rs", all_extensions),
        ]
        for path, expected in cases:
            with self.subTest(path=path):
                plan = classify.plan_changes([path], self.impact, workspace, self.extensions)
                self.assertFalse(plan.all)
                self.assertEqual(expected, tuple(e.short_id for e in plan.extensions))
                if path.startswith("crates/morphir-rust-binding/"):
                    self.assertEqual(("morphir-rust-binding",), plan.crates)
                    self.assertTrue(plan.rust)
                    self.assertFalse(plan.jobs["test-extism"])

    def test_rust_guest_tests_are_owned_by_the_bundle_task(self) -> None:
        task = REPOSITORY_ROOT / ".mise/tasks/extension/artifact/rust"
        self.assertTrue(task.is_file())
        script = task.read_text(encoding="utf-8")
        self.assertIn("--features wasm-host-tests --test wasm -- --ignored", script)
        self.assertIn("--test rust_extension -- --ignored", script)
        extism = self.workflow.split("  test-extism:\n", 1)[1].split("  extension-bundle:\n", 1)[0]
        self.assertNotIn("morphir-rust-binding", extism)

    def test_elm_native_guest_tests_are_owned_by_the_bundle_task(self) -> None:
        task = REPOSITORY_ROOT / ".mise/tasks/extension/artifact/elm-native"
        self.assertTrue(task.is_file())
        script = task.read_text(encoding="utf-8")
        self.assertIn("--features wasm-host-tests --test wasm -- --ignored", script)
        self.assertIn("--test elm_native_extension -- --ignored", script)
        extism = self.workflow.split("  test-extism:\n", 1)[1].split("  extension-bundle:\n", 1)[0]
        self.assertNotIn("morphir-elm-binding", extism)

    @classmethod
    def setUpClass(cls) -> None:
        cls.impact = config.load_config(REPOSITORY_ROOT / ".github" / "ci-impact.toml")
        cls.extensions = classify.load_extensions(REPOSITORY_ROOT / ".github" / "extensions.toml")
        cls.workflow = CI_WORKFLOW.read_text(encoding="utf-8")

    @unittest.skipUnless(shutil.which("cargo"), "cargo is not installed")
    def test_every_configured_crate_is_a_workspace_member(self) -> None:
        workspace = graph.load_workspace(REPOSITORY_ROOT)
        for job in self.impact.jobs:
            for crate in sorted(job.crates):
                with self.subTest(job=job.name, crate=crate):
                    self.assertIn(crate, workspace.members)
        for crate in sorted(self.impact.extension_crates):
            with self.subTest(crate=crate):
                self.assertIn(crate, workspace.members)
        for extension in self.extensions:
            with self.subTest(extension=extension.short_id):
                self.assertIn(extension.package, workspace.members)

    def test_every_configured_job_has_an_output_and_a_workflow_job(self) -> None:
        for job in self.impact.jobs:
            with self.subTest(job=job.name):
                self.assertIn(f"      {outputs.output_key(job.name)}: ", self.workflow)
                self.assertRegex(self.workflow, rf"(?m)^  {re.escape(job.name)}:\n")

    def test_every_extension_has_an_artifact_task(self) -> None:
        for extension in self.extensions:
            with self.subTest(extension=extension.short_id):
                task = REPOSITORY_ROOT / ".mise" / "tasks" / "extension" / "artifact" / extension.short_id
                self.assertTrue(task.is_file(), f"missing mise task for {extension.short_id}")


if __name__ == "__main__":
    unittest.main()
