"""Tests for the crates.io publish workflow and its tag resolver."""

from __future__ import annotations

import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest

from ci_impact_test_support import (  # also puts .github/scripts on sys.path
    REPOSITORY_ROOT,
    SCRIPTS_DIRECTORY,
    classify,
    config,
    fake_metadata,
    graph,
)

import crate_release  # noqa: E402

CRATE_SCRIPT = SCRIPTS_DIRECTORY / "crate_release.py"
PUBLISH_WORKFLOW = REPOSITORY_ROOT / ".github" / "workflows" / "publish-crate.yml"
RELEASE_WORKFLOW = REPOSITORY_ROOT / ".github" / "workflows" / "release.yml"
COMMIT = "0123456789abcdef0123456789abcdef01234567"


def write_repository(root: Path, crate_manifest: str, crate: str = "morphir-config") -> None:
    """Write a workspace with one crate manifest."""
    (root / "Cargo.toml").write_text(
        '[workspace]\nmembers = ["crates/*"]\n\n[workspace.package]\nversion = "0.2.0"\n',
        encoding="utf-8",
    )
    crate_directory = root / "crates" / crate
    crate_directory.mkdir(parents=True)
    (crate_directory / "Cargo.toml").write_text(
        textwrap.dedent(crate_manifest), encoding="utf-8"
    )


def git(root: Path, *args: str) -> str:
    """Run git in a fixture repository and return its output."""
    return subprocess.run(
        ["git", "-C", os.fspath(root), *args],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def github_filter_matches(pattern: str, ref: str) -> bool:
    """Match a GitHub Actions tag filter: `*` stops at `/`, `**` does not."""
    expression = ""
    index = 0
    while index < len(pattern):
        if pattern.startswith("**", index):
            expression += ".*"
            index += 2
        elif pattern[index] == "*":
            expression += "[^/]*"
            index += 1
        else:
            expression += re.escape(pattern[index])
            index += 1
    return re.fullmatch(expression, ref) is not None


def push_tag_filters(workflow: str) -> list[str]:
    """Return the `on.push.tags` filters of a workflow."""
    block = workflow.split("  push:\n    tags:\n", 1)[1]
    filters = []
    for line in block.splitlines():
        match = re.fullmatch(r"      - [\"']?([^\"']+)[\"']?", line)
        if match is None:
            break
        filters.append(match.group(1))
    return filters


class ParseTagTests(unittest.TestCase):
    def test_splits_a_crate_tag_into_crate_and_version(self) -> None:
        self.assertEqual(
            ("morphir-config", "0.0.1"),
            crate_release.parse_tag("crates/morphir-config/v0.0.1"),
        )

    def test_accepts_prerelease_versions(self) -> None:
        self.assertEqual(
            ("morphir-core", "0.1.0-alpha.2"),
            crate_release.parse_tag("crates/morphir-core/v0.1.0-alpha.2"),
        )

    def test_rejects_malformed_tags(self) -> None:
        for tag in (
            "v0.0.1",
            "extension/avro/v0.2.0",
            "crates/morphir-config/0.0.1",
            "crates/morphir-config/v0.0",
            "crates/morphir-config/v01.0.0",
            "crates/morphir-config/v0.0.1-01",
            "crates/Morphir-Config/v0.0.1",
            "crates/../v0.0.1",
            "crates/a/b/v0.0.1",
            "crates//v0.0.1",
            "refs/tags/crates/morphir-config/v0.0.1",
        ):
            with self.subTest(tag=tag):
                with self.assertRaises(crate_release.CrateReleaseError):
                    crate_release.parse_tag(tag)


class ResolveReleaseTests(unittest.TestCase):
    def resolve(self, tag: str, manifest: str, crate: str = "morphir-config"):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            write_repository(root, manifest, crate)
            return crate_release.resolve_release(root, tag)

    def test_resolves_when_manifest_version_equals_tag_version(self) -> None:
        release = self.resolve(
            "crates/morphir-config/v0.0.1",
            '[package]\nname = "morphir-config"\nversion = "0.0.1"\n',
        )
        self.assertEqual("crates/morphir-config/v0.0.1", release.tag)
        self.assertEqual("morphir-config", release.crate)
        self.assertEqual("0.0.1", release.version)

    def test_rejects_a_version_that_differs_from_the_manifest(self) -> None:
        with self.assertRaisesRegex(crate_release.CrateReleaseError, "does not match"):
            self.resolve(
                "crates/morphir-config/v0.0.2",
                '[package]\nname = "morphir-config"\nversion = "0.0.1"\n',
            )

    def test_reads_a_version_inherited_from_the_workspace(self) -> None:
        release = self.resolve(
            "crates/morphir-config/v0.2.0",
            '[package]\nname = "morphir-config"\nversion.workspace = true\n',
        )
        self.assertEqual("0.2.0", release.version)

    def test_rejects_an_unknown_crate(self) -> None:
        with self.assertRaisesRegex(crate_release.CrateReleaseError, "no crate"):
            self.resolve(
                "crates/morphir-missing/v0.0.1",
                '[package]\nname = "morphir-config"\nversion = "0.0.1"\n',
            )

    def test_rejects_a_directory_whose_package_has_another_name(self) -> None:
        with self.assertRaisesRegex(crate_release.CrateReleaseError, "package name"):
            self.resolve(
                "crates/morphir-config/v0.0.1",
                '[package]\nname = "morphir-other"\nversion = "0.0.1"\n',
            )

    def test_rejects_a_crate_marked_not_publishable(self) -> None:
        with self.assertRaisesRegex(crate_release.CrateReleaseError, "publish = false"):
            self.resolve(
                "crates/morphir-config/v0.0.1",
                '[package]\nname = "morphir-config"\nversion = "0.0.1"\npublish = false\n',
            )

    def test_the_repository_morphir_config_manifest_matches_its_first_tag(self) -> None:
        release = crate_release.resolve_release(
            REPOSITORY_ROOT, "crates/morphir-config/v0.0.1"
        )
        self.assertEqual("morphir-config", release.crate)


class CommitOnMainTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        git(self.root, "init", "-q", "-b", "main")
        git(self.root, "config", "user.email", "release@example.invalid")
        git(self.root, "config", "user.name", "Release Test")
        git(self.root, "commit", "-q", "--allow-empty", "-m", "first")
        self.on_main = git(self.root, "rev-parse", "HEAD")
        git(self.root, "checkout", "-q", "-b", "side")
        git(self.root, "commit", "-q", "--allow-empty", "-m", "side")
        self.off_main = git(self.root, "rev-parse", "HEAD")

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_accepts_a_commit_reachable_from_main(self) -> None:
        crate_release.require_commit_on_main(self.root, self.on_main, "main")

    def test_rejects_a_commit_that_is_not_on_main(self) -> None:
        with self.assertRaisesRegex(crate_release.CrateReleaseError, "not on main"):
            crate_release.require_commit_on_main(self.root, self.off_main, "main")

    def test_rejects_a_malformed_commit(self) -> None:
        with self.assertRaisesRegex(crate_release.CrateReleaseError, "invalid"):
            crate_release.require_commit_on_main(self.root, "HEAD", "main")

    def test_reports_a_missing_main_ref(self) -> None:
        with self.assertRaisesRegex(crate_release.CrateReleaseError, "cannot check"):
            crate_release.require_commit_on_main(self.root, self.on_main, "origin/main")


class CommandLineTests(unittest.TestCase):
    def test_writes_workflow_outputs_for_a_valid_tag_on_main(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            write_repository(
                root, '[package]\nname = "morphir-config"\nversion = "0.0.1"\n'
            )
            git(root, "init", "-q", "-b", "main")
            git(root, "config", "user.email", "release@example.invalid")
            git(root, "config", "user.name", "Release Test")
            git(root, "add", ".")
            git(root, "commit", "-q", "-m", "first")
            commit = git(root, "rev-parse", "HEAD")
            output = root / "github-output"
            result = subprocess.run(
                [
                    sys.executable,
                    os.fspath(CRATE_SCRIPT),
                    "--tag", "crates/morphir-config/v0.0.1",
                    "--commit", commit,
                    "--main-ref", "main",
                    "--root", os.fspath(root),
                ],
                check=False,
                capture_output=True,
                text=True,
                env={**os.environ, "GITHUB_OUTPUT": os.fspath(output)},
            )
            self.assertEqual(0, result.returncode, result.stderr)
            self.assertEqual(
                "tag=crates/morphir-config/v0.0.1\n"
                "crate=morphir-config\n"
                "version=0.0.1\n"
                f"commit={commit}\n",
                output.read_text(encoding="utf-8"),
            )

    def test_fails_without_a_traceback_for_a_bad_tag(self) -> None:
        result = subprocess.run(
            [
                sys.executable,
                os.fspath(CRATE_SCRIPT),
                "--tag", "crates/morphir-config/v9.9.9",
                "--commit", COMMIT,
                "--root", os.fspath(REPOSITORY_ROOT),
            ],
            check=False,
            capture_output=True,
            text=True,
            env={key: value for key, value in os.environ.items() if key != "GITHUB_OUTPUT"},
        )
        self.assertEqual(2, result.returncode)
        self.assertIn("crate release error:", result.stderr)
        self.assertNotIn("Traceback", result.stderr)


class PublishWorkflowDefinitionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = PUBLISH_WORKFLOW.read_text(encoding="utf-8")
        cls.release_workflow = RELEASE_WORKFLOW.read_text(encoding="utf-8")

    def job(self, name: str) -> str:
        start = f"  {name}:\n"
        self.assertIn(start, self.workflow)
        body = self.workflow.split(start, 1)[1]
        return re.split(r"(?m)^  [a-z-]+:\n", body, maxsplit=1)[0]

    def test_triggers_on_crate_tags_and_manual_dispatch_of_an_existing_tag(self) -> None:
        self.assertEqual(["crates/*/v*"], push_tag_filters(self.workflow))
        self.assertIn("workflow_dispatch:\n    inputs:\n      tag:\n", self.workflow)
        self.assertIn("inputs.tag || github.ref_name", self.workflow)
        self.assertIn("format('refs/tags/{0}', inputs.tag)", self.workflow)

    def test_release_workflow_does_not_react_to_crate_tags(self) -> None:
        tag = "crates/morphir-config/v0.0.1"
        filters = push_tag_filters(self.release_workflow)
        self.assertEqual(["v*", "extension/*/v*"], filters)
        for pattern in filters:
            with self.subTest(pattern=pattern):
                self.assertFalse(github_filter_matches(pattern, tag))
        self.assertTrue(github_filter_matches("crates/*/v*", tag))
        self.assertFalse(github_filter_matches("crates/*/v*", "extension/avro/v0.2.0"))
        self.assertFalse(github_filter_matches("crates/*/v*", "v0.2.0"))

    def test_runs_one_publish_per_tag_without_cancelling(self) -> None:
        self.assertIn(
            "concurrency:\n  group: publish-crate-${{ inputs.tag || github.ref_name }}\n"
            "  cancel-in-progress: false\n",
            self.workflow,
        )

    def test_resolve_checks_the_peeled_tag_commit_against_main(self) -> None:
        resolve = self.job("resolve")
        self.assertIn('git rev-parse --verify "refs/tags/${RELEASE_TAG}^{commit}"', resolve)
        self.assertIn("refs/heads/main:refs/remotes/origin/main", resolve)
        self.assertIn("python3 .github/scripts/crate_release.py", resolve)
        self.assertIn("--main-ref origin/main", resolve)

    def test_publish_dry_runs_then_publishes_the_resolved_commit(self) -> None:
        publish = self.job("publish")
        self.assertIn("needs: resolve", publish)
        self.assertIn("permissions:\n      contents: read\n", publish)
        self.assertIn("ref: ${{ needs.resolve.outputs.commit }}", publish)
        self.assertIn("uses: jdx/mise-action@v4", publish)
        dry_run = publish.index('cargo publish --dry-run -p "$CRATE" --locked')
        real_run = publish.index('cargo publish -p "$CRATE" --locked')
        self.assertLess(dry_run, real_run)
        self.assertIn("CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}", publish)
        self.assertEqual(1, self.workflow.count("secrets.CARGO_REGISTRY_TOKEN"))

    def test_workflow_defaults_to_read_only_permissions(self) -> None:
        self.assertIn("\npermissions:\n  contents: read\n", self.workflow)
        self.assertNotIn("contents: write", self.workflow)
        self.assertNotIn("id-token: write", self.workflow)

    def test_explains_the_token_and_the_trusted_publishing_follow_up(self) -> None:
        header = self.workflow.split("\nname:", 1)[0]
        self.assertIn("CARGO_REGISTRY_TOKEN", header)
        self.assertIn("rust-lang/crates-io-auth-action", header)


class PublishWorkflowImpactTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.impact = config.load_config(REPOSITORY_ROOT / ".github" / "ci-impact.toml")
        cls.extensions = classify.load_extensions(REPOSITORY_ROOT / ".github" / "extensions.toml")
        cls.workspace = graph.workspace_from_metadata(fake_metadata())

    def test_publish_workflow_and_script_run_the_release_workflow_tests(self) -> None:
        for path in (".github/workflows/publish-crate.yml", ".github/scripts/crate_release.py"):
            with self.subTest(path=path):
                plan = classify.plan_changes([path], self.impact, self.workspace, self.extensions)
                self.assertFalse(plan.all, plan)
                self.assertEqual((), plan.crates)
                self.assertTrue(plan.jobs["test-release-workflow"])

    def test_publish_workflow_is_linted_as_yaml(self) -> None:
        plan = classify.plan_changes(
            [".github/workflows/publish-crate.yml"], self.impact, self.workspace, self.extensions
        )
        self.assertTrue(plan.jobs["lint-yaml"])


if __name__ == "__main__":
    unittest.main()
