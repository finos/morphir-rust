"""Tests for the native provider snapshot release workflow and its tooling."""

from __future__ import annotations

import io
import json
from pathlib import Path
import re
import tarfile
import tempfile
import unittest

from ci_impact_test_support import REPOSITORY_ROOT  # also puts .github/scripts on sys.path
from test_crate_release import github_filter_matches, push_tag_filters

import provider_snapshot_release as release  # noqa: E402

WORKFLOWS = REPOSITORY_ROOT / ".github" / "workflows"
PROVIDER_WORKFLOW = WORKFLOWS / "release-provider-snapshot.yml"
FEATURE = (
    REPOSITORY_ROOT / "crates" / "morphir-package" / "tests" / "features" / "provider_release.feature"
)
COMMIT = "0123456789abcdef0123456789abcdef01234567"
TAG = "provider-snapshot/v0.2.0"
VERSION = "0.2.0"
LINUX = "x86_64-unknown-linux-gnu"
MACOS = "aarch64-apple-darwin"


def rustc_output(host: str) -> str:
    return (
        "rustc 1.98.1 (0123456789 2026-09-01)\n"
        "binary: rustc\n"
        "commit-hash: 0123456789abcdef\n"
        "commit-date: 2026-09-01\n"
        f"host: {host}\n"
        "release: 1.98.1\n"
        "LLVM version: 21.1.0\n"
    )


def write_evidence(directory: Path) -> Path:
    directory.mkdir()
    (directory / "capabilities.json").write_text(
        json.dumps({"service": "morphir-package/provider-snapshot/0.1.0-draft.1"}),
        encoding="utf-8",
    )
    for variant in release.VARIANTS:
        (directory / f"variant-{variant}.json").write_text(
            json.dumps({"variant": variant}), encoding="utf-8"
        )
    return directory


def write_built_executables(directory: Path, marker: str) -> Path:
    (directory / "examples").mkdir(parents=True)
    (directory / "morphir-provider-snapshot").write_bytes(f"service {marker}".encode())
    (directory / "examples" / "provider-snapshot-fixture").write_bytes(
        f"fixture {marker}".encode()
    )
    return directory


class ReleaseFixture:
    """Builds, describes and assembles fake executables for both targets."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.assets = root / "assets"
        self.assets.mkdir()

    def build_target(self, target: str) -> None:
        built = write_built_executables(self.root / f"built-{target}", target)
        staged = self.root / f"staged-{target}"
        release.package(built, VERSION, target, staged)
        rustc = self.root / f"rustc-{target}.txt"
        rustc.write_text(rustc_output(target), encoding="utf-8")
        evidence = write_evidence(self.root / f"evidence-{target}")
        release.describe(staged, VERSION, COMMIT, target, rustc, evidence, "20261001.1")
        for path in staged.iterdir():
            path.rename(self.assets / path.name)

    def publish(self) -> dict:
        for target in release.TARGETS:
            self.build_target(target)
        path = release.assemble(self.assets, TAG, VERSION, COMMIT)
        return json.loads(path.read_text(encoding="utf-8"))


class TagTests(unittest.TestCase):
    def test_accepts_the_current_package_version(self) -> None:
        version = release.package_version(REPOSITORY_ROOT)
        self.assertEqual(
            version, release.resolve_tag(REPOSITORY_ROOT, f"provider-snapshot/v{version}")
        )

    def test_rejects_other_versions_and_tag_families(self) -> None:
        for tag in (
            "provider-snapshot/v99.0.0",
            "v0.2.0",
            "crates/morphir-package/v0.2.0",
            "provider-snapshot/0.2.0",
        ):
            with self.subTest(tag=tag), self.assertRaises(release.ProviderReleaseError):
                release.resolve_tag(REPOSITORY_ROOT, tag)

    def test_tag_family_starts_only_the_provider_workflow(self) -> None:
        for workflow in WORKFLOWS.glob("*.yml"):
            text = workflow.read_text(encoding="utf-8")
            if "  push:\n    tags:\n" not in text:
                continue
            matched = any(github_filter_matches(f, TAG) for f in push_tag_filters(text))
            self.assertEqual(workflow == PROVIDER_WORKFLOW, matched, workflow.name)


class ArchiveTests(unittest.TestCase):
    def test_archives_are_byte_identical_and_contain_one_executable(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            executable = Path(directory) / "tool"
            executable.write_bytes(b"binary")
            first = release.deterministic_archive(executable, "tool")
            self.assertEqual(first, release.deterministic_archive(executable, "tool"))
            archive = Path(directory) / "tool.tgz"
            archive.write_bytes(first)
            self.assertEqual(b"binary", release.read_single_executable(archive, "tool"))

    def test_rejects_extra_members_and_wrong_names(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "tool.tgz"
            with tarfile.open(archive, "w:gz") as output:
                for name in ("tool", "extra"):
                    info = tarfile.TarInfo(name)
                    info.size, info.mode = 1, 0o755
                    output.addfile(info, io.BytesIO(b"x"))
            for expected in ("tool", "other"):
                with self.assertRaises(release.ProviderReleaseError):
                    release.read_single_executable(archive, expected)

    def test_service_and_test_fixture_never_share_an_archive(self) -> None:
        names = [release.archive_name(role, VERSION, LINUX) for role in release.ROLES]
        self.assertEqual(
            [
                "morphir-provider-snapshot-0.2.0-x86_64-unknown-linux-gnu.tgz",
                "morphir-provider-snapshot-test-fixture-0.2.0-x86_64-unknown-linux-gnu.tgz",
            ],
            names,
        )


class DescriptorTests(unittest.TestCase):
    def test_descriptor_records_every_artifact_with_provenance(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            descriptor = ReleaseFixture(Path(directory)).publish()
        self.assertEqual(COMMIT, descriptor["gitCommit"])
        self.assertEqual(TAG, descriptor["tag"])
        self.assertEqual(
            {(role.name, target) for role in release.ROLES for target in release.TARGETS},
            {(a["role"], a["target"]) for a in descriptor["artifacts"]},
        )
        for artifact in descriptor["artifacts"]:
            self.assertEqual("release", artifact["buildMode"])
            self.assertEqual(artifact["target"], artifact["rustc"]["host"])
            self.assertEqual("rustc 1.98.1 (0123456789 2026-09-01)", artifact["rustc"]["version"])
            self.assertEqual("1.98.1", artifact["rustc"]["release"])
            self.assertRegex(artifact["sha256"], r"^[0-9a-f]{64}$")
            self.assertRegex(artifact["executableSha256"], r"^[0-9a-f]{64}$")
        for target in release.TARGETS:
            self.assertEqual(list(release.VARIANTS), descriptor["qualifiedFixtureVariants"][target])

    def test_downloaded_assets_verify_for_each_target(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = ReleaseFixture(Path(directory))
            fixture.publish()
            for target in release.TARGETS:
                release.verify(fixture.assets, TAG, VERSION, COMMIT, target)
                output = Path(directory) / f"installed-{target}"
                release.extract(fixture.assets, VERSION, target, output)
                self.assertEqual(
                    f"service {target}".encode(),
                    (output / "morphir-provider-snapshot").read_bytes(),
                )

    def test_verification_rejects_tampering(self) -> None:
        def replace_archive(assets: Path) -> None:
            path = assets / release.archive_name(release.ROLES[0], VERSION, LINUX)
            path.write_bytes(path.read_bytes() + b"x")

        def replace_both(assets: Path) -> None:
            path = assets / release.archive_name(release.ROLES[0], VERSION, LINUX)
            other = Path(str(path) + ".tmp")
            other.write_bytes(b"bin")
            path.write_bytes(release.deterministic_archive(other, "morphir-provider-snapshot"))
            release.write_checksum(path)

        def change_descriptor(key: str, value: object):
            def mutate(assets: Path) -> None:
                path = assets / release.descriptor_name(VERSION)
                descriptor = json.loads(path.read_text(encoding="utf-8"))
                descriptor[key] = value
                path.write_text(json.dumps(descriptor), encoding="utf-8")

            return mutate

        for name, mutate in (
            ("archive bytes", replace_archive),
            ("archive and checksum", replace_both),
            ("commit", change_descriptor("gitCommit", "f" * 40)),
            ("tag", change_descriptor("tag", "provider-snapshot/v0.2.1")),
            ("variants", change_descriptor("qualifiedFixtureVariants", {})),
        ):
            with self.subTest(name), tempfile.TemporaryDirectory() as directory:
                fixture = ReleaseFixture(Path(directory))
                fixture.publish()
                mutate(fixture.assets)
                with self.assertRaises(release.ProviderReleaseError):
                    release.verify(fixture.assets, TAG, VERSION, COMMIT, LINUX)

    def test_describe_requires_every_signed_variant(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            built = write_built_executables(root / "built", "x")
            release.package(built, VERSION, LINUX, root / "assets")
            rustc = root / "rustc.txt"
            rustc.write_text(rustc_output(LINUX), encoding="utf-8")
            evidence = write_evidence(root / "evidence")
            (evidence / "variant-ambiguous-contexts.json").unlink()
            with self.assertRaises(release.ProviderReleaseError):
                release.describe(root / "assets", VERSION, COMMIT, LINUX, rustc, evidence, None)

    def test_describe_rejects_a_cross_compiler_host(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            release.package(write_built_executables(root / "b", "x"), VERSION, LINUX, root / "a")
            rustc = root / "rustc.txt"
            rustc.write_text(rustc_output(MACOS), encoding="utf-8")
            with self.assertRaises(release.ProviderReleaseError):
                release.describe(root / "a", VERSION, COMMIT, LINUX, rustc, write_evidence(root / "e"), None)

    def test_assemble_requires_every_target(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = ReleaseFixture(Path(directory))
            fixture.build_target(LINUX)
            with self.assertRaises(release.ProviderReleaseError):
                release.assemble(fixture.assets, TAG, VERSION, COMMIT)


class DefinitionTests(unittest.TestCase):
    def test_workflow_matrices_build_and_verify_exactly_the_qualified_targets(self) -> None:
        workflow = PROVIDER_WORKFLOW.read_text(encoding="utf-8")
        pairs = re.findall(r"- runner: (\S+)\n\s+target: (\S+)", workflow)
        expected = sorted((t.runner, t.triple) for t in release.TARGETS.values())
        self.assertEqual(sorted(expected * 2), sorted(pairs))

    def test_feature_examples_match_the_required_variants(self) -> None:
        feature = FEATURE.read_text(encoding="utf-8")
        table = feature.split("| variant            |", 1)[1].split("\n\n", 1)[0]
        self.assertEqual(list(release.VARIANTS), re.findall(r"\|\s+([a-z-]+)\s+\|", table))

    def test_release_acceptance_is_opt_in(self) -> None:
        manifest = (REPOSITORY_ROOT / "crates" / "morphir-package" / "Cargo.toml").read_text(
            encoding="utf-8"
        )
        self.assertIn(
            '[[test]]\nname = "provider_release_acceptance"\nharness = false\ntest = false\n',
            manifest,
        )

    def test_only_qualified_bytes_are_attested_and_uploaded(self) -> None:
        workflow = PROVIDER_WORKFLOW.read_text(encoding="utf-8")
        build = workflow.split("  build:\n", 1)[1].split("\n  publish:\n", 1)[0]
        qualify = build.index("--test provider_release_acceptance")
        self.assertLess(qualify, build.index("actions/attest-build-provenance@v4"))
        self.assertLess(qualify, build.index("actions/upload-artifact@v7"))
        self.assertNotIn("actions/cache", workflow)
        self.assertNotIn("rust-cache", workflow)
        self.assertNotIn("--clobber", workflow)


if __name__ == "__main__":
    unittest.main()
