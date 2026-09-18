"""Tests for the CI impact config loader and glob matching."""

from __future__ import annotations

import unittest

from ci_impact_test_support import *


class ImpactConfigTests(unittest.TestCase):
    def test_parse_config_builds_frozen_rules(self) -> None:
        loaded = config.parse_config(fake_config_data())

        self.assertEqual(("Cargo.toml", "Cargo.lock", "mise.toml", ".github/workflows/ci.yml"), loaded.global_paths)
        self.assertEqual(frozenset({"README.md", "LICENSE"}), loaded.safe_exact)
        self.assertEqual((".beads/", "docs/"), loaded.safe_prefixes)
        self.assertEqual(frozenset({"morphir-daemon"}), loaded.extension_crates)
        names = [job.name for job in loaded.jobs]
        self.assertEqual(["kit-conformance", "test-extism", "lint-shell", "docs-generated"], names)
        kit = loaded.jobs[0]
        self.assertEqual(frozenset({"morphir-projection"}), kit.crates)
        self.assertEqual((".config/mck-driver-version",), kit.paths)
        self.assertEqual(frozenset(), loaded.jobs[2].crates)

    def test_parse_config_rejects_unknown_job_keys(self) -> None:
        data = fake_config_data()
        data["jobs"]["kit-conformance"]["crate"] = ["typo"]
        with self.assertRaises(ValueError):
            config.parse_config(data)

    def test_parse_config_rejects_non_string_entries(self) -> None:
        data = fake_config_data()
        data["global"]["paths"] = ["Cargo.toml", 7]
        with self.assertRaises(ValueError):
            config.parse_config(data)

    def test_load_config_reads_the_repository_file(self) -> None:
        loaded = config.load_config(REPOSITORY_ROOT / ".github" / "ci-impact.toml")
        self.assertIn("Cargo.lock", loaded.global_paths)
        self.assertIn(".github/ci-impact.toml", loaded.global_paths)
        self.assertIn(".github/scripts/classify_ci_changes.py", loaded.global_paths)
        self.assertIn("docs/", loaded.safe_prefixes)
        self.assertIn("kit-conformance", [job.name for job in loaded.jobs])

    def test_matches_pattern(self) -> None:
        cases = [
            ("Cargo.toml", "Cargo.toml", True),
            ("Cargo.toml", "crates/x/Cargo.toml", False),
            ("rust-toolchain*", "rust-toolchain.toml", True),
            (".cargo/**", ".cargo/config.toml", True),
            (".cargo/**", ".cargo/nested/deep.toml", True),
            ("**/*.sh", "scripts/run.sh", True),
            ("**/*.sh", "run.sh", True),
            ("**/*.sh", "scripts/run.py", False),
            (".mise/tasks/**", ".mise/tasks/test/unit", True),
            ("docs/**", "docs", False),
            ("crates/*/Cargo.toml", "crates/morphir-core/Cargo.toml", True),
            ("crates/*/Cargo.toml", "crates/morphir-core/src/Cargo.toml", False),
        ]
        for pattern, path, expected in cases:
            with self.subTest(pattern=pattern, path=path):
                self.assertEqual(expected, config.matches_pattern(pattern, path))


if __name__ == "__main__":
    unittest.main()
