"""The impact config and the extensions registry must name real crates and jobs."""

from __future__ import annotations

import re
import shutil
import unittest

from ci_impact_test_support import *

CI_WORKFLOW = REPOSITORY_ROOT / ".github" / "workflows" / "ci.yml"


class ImpactConsistencyTests(unittest.TestCase):
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
