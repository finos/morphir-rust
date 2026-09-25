"""Exercise extension selection and the publish step of the cli-release task."""

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

def _descriptor(**extra: object) -> str:
    fields = {
        "schemaVersion": "2.0.0-draft.2",
        "shortId": "gleam",
        "extensionId": "morphir-gleam",
        "version": "0.1.0",
        "artifacts": [{"runtime": "wasm", "filename": "morphir-gleam.wasm",
                       "sha256": "0" * 64, "claims": {
                           "claimsVersion": "0.1.0-draft.2",
                           "protocolVersions": ["0.1"],
                           "extension": {"id": "morphir-gleam", "name": "Morphir Gleam",
                                         "version": "0.1.0", "types": ["backend"]},
                           "capabilities": {"backend": {"targets": ["gleam"],
                                                       "irVersions": ["3", "4"], "generate": True}},
                       }}],
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

    def test_elm_native_selection_reaches_bundle_validation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            bundle = Path(temporary_directory) / "missing-bundle"
            result = _run_task("elm-native", bundle=bundle, cwd=temporary_directory)
            self.assertEqual(1, result.returncode, result.stderr)
            self.assertIn(f"no bundle at {bundle}", result.stderr)

    def test_elm_native_compiles_a_multi_file_selection(self) -> None:
        """An installed multiDocument frontend takes a selection of several files in one compile."""
        task = TASK.read_text(encoding="utf-8")
        self.assertIn("elm-native) EXTENSION_ID=morphir-elm-native ;;", task)
        self.assertIn(
            "--input src/Acme/Types.elm --input src/Acme/Rules.elm",
            task,
        )


class PublishTests(unittest.TestCase):
    """Every publish failure fails the task; nothing is downgraded to a skip."""

    def test_the_task_has_no_transitional_skip(self) -> None:
        task = TASK.read_text(encoding="utf-8")
        self.assertNotIn("TRANSITIONAL_FIELDS", task)
        self.assertNotIn("skipping the compatibility check", task)

    def test_a_successful_publish_goes_on_to_install(self) -> None:
        result = self._publish(
            '    echo "published"\n    exit 0',
            descriptor=_descriptor(),
        )
        # The stub exits 9 from `extension install`, so the task carried on past the publish.
        self.assertEqual(9, result.returncode, result.stderr)
        self.assertIn("published", result.stdout)

    def test_a_publish_failure_fails_with_its_status(self) -> None:
        result = self._publish(
            '    echo "Failed to publish extension release: checksum mismatch" >&2\n    exit 3',
            descriptor=_descriptor(),
        )
        self.assertEqual(3, result.returncode, result.stdout)
        self.assertIn("checksum mismatch", result.stderr)
        self.assertNotIn("skipping", result.stderr)
        self.assertNotIn("morphir extension install", result.stderr)

    def test_an_unknown_field_error_still_fails(self) -> None:
        """A CLI that rejects a descriptor member fails the check; it is not a transition."""
        result = self._publish(
            '    echo "invalid extension release bundle: unknown field \\`workspaceDiscovery\\`" >&2\n'
            '    exit 1',
            descriptor=_descriptor(),
        )
        self.assertEqual(1, result.returncode, result.stdout)
        self.assertIn("unknown field", result.stderr)
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
