"""Tests for turning changed paths into a CI plan."""

from __future__ import annotations

import unittest

from ci_impact_test_support import *


class ClassifyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.config = config.parse_config(fake_config_data())
        self.workspace = graph.workspace_from_metadata(fake_metadata())
        self.extensions = fake_extensions()

    def plan(self, *paths: str) -> classify.Plan:
        return classify.plan_changes(paths, self.config, self.workspace, self.extensions)

    def test_crate_for_path(self) -> None:
        self.assertEqual("morphir-core", classify.crate_for_path("crates/morphir-core/src/lib.rs"))
        self.assertEqual("morphir-core", classify.crate_for_path("crates/morphir-core/Cargo.toml"))
        self.assertIsNone(classify.crate_for_path("crates"))
        self.assertIsNone(classify.crate_for_path("crates/"))
        self.assertIsNone(classify.crate_for_path("docs/crates/morphir-core/x.md"))

    def test_leaf_binding_change_affects_only_itself(self) -> None:
        plan = self.plan("crates/morphir-python-binding/src/lib.rs")
        self.assertFalse(plan.all)
        self.assertEqual(("morphir-python-binding",), plan.crates)
        self.assertEqual("-p morphir-python-binding", plan.cargo_packages)
        self.assertTrue(plan.rust)
        self.assertFalse(plan.jobs["kit-conformance"])
        self.assertFalse(plan.jobs["test-extism"])
        self.assertEqual(("python",), tuple(item.short_id for item in plan.extensions))

    def test_core_change_affects_dependents_and_all_extensions(self) -> None:
        plan = self.plan("crates/morphir-core/src/lib.rs")
        self.assertFalse(plan.all)
        self.assertEqual(sorted(self.workspace.members), list(plan.crates))
        self.assertNotIn("morphir-ext-example", plan.cargo_packages.split())
        self.assertIn("-p morphir-daemon", plan.cargo_packages)
        self.assertTrue(plan.jobs["kit-conformance"])
        self.assertTrue(plan.jobs["test-extism"])
        self.assertEqual(("avro", "python"), tuple(item.short_id for item in plan.extensions))

    def test_daemon_change_rebuilds_every_extension(self) -> None:
        plan = self.plan("crates/morphir-daemon/src/lib.rs")
        self.assertEqual(("avro", "python"), tuple(item.short_id for item in plan.extensions))

    def test_global_path_affects_everything(self) -> None:
        plan = self.plan("Cargo.lock")
        self.assertTrue(plan.all)
        self.assertEqual("", plan.cargo_packages)
        self.assertTrue(all(plan.jobs.values()))
        self.assertTrue(plan.rust)
        self.assertEqual(("avro", "python"), tuple(item.short_id for item in plan.extensions))
        self.assertTrue(any("Cargo.lock" in reason for reason in plan.reasons))

    def test_unclaimed_path_affects_everything(self) -> None:
        plan = self.plan("mystery.txt")
        self.assertTrue(plan.all)
        self.assertTrue(any("mystery.txt" in reason for reason in plan.reasons))

    def test_unknown_crate_directory_affects_everything(self) -> None:
        plan = self.plan("crates/not-a-member/src/lib.rs")
        self.assertTrue(plan.all)

    def test_docs_only_change_affects_no_crates(self) -> None:
        plan = self.plan("docs/guide.md", "README.md")
        self.assertFalse(plan.all)
        self.assertEqual((), plan.crates)
        self.assertEqual("", plan.cargo_packages)
        self.assertFalse(plan.rust)
        self.assertTrue(plan.jobs["docs-generated"])
        self.assertFalse(plan.jobs["lint-shell"])
        self.assertFalse(plan.jobs["kit-conformance"])
        self.assertEqual((), plan.extensions)

    def test_job_path_match_enables_job_without_crates(self) -> None:
        plan = self.plan(".mise/tasks/test/unit")
        self.assertFalse(plan.all)
        self.assertEqual((), plan.crates)
        self.assertTrue(plan.jobs["lint-shell"])
        self.assertFalse(plan.jobs["test-extism"])
        self.assertTrue(plan.rust)
        self.assertEqual("", plan.cargo_packages)

    def test_non_default_member_change_does_not_enable_shared_rust_jobs(self) -> None:
        plan = self.plan("crates/morphir-ext-example/src/lib.rs")
        self.assertFalse(plan.all)
        self.assertEqual(("morphir-ext-example",), plan.crates)
        self.assertEqual("", plan.cargo_packages)
        self.assertFalse(plan.rust)
        self.assertFalse(plan.jobs["test-extism"])

    def test_empty_change_set_affects_everything(self) -> None:
        plan = self.plan()
        self.assertTrue(plan.all)

    def test_full_plan_enables_everything(self) -> None:
        plan = classify.full_plan(self.config, self.workspace, self.extensions, "forced")
        self.assertTrue(plan.all)
        self.assertEqual(sorted(self.workspace.members), list(plan.crates))
        self.assertTrue(all(plan.jobs.values()))
        self.assertEqual(("forced",), plan.reasons)

    def test_load_extensions_reads_the_repository_registry(self) -> None:
        loaded = classify.load_extensions(REPOSITORY_ROOT / ".github" / "extensions.toml")
        self.assertIn(classify.Extension("python", "morphir-python-binding"), loaded)


if __name__ == "__main__":
    unittest.main()
