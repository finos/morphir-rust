"""Exercise extension selection before the released CLI is downloaded."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]


class CliReleaseTaskTests(unittest.TestCase):
    def test_gleam_selection_reaches_bundle_validation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            bundle = Path(temporary_directory) / "missing-bundle"
            result = subprocess.run(
                [str(REPOSITORY_ROOT / ".mise/tasks/test/cli-release"), "gleam"],
                cwd=temporary_directory,
                env={**os.environ, "MORPHIR_EXTENSION_BUNDLE": str(bundle)},
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(1, result.returncode, result.stderr)
            self.assertIn(f"no bundle at {bundle}", result.stderr)


if __name__ == "__main__":
    unittest.main()
