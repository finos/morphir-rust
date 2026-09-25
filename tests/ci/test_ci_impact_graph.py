"""Tests for the reverse-dependency graph built from cargo metadata."""

from __future__ import annotations

import shutil
import unittest

from ci_impact_test_support import *


class WorkspaceGraphTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workspace = graph.workspace_from_metadata(fake_metadata())

    def test_members_and_default_members(self) -> None:
        self.assertIn("morphir-ext-example", self.workspace.members)
        self.assertNotIn("morphir-ext-example", self.workspace.default_members)
        self.assertIn("morphir-core", self.workspace.default_members)

    def test_direct_dependents_include_dev_and_build_edges(self) -> None:
        self.assertEqual(
            frozenset({"morphir-extension-sdk", "morphir-projection"}),
            self.workspace.dependents["morphir-core"],
        )
        self.assertIn("morphir-daemon", self.workspace.dependents["morphir-avro-extension"])
        self.assertIn("morphir-avro-extension", self.workspace.dependents["morphir-extension-sdk"])

    def test_external_dependencies_are_ignored(self) -> None:
        self.assertNotIn("serde", self.workspace.dependents)
        self.assertNotIn("serde", self.workspace.members)

    def test_leaf_change_affects_only_itself(self) -> None:
        self.assertEqual(
            frozenset({"morphir-python-binding"}),
            graph.affected_closure(self.workspace, ["morphir-python-binding"]),
        )

    def test_core_change_affects_every_dependent(self) -> None:
        self.assertEqual(
            self.workspace.members,
            graph.affected_closure(self.workspace, ["morphir-core"]),
        )

    def test_dev_dependency_edge_counts(self) -> None:
        self.assertEqual(
            frozenset({"morphir-avro-extension", "morphir-daemon"}),
            graph.affected_closure(self.workspace, ["morphir-avro-extension"]),
        )

    def test_unknown_crate_is_kept_so_callers_can_fail_safe(self) -> None:
        self.assertEqual(
            frozenset({"not-a-crate"}),
            graph.affected_closure(self.workspace, ["not-a-crate"]),
        )

    @unittest.skipUnless(shutil.which("cargo"), "cargo is not installed")
    def test_load_workspace_reads_the_real_workspace(self) -> None:
        workspace = graph.load_workspace(REPOSITORY_ROOT)
        self.assertIn("morphir-core", workspace.members)
        self.assertIn("morphir-host-native", workspace.dependents["morphir-extension-sdk"])
        self.assertNotIn("morphir-ext-example", workspace.default_members)


if __name__ == "__main__":
    unittest.main()
