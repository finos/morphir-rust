"""Tests for the Avro extension artifact task."""

from package_extension_test_support import *

class AvroArtifactTaskTests(unittest.TestCase):
    def test_task_is_executable_and_runs_the_required_pipeline_in_order(self) -> None:
        mode = AVRO_ARTIFACT_TASK.stat().st_mode
        script = AVRO_ARTIFACT_TASK.read_text(encoding="utf-8")
        commands = [
            "cargo test --locked -p morphir-avro-extension",
            "cargo build --locked --release -p morphir-avro-extension --target wasm32-unknown-unknown",
            "cargo test --locked -p morphir-daemon --test installed_wasm_extension -- --ignored",
            "mise run test:avro-idl",
            "wasm-tools validate target/wasm32-unknown-unknown/release/morphir_avro_extension.wasm",
            "set -- scripts/package_extension.py",
        ]

        self.assertTrue(mode & stat.S_IXUSR)
        self.assertIn("set -eu", script)
        positions = [script.index(command) for command in commands]
        self.assertEqual(sorted(positions), positions)

    def test_task_clears_only_the_explicit_avro_staging_directory(self) -> None:
        script = AVRO_ARTIFACT_TASK.read_text(encoding="utf-8")

        self.assertIn(
            'STAGING_DIR="$REPO_ROOT/.morphir/build/extensions/avro"', script
        )
        self.assertNotIn("rm -rf", script)
        self.assertIn("--clean-extension-staging avro", script)
        self.assertNotIn(".morphir/build/extensions/*", script)

    def test_task_packages_without_tagging_releasing_or_publishing(self) -> None:
        script = AVRO_ARTIFACT_TASK.read_text(encoding="utf-8")

        self.assertIn('--output "$output"', script)
        self.assertIn('run_pipeline "$REPO_ROOT" "$STAGING_DIR" ""', script)
        self.assertIn("MORPHIR_AVRO_SNAPSHOT_MODE=1", script)
        self.assertIn('--transfer-bundle "$SNAPSHOT_BUNDLE"', script)
        self.assertNotIn("git tag", script)
        self.assertNotIn("gh release", script)
        self.assertNotIn("cargo publish", script)
        self.assertNotIn("mise run extension:artifact:avro", script)

    def test_symlinked_staging_ancestor_is_rejected_without_external_deletion(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory) / "repository"
            fixture = ArtifactTaskFixture(root)
            external = root.parent / "external"
            avro = external / "avro"
            avro.mkdir(parents=True)
            sentinel = avro / "survives.txt"
            sentinel.write_text("keep\n", encoding="utf-8")
            (root / ".morphir/build").mkdir(parents=True)
            (root / ".morphir/build/extensions").symlink_to(
                external, target_is_directory=True
            )

            result = fixture.run()

            self.assertNotEqual(0, result.returncode)
            self.assertIn("symbolic link", result.stderr)
            self.assertEqual("keep\n", sentinel.read_text(encoding="utf-8"))
            self.assertFalse(fixture.log.exists())

    @unittest.skipIf(os.name == "nt", "POSIX permission behavior")
    def test_staging_cleanup_io_error_has_no_traceback(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            staging = fixture.root / ".morphir/build/extensions/avro"
            staging.mkdir(parents=True)
            (staging / "blocked").write_text("bytes", encoding="utf-8")
            staging.chmod(0)
            try:
                result = fixture.run()
            finally:
                if staging.exists():
                    staging.chmod(0o700)

            self.assertNotEqual(0, result.returncode)
            self.assertIn("cannot clean avro staging directory", result.stderr)
            self.assertNotIn("Traceback", result.stderr)
            self.assertFalse(fixture.log.exists())

    def test_clean_tree_records_the_head_commit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")

            result = fixture.run()

            self.assertEqual(0, result.returncode, result.stderr)
            self.assertEqual(fixture.head(), fixture.descriptor()["gitCommit"])
            snapshot_source = fixture.snapshot_source()
            self.assertFalse(snapshot_source.is_relative_to(fixture.root))
            self.assertTrue(fixture.archived_task_path().is_relative_to(snapshot_source))
            self.assertTrue(
                all(line.startswith(f"{snapshot_source} | ") for line in fixture.log_lines())
            )
            self.assertFalse(snapshot_source.parent.exists())

    def test_ignored_source_absent_from_head_cannot_affect_provenance_build(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            ignored = fixture.root / "crates/morphir-avro-extension/ignored-source.rs"
            ignored.write_text("ambient-only\n", encoding="utf-8")
            self.assertEqual(
                "",
                fixture._git(
                    "status", "--porcelain", "--untracked-files=normal"
                ).stdout,
            )

            result = fixture.run()

            self.assertEqual(0, result.returncode, result.stderr)
            self.assertEqual(fixture.head(), fixture.descriptor()["gitCommit"])
            self.assertNotIn("ignored-source-present", fixture.log_lines())
            self.assertFalse(fixture.snapshot_source().parent.exists())

    def test_task_resolves_its_repository_when_called_from_outside(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")

            result = fixture.run(working_directory=fixture.root.parent)

            self.assertEqual(0, result.returncode, result.stderr)
            self.assertEqual(fixture.head(), fixture.descriptor()["gitCommit"])

    def test_dirty_tree_omits_the_head_commit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            (fixture.root / "tracked.txt").write_text("dirty\n", encoding="utf-8")

            result = fixture.run()

            self.assertEqual(0, result.returncode, result.stderr)
            self.assertNotIn("gitCommit", fixture.descriptor())

    def test_snapshot_mode_cannot_claim_an_ambient_worktree(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            (fixture.root / "tracked.txt").write_text("dirty\n", encoding="utf-8")

            result = fixture.run(
                {
                    "MORPHIR_AVRO_SNAPSHOT_MODE": "1",
                    "MORPHIR_AVRO_HEAD_COMMIT": fixture.head(),
                }
            )

            self.assertNotEqual(0, result.returncode)
            self.assertIn("snapshot source must not be inside a Git worktree", result.stderr)
            self.assertFalse(
                (fixture.root / ".morphir/build/extensions/avro/release.json").exists()
            )

    def test_plain_untracked_input_builds_locally_without_git_commit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            untracked = fixture.root / "crates/morphir-avro-extension/local-input.rs"
            untracked.write_text("local-only\n", encoding="utf-8")

            result = fixture.run()

            self.assertEqual(0, result.returncode, result.stderr)
            self.assertNotIn("gitCommit", fixture.descriptor())
            self.assertTrue(
                all(line.startswith(f"{fixture.root} | ") for line in fixture.log_lines())
            )
            self.assertFalse(fixture.snapshot_marker.exists())

    def test_clean_status_fails_when_head_lacks_a_packaging_input(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            fixture._git("rm", "--cached", "scripts/package_extension.py")
            with (fixture.root / ".gitignore").open("a", encoding="utf-8") as ignore:
                ignore.write("scripts/package_extension.py\n")
            fixture._git("add", ".gitignore")
            fixture._git(
                "-c", "commit.gpgsign=false", "commit", "-qm", "ignore packager"
            )
            self.assertEqual(
                "",
                fixture._git(
                    "status", "--porcelain", "--untracked-files=normal"
                ).stdout,
            )

            result = fixture.run()

            self.assertNotEqual(0, result.returncode)
            self.assertFalse(
                (fixture.root / ".morphir/build/extensions/avro/release.json").exists()
            )
            self.assertFalse(fixture.snapshot_source().parent.exists())

    def test_clean_status_fails_when_head_lacks_a_packaging_module(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            fixture._git("rm", "scripts/extension_packaging/bundles.py")
            fixture._git(
                "-c", "commit.gpgsign=false", "commit", "-qm", "remove packaging module"
            )

            result = fixture.run()

            self.assertNotEqual(0, result.returncode)
            self.assertIn("packaging module unavailable", result.stderr)
            self.assertNotIn("Traceback", result.stderr)
            self.assertFalse(
                (fixture.root / ".morphir/build/extensions/avro/release.json").exists()
            )

    def test_clean_status_fails_when_head_lacks_the_artifact_task(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            fixture._git("rm", "--cached", ".mise/tasks/extension/artifact/avro")
            with (fixture.root / ".gitignore").open("a", encoding="utf-8") as ignore:
                ignore.write(".mise/tasks/extension/artifact/avro\n")
            fixture._git("add", ".gitignore")
            fixture._git(
                "-c", "commit.gpgsign=false", "commit", "-qm", "ignore task"
            )

            result = fixture.run()

            self.assertNotEqual(0, result.returncode)
            self.assertFalse(
                (fixture.root / ".morphir/build/extensions/avro/release.json").exists()
            )

    def test_clean_status_fails_when_head_lacks_root_cargo_inputs(self) -> None:
        for missing in ("Cargo.toml", "Cargo.lock"):
            with (
                self.subTest(missing=missing),
                tempfile.TemporaryDirectory() as temporary_directory,
            ):
                fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
                fixture._git("rm", missing)
                fixture._git(
                    "-c", "commit.gpgsign=false", "commit", "-qm", "remove cargo input"
                )

                result = fixture.run()

                self.assertNotEqual(0, result.returncode)
                self.assertIn("required provenance input is missing", result.stderr)
                self.assertFalse(
                    (fixture.root / ".morphir/build/extensions/avro/release.json").exists()
                )

    def test_clean_status_fails_when_head_lacks_mise_inputs(self) -> None:
        for missing in ("mise.toml", ".mise/tasks/test/avro-idl"):
            with (
                self.subTest(missing=missing),
                tempfile.TemporaryDirectory() as temporary_directory,
            ):
                fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
                fixture._git("rm", missing)
                fixture._git(
                    "-c", "commit.gpgsign=false", "commit", "-qm", "remove mise input"
                )

                result = fixture.run()

                self.assertNotEqual(0, result.returncode)
                self.assertIn("required provenance input is missing", result.stderr)
                self.assertFalse(
                    (fixture.root / ".morphir/build/extensions/avro/release.json").exists()
                )

    def test_clean_status_fails_when_head_lacks_source_or_test_inputs(self) -> None:
        for missing in (
            "crates/morphir-avro-extension/src/lib.rs",
            "crates/morphir-avro-extension/tests/guest.rs",
            "tests/ci/test_package_extension.py",
            "tests/ci/package_extension_test_support.py",
            "tests/ci/test_package_extension_packaging.py",
            "tests/ci/test_package_extension_task.py",
            "crates/morphir-daemon/tests/support/mod.rs",
            "crates/morphir-daemon/tests/support/installed_wasm.rs",
        ):
            with (
                self.subTest(missing=missing),
                tempfile.TemporaryDirectory() as temporary_directory,
            ):
                fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
                fixture._git("rm", missing)
                fixture._git(
                    "-c", "commit.gpgsign=false", "commit", "-qm", "remove provenance input"
                )

                result = fixture.run()

                self.assertNotEqual(0, result.returncode)
                self.assertIn("required provenance input is missing", result.stderr)
                self.assertFalse(
                    (fixture.root / ".morphir/build/extensions/avro/release.json").exists()
                )

    def test_python_older_than_3_11_fails_before_build(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            fixture._write_tool(
                "python3",
                'if [ "${1:-}" = "-c" ]; then exit 1; fi\nexec "$REAL_PYTHON" "$@"\n',
            )
            result = fixture.run({"REAL_PYTHON": sys.executable})

            self.assertNotEqual(0, result.returncode)
            self.assertIn("Python 3.11 or newer is required", result.stderr)
            self.assertFalse(fixture.log.exists())

    def test_java_older_than_11_fails_before_build(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fixture = ArtifactTaskFixture(Path(temporary_directory) / "repository")
            fixture._write_tool("java", 'echo \'java version "1.8.0_402"\' >&2\n')

            result = fixture.run()

            self.assertNotEqual(0, result.returncode)
            self.assertIn("Java 11 or newer is required", result.stderr)
            self.assertFalse(fixture.log.exists())
