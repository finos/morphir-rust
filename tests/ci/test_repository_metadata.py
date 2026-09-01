import os
import subprocess
import unittest
from pathlib import Path


class RepositoryMetadataTest(unittest.TestCase):
    def test_local_beads_runtime_output_is_ignored(self):
        repository_root = Path(__file__).resolve().parents[2]
        git_environment = os.environ.copy()
        git_environment["GIT_CONFIG_GLOBAL"] = os.devnull
        positive_probes = (
            (".beads/backup/probe.jsonl", "backup/"),
            (".beads/embeddeddolt/probe", "embeddeddolt/"),
        )

        for probe, expected_pattern in positive_probes:
            with self.subTest(probe=probe):
                result = subprocess.run(
                    ["git", "check-ignore", "--no-index", "-v", probe],
                    cwd=repository_root,
                    capture_output=True,
                    text=True,
                    env=git_environment,
                )
                self.assertEqual(
                    result.returncode,
                    0,
                    msg=(
                        f"Expected {probe} to be ignored from "
                        f"{repository_root}; exit={result.returncode}, "
                        f"stdout={result.stdout!r}, stderr={result.stderr!r}"
                    ),
                )
                source_pattern, _ = result.stdout.strip().split("\t", 1)
                source, _, pattern = source_pattern.rsplit(":", 2)
                self.assertEqual(
                    Path(source.removeprefix("./")).as_posix(),
                    ".beads/.gitignore",
                    msg=f"Unexpected ignore source for {probe}: {result.stdout!r}",
                )
                self.assertEqual(
                    pattern,
                    expected_pattern,
                    msg=f"Unexpected ignore pattern for {probe}: {result.stdout!r}",
                )

        for probe in ("backup/probe.jsonl", "embeddeddolt/probe"):
            with self.subTest(probe=probe):
                result = subprocess.run(
                    ["git", "check-ignore", "--no-index", "-v", probe],
                    cwd=repository_root,
                    capture_output=True,
                    text=True,
                    env=git_environment,
                )
                self.assertEqual(
                    result.returncode,
                    1,
                    msg=(
                        f"Expected {probe} outside .beads to be unignored from "
                        f"{repository_root}; exit={result.returncode}, "
                        f"stdout={result.stdout!r}, stderr={result.stderr!r}"
                    ),
                )


if __name__ == "__main__":
    unittest.main()
