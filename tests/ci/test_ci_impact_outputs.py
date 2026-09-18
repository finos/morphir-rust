"""Tests for GitHub output rendering."""

from __future__ import annotations

import json
import unittest

from ci_impact_test_support import *


class OutputTests(unittest.TestCase):
    def setUp(self) -> None:
        self.config = config.parse_config(fake_config_data())
        self.workspace = graph.workspace_from_metadata(fake_metadata())
        self.extensions = fake_extensions()

    def test_output_key_uses_underscores(self) -> None:
        self.assertEqual("job_kit_conformance", outputs.output_key("kit-conformance"))
        self.assertEqual("job_docs_generated", outputs.output_key("docs-generated"))

    def test_render_github_for_a_leaf_change(self) -> None:
        plan = classify.plan_changes(
            ["crates/morphir-python-binding/src/lib.rs"], self.config, self.workspace, self.extensions
        )
        rendered = outputs.render_github(plan)
        lines = dict(line.split("=", 1) for line in rendered.splitlines())

        self.assertEqual("false", lines["all"])
        self.assertEqual(["morphir-python-binding"], json.loads(lines["crates"]))
        self.assertEqual("-p morphir-python-binding", lines["cargo_packages"])
        self.assertEqual("true", lines["job_rust"])
        self.assertEqual("false", lines["job_kit_conformance"])
        self.assertEqual("false", lines["job_test_extism"])
        self.assertEqual("false", lines["job_lint_shell"])
        self.assertEqual("false", lines["job_docs_generated"])
        self.assertEqual(
            [{"id": "python", "package": "morphir-python-binding"}],
            json.loads(lines["extensions"]),
        )
        self.assertTrue(rendered.endswith("\n"))
        self.assertEqual(1, rendered.count("\nreasons="))

    def test_render_github_for_full_plan(self) -> None:
        plan = classify.full_plan(self.config, self.workspace, self.extensions, "forced")
        lines = dict(line.split("=", 1) for line in outputs.render_github(plan).splitlines())
        self.assertEqual("true", lines["all"])
        self.assertEqual("", lines["cargo_packages"])
        self.assertEqual("true", lines["job_rust"])
        self.assertEqual(2, len(json.loads(lines["extensions"])))

    def test_render_github_empty_extension_matrix_is_json_empty_list(self) -> None:
        plan = classify.plan_changes(["docs/a.md"], self.config, self.workspace, self.extensions)
        lines = dict(line.split("=", 1) for line in outputs.render_github(plan).splitlines())
        self.assertEqual("[]", lines["extensions"])
        self.assertEqual("false", lines["job_rust"])

    def test_render_text_lists_jobs_and_reasons(self) -> None:
        plan = classify.plan_changes(["docs/a.md"], self.config, self.workspace, self.extensions)
        text = outputs.render_text(plan)
        self.assertIn("all: false", text)
        self.assertIn("docs-generated: run", text)
        self.assertIn("kit-conformance: skip", text)
        self.assertIn("extensions: (none)", text)


if __name__ == "__main__":
    unittest.main()
