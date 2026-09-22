"""Exercise extension selection and the transitional publish gate of the cli-release task."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
TASK = REPOSITORY_ROOT / ".mise/tasks/test/cli-release"

# A stub CLI stands in for the released one. `--version` always answers, `extension install`
# always exits 9 so that reaching it is visible, and the publish behaves however a test asks.
STUB = """#!/bin/sh
if [ "$1" = "--version" ]; then
    echo "morphir 0.0.0-stub"
    exit 0
fi
if [ "$2" = "publish" ] || [ "$3" = "publish" ]; then
{publish}
fi
if [ "$2" = "install" ]; then
    exit 9
fi
exit 0
"""

# What a released CLI says about a descriptor field it has never heard of.
#
# Reproduced with the CLI's real layout, not as one long line. miette hard-wraps the diagnostic to
# the terminal width and prefixes every continuation with a `│` gutter, which routinely splits
# `unknown field` from the field name it is about:
#
#     × Failed to publish extension release: invalid extension release bundle
#     │ at .../release.json: unknown field
#     │ `workspaceDiscovery`, expected one of `schemaVersion`, ...
#
# A single-line stub passes against a task that only flattens newlines, while the real CI job
# fails — which is exactly what happened. The wrapping is the part worth pinning.
UNKNOWN_FIELD_FAILURE = """    echo "Error:   × Failed to publish extension release: invalid extension release bundle" >&2
    echo "  │ at $PWD/release.json: unknown field" >&2
    echo "  │ \\`{field}\\`, expected one of \\`schemaVersion\\`, \\`shortId\\`," >&2
    echo "  │ \\`extensionId\\`, \\`package\\`, \\`version\\` at line 29 column 22" >&2
    exit 1"""


def _descriptor(**extra: object) -> str:
    fields = {
        "schemaVersion": 1,
        "shortId": "gleam",
        "extensionId": "morphir-gleam",
        "version": "0.1.0",
        "artifact": "morphir-gleam.tar.gz",
        "sha256": "0" * 64,
    }
    fields.update(extra)
    return json.dumps(fields, indent=2) + "\n"


class CliReleaseTaskTests(unittest.TestCase):
    def test_release_and_mck_checks_share_native_cli_acquisition(self) -> None:
        task = TASK.read_text()
        self.assertIn("scripts/released-cli.ts", task)
        self.assertNotIn("fetch_cli()", task)
        self.assertNotIn("sha256_of()", task)

    def test_gleam_selection_reaches_bundle_validation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            bundle = Path(temporary_directory) / "missing-bundle"
            result = _run_task("gleam", bundle=bundle, cwd=temporary_directory)
            self.assertEqual(1, result.returncode, result.stderr)
            self.assertIn(f"no bundle at {bundle}", result.stderr)


class TransitionalFieldTests(unittest.TestCase):
    """The gate degrades to a skip for a field no released CLI can parse, and for nothing else."""

    def test_the_transitional_list_documents_its_own_removal(self) -> None:
        task = TASK.read_text(encoding="utf-8")
        self.assertIn("TRANSITIONAL_FIELDS='workspaceDiscovery'", task)
        self.assertIn("deny_unknown_fields", task)
        self.assertIn("Delete this entry", task)

    def test_an_unparsable_transitional_field_skips_the_check(self) -> None:
        result = self._publish(
            UNKNOWN_FIELD_FAILURE.format(field="workspaceDiscovery"),
            descriptor=_descriptor(workspaceDiscovery=True),
        )
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("skipping the compatibility check", result.stderr)
        self.assertIn("workspaceDiscovery", result.stderr)
        self.assertIn("0.0.0-stub", result.stderr)
        self.assertIn(".config/morphir-cli-version", result.stderr)
        # The install, and everything after it, must not have run.
        self.assertNotIn("morphir extension install", result.stderr)
        # Diagnostics belong on stderr; this task's stdout is consumed as output.
        self.assertNotIn("skipping", result.stdout)

    def test_a_published_transitional_field_reports_the_entry_as_stale(self) -> None:
        result = self._publish(
            '    echo "published"\n    exit 0',
            descriptor=_descriptor(workspaceDiscovery=True),
        )
        # The stub exits 9 from `extension install`, so the task carried on past the publish.
        self.assertEqual(9, result.returncode, result.stderr)
        self.assertIn("is stale", result.stderr)
        self.assertIn("workspaceDiscovery", result.stderr)
        self.assertIn("published", result.stdout)
        self.assertNotIn("is stale", result.stdout)

    def test_a_descriptor_without_the_field_reports_nothing(self) -> None:
        result = self._publish('    exit 0', descriptor=_descriptor())
        self.assertEqual(9, result.returncode, result.stderr)
        self.assertNotIn("stale", result.stderr)

    def test_an_unrelated_publish_failure_still_fails(self) -> None:
        result = self._publish(
            '    echo "Failed to publish extension release: checksum mismatch" >&2\n    exit 3',
            descriptor=_descriptor(workspaceDiscovery=True),
        )
        self.assertEqual(3, result.returncode, result.stdout)
        self.assertIn("checksum mismatch", result.stderr)
        self.assertNotIn("skipping", result.stderr)

    def test_a_silent_publish_failure_still_fails(self) -> None:
        """A non-zero exit with output the task does not recognise is never skippable."""
        result = self._publish('    exit 4', descriptor=_descriptor(workspaceDiscovery=True))
        self.assertEqual(4, result.returncode, result.stdout)
        self.assertNotIn("skipping", result.stderr)

    def test_an_unknown_field_outside_the_list_still_fails(self) -> None:
        result = self._publish(
            UNKNOWN_FIELD_FAILURE.format(field="providerSynthesis"),
            descriptor=_descriptor(providerSynthesis=True),
        )
        self.assertEqual(1, result.returncode, result.stdout)
        self.assertIn("providerSynthesis", result.stderr)
        self.assertNotIn("skipping", result.stderr)

    def test_a_transitional_error_about_an_absent_field_still_fails(self) -> None:
        """The error naming the field is not enough; the descriptor has to carry it."""
        result = self._publish(
            UNKNOWN_FIELD_FAILURE.format(field="workspaceDiscovery"),
            descriptor=_descriptor(),
        )
        self.assertEqual(1, result.returncode, result.stdout)
        self.assertNotIn("skipping", result.stderr)

    def _publish(self, behaviour: str, descriptor: str) -> subprocess.CompletedProcess:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cli = root / "morphir"
            cli.write_text(STUB.format(publish=behaviour), encoding="utf-8")
            cli.chmod(0o755)
            bundle = root / "bundle"
            bundle.mkdir()
            (bundle / "morphir-gleam.release.json").write_text(descriptor, encoding="utf-8")
            (bundle / "morphir-gleam.tar.gz").write_text("artifact", encoding="utf-8")
            return _run_task("gleam", bundle=bundle, cwd=REPOSITORY_ROOT, cli=cli)


def _run_task(
    identifier: str,
    *,
    bundle: Path,
    cwd: Path | str,
    cli: Path | None = None,
) -> subprocess.CompletedProcess:
    environment = {**os.environ, "MORPHIR_EXTENSION_BUNDLE": str(bundle)}
    if cli is not None:
        environment["MORPHIR_CLI"] = str(cli)
    return subprocess.run(
        ["sh", str(TASK), identifier],
        cwd=str(cwd),
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )


if __name__ == "__main__":
    unittest.main()
