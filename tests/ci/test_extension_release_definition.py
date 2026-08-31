"""Tests for the extension release workflow definition."""

from extension_release_test_support import *

class ReleaseWorkflowDefinitionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.release_workflow = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        cls.ci_workflow = CI_WORKFLOW.read_text(encoding="utf-8")

    def job(self, name: str, next_name: str | None) -> str:
        start = f"  {name}:\n"
        self.assertIn(start, self.release_workflow)
        body = self.release_workflow.split(start, 1)[1]
        if next_name is not None:
            body = body.split(f"  {next_name}:\n", 1)[0]
        return body

    def run_script_for_step(self, name: str, root: Path, *, fail_refresh: bool):
        """Execute one workflow shell step with deterministic fake git and gh CLIs."""
        marker = f"      - name: {name}\n"
        section = self.release_workflow.split(marker, 1)[1]
        section = section.split("\n      - name: ", 1)[0]
        script = textwrap.dedent(section.split("        run: |\n", 1)[1])
        fake_bin = root / "bin"
        fake_bin.mkdir()
        log = root / "gh.log"
        state = root / "release-state"
        state.write_text("missing\n", encoding="utf-8")
        git = fake_bin / "git"
        git.write_text(
            textwrap.dedent(
                f"""\
                #!/bin/sh
                set -eu
                case "$1" in
                  fetch) exit 0 ;;
                  rev-parse) printf '%s\\n' '{RELEASE_COMMIT}' ;;
                  *) exit 64 ;;
                esac
                """
            ),
            encoding="utf-8",
        )
        git.chmod(0o755)
        jq = fake_bin / "jq"
        jq.write_text(
            "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$RELEASE_TAG\"\n",
            encoding="utf-8",
        )
        jq.chmod(0o755)
        gh = fake_bin / "gh"
        gh.write_text(
            textwrap.dedent(
                """\
                #!/bin/sh
                set -eu
                printf '%s\\n' "$*" >> "$FAKE_GH_LOG"
                if [ "$1" = "api" ]; then
                  case "$2" in
                    */releases/tags/*)
                      if [ "$(cat "$FAKE_RELEASE_STATE")" = "missing" ]; then
                        printf '%s\\n' 'gh: Not Found (HTTP 404)' >&2
                        exit 1
                      fi
                      if [ "$FAKE_FAIL_REFRESH" = "1" ]; then
                        printf '%s\\n' 'gh: API unavailable (HTTP 503)' >&2
                        exit 1
                      fi
                      printf '%s\\n' '123'
                      exit 0
                      ;;
                    */releases/123/assets)
                      exit 0
                      ;;
                    *) exit 65 ;;
                  esac
                fi
                if [ "$1" = "release" ] && [ "$2" = "create" ]; then
                  printf '%s\\n' 'created' > "$FAKE_RELEASE_STATE"
                  exit 0
                fi
                if [ "$1" = "release" ] && [ "$2" = "upload" ]; then
                  exit 0
                fi
                exit 66
                """
            ),
            encoding="utf-8",
        )
        gh.chmod(0o755)
        runner_temp = root / "runner"
        manifest_dir = runner_temp / "extension-release"
        manifest_dir.mkdir(parents=True)
        asset = root / "asset.wasm"
        asset.write_bytes(b"exact asset bytes")
        (manifest_dir / "assets.txt").write_text(
            f"{asset}\n", encoding="utf-8"
        )
        result = subprocess.run(
            ["bash", "-eu", "-o", "pipefail", "-c", script],
            check=False,
            capture_output=True,
            text=True,
            env={
                **os.environ,
                "PATH": f"{fake_bin}{os.pathsep}{os.environ['PATH']}",
                "FAKE_GH_LOG": str(log),
                "FAKE_RELEASE_STATE": str(state),
                "FAKE_FAIL_REFRESH": "1" if fail_refresh else "0",
                "GITHUB_REPOSITORY": "finos/morphir-rust",
                "GH_TOKEN": "test-token",
                "RELEASE_TAG": "v0.2.0",
                "EXPECTED_COMMIT": RELEASE_COMMIT,
                "RUNNER_TEMP": str(runner_temp),
            },
        )
        return result, log.read_text(encoding="utf-8")

    def test_triggers_only_supported_tags_and_manual_existing_tag_dispatch(self) -> None:
        self.assertIn('      - "v*"', self.release_workflow)
        self.assertIn('      - "extension/*/v*"', self.release_workflow)
        self.assertIn("workflow_dispatch:", self.release_workflow)
        self.assertIn("tag:\n        description: Existing release tag", self.release_workflow)
        self.assertIn("required: true", self.release_workflow)
        self.assertIn("inputs.tag || github.ref_name", self.release_workflow)
        self.assertIn('refs/tags/${RELEASE_TAG}^{commit}', self.release_workflow)
        self.assertNotIn("git tag ", self.release_workflow)

    def test_manual_dispatch_checkout_uses_fully_qualified_tag_ref(self) -> None:
        release_info = self.job("release-info", "create-extension-artifacts")
        self.assertIn("format('refs/tags/{0}', inputs.tag)", release_info)
        self.assertNotIn("ref: ${{ inputs.tag || github.ref }}", release_info)

    def test_fully_qualified_tag_ref_wins_over_same_named_branch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary).resolve()
            subprocess.run(["git", "init", "-q", str(repository)], check=True)
            subprocess.run(
                ["git", "config", "user.email", "release@example.invalid"],
                cwd=repository,
                check=True,
            )
            subprocess.run(
                ["git", "config", "user.name", "Release Test"],
                cwd=repository,
                check=True,
            )
            marker = repository / "marker"
            marker.write_text("tag\n", encoding="utf-8")
            subprocess.run(["git", "add", "marker"], cwd=repository, check=True)
            subprocess.run(["git", "commit", "-qm", "tag target"], cwd=repository, check=True)
            tag_commit = subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=repository, text=True
            ).strip()
            subprocess.run(["git", "tag", "collision"], cwd=repository, check=True)
            marker.write_text("branch\n", encoding="utf-8")
            subprocess.run(["git", "commit", "-qam", "branch target"], cwd=repository, check=True)
            branch_commit = subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=repository, text=True
            ).strip()
            subprocess.run(["git", "branch", "collision"], cwd=repository, check=True)

            peeled = subprocess.check_output(
                ["git", "rev-parse", "refs/tags/collision^{commit}"],
                cwd=repository,
                text=True,
            ).strip()

            self.assertEqual(tag_commit, peeled)
            self.assertNotEqual(branch_commit, peeled)

    def test_serializes_each_tag_without_cancelling_an_active_publication(self) -> None:
        before_jobs = self.release_workflow.split("jobs:\n", 1)[0]
        self.assertIn("concurrency:", before_jobs)
        self.assertIn(
            "group: release-${{ inputs.tag || github.ref_name }}", before_jobs
        )
        self.assertIn("cancel-in-progress: false", before_jobs)

    def test_release_info_peels_once_and_downstream_jobs_checkout_commit(self) -> None:
        release_info = self.job("release-info", "create-extension-artifacts")
        create = self.job("create-extension-artifacts", "publish-extensions")
        publish = self.job("publish-extensions", None)
        self.assertIn("RELEASE_COMMIT=\"$(", release_info)
        self.assertIn('git rev-parse --verify "$tag_ref"', release_info)
        self.assertIn('--commit "$RELEASE_COMMIT"', release_info)
        self.assertIn("commit: ${{ steps.release.outputs.commit }}", release_info)
        self.assertIn("ref: ${{ needs.release-info.outputs.commit }}", create)
        self.assertIn("ref: ${{ needs.release-info.outputs.commit }}", publish)
        self.assertNotIn("ref: ${{ needs.release-info.outputs.tag }}", create)
        self.assertNotIn("ref: ${{ needs.release-info.outputs.tag }}", publish)
        self.assertIn('--commit "$RELEASE_COMMIT"', create)
        self.assertIn('--commit "$RELEASE_COMMIT"', publish)

    def test_publish_rechecks_remote_tag_commit_before_any_mutation(self) -> None:
        publish = self.job("publish-extensions", None)
        self.assertIn("verify_remote_tag_commit", publish)
        self.assertIn('EXPECTED_COMMIT: ${{ needs.release-info.outputs.commit }}', publish)
        self.assertIn('git fetch --force --no-tags origin "refs/tags/$RELEASE_TAG"', publish)
        self.assertIn('tag moved from $EXPECTED_COMMIT to $actual_commit', publish)
        mutation = min(
            publish.index("gh release create"),
            publish.index("gh release upload"),
        )
        self.assertLess(publish.index("verify_remote_tag_commit"), mutation)

    def test_creation_jobs_have_read_only_contents_permissions(self) -> None:
        self.assertIn("permissions:\n  contents: read", self.release_workflow)
        release_info = self.job("release-info", "create-extension-artifacts")
        create = self.job("create-extension-artifacts", "publish-extensions")
        self.assertIn("permissions:\n      contents: read", release_info)
        self.assertIn("permissions:\n      contents: read", create)
        self.assertNotIn("contents: write", release_info)
        self.assertNotIn("contents: write", create)

    def test_creation_job_disables_python_bytecode_writes(self) -> None:
        create = self.job("create-extension-artifacts", "publish-extensions")
        self.assertIn(
            '    env:\n      PYTHONDONTWRITEBYTECODE: "1"\n    strategy:',
            create,
        )
        self.assertEqual(
            1, self.release_workflow.count('PYTHONDONTWRITEBYTECODE: "1"')
        )

    def test_creation_job_builds_validates_and_uploads_seven_day_artifact(self) -> None:
        create = self.job("create-extension-artifacts", "publish-extensions")
        self.assertIn("mise run \"extension:artifact:${SHORT_ID}\"", create)
        self.assertIn("--validate-short-id \"$SHORT_ID\"", create)
        self.assertIn("uses: actions/upload-artifact@v7", create)
        self.assertIn("name: extension-${{ matrix.short_id }}", create)
        self.assertIn("retention-days: 7", create)
        self.assertIn("if-no-files-found: error", create)

    def test_publish_job_downloads_exact_artifacts_and_never_builds(self) -> None:
        publish = self.job("publish-extensions", None)
        self.assertIn(
            "needs: [release-info, create-extension-artifacts]", publish
        )
        self.assertIn("permissions:\n      contents: write", publish)
        self.assertIn("uses: actions/download-artifact@v7", publish)
        self.assertIn("pattern: extension-*", publish)
        self.assertIn("select_extension_assets.py", publish)
        self.assertNotIn("cargo build", publish)
        self.assertNotIn("cargo test", publish)
        self.assertNotIn("mise run extension:artifact", publish)

    def test_publish_job_checks_existing_bytes_and_never_clobbers(self) -> None:
        publish = self.job("publish-extensions", None)
        self.assertIn("gh release download", publish)
        self.assertIn("--existing-assets", publish)
        self.assertIn("--prepared-assets", publish)
        self.assertIn("gh release create", publish)
        self.assertIn("gh release upload", publish)
        self.assertNotIn("--clobber", publish)
        self.assertIn('${RUNNER_TEMP}/extension-release/upload', publish)
        self.assertIn('${RUNNER_TEMP}/extension-release/assets.txt', publish)
        self.assertNotIn("--prepared-assets .release/", publish)
        self.assertLess(
            publish.index("select_extension_assets.py"),
            publish.index("gh release create"),
        )

    def test_empty_existing_release_is_a_valid_state(self) -> None:
        publish = self.job("publish-extensions", None)
        self.assertIn("gh api", publish)
        self.assertIn("--jq '.[].name'", publish)
        self.assertIn('if [ -s "$asset_list" ]; then', publish)
        self.assertIn("gh release download", publish)
        self.assertIn("(HTTP 404)", publish)
        self.assertIn("cat \"$release_error\" >&2", publish)
        self.assertLess(
            publish.index('if [ -s "$asset_list" ]; then'),
            publish.index("gh release download"),
        )

    def test_all_asset_lookups_use_paginated_release_id_endpoint(self) -> None:
        publish = self.job("publish-extensions", None)
        self.assertIn('release_id="$(cat', publish)
        self.assertIn('releases/${release_id}/assets', publish)
        self.assertGreaterEqual(publish.count("--paginate"), 2)
        self.assertNotIn("--jq '.assets | length'", publish)
        self.assertNotIn(".assets[] | select", publish)

    def test_final_check_compares_every_expected_asset_and_rechecks_tag(self) -> None:
        publish = self.job("publish-extensions", None)
        self.assertIn("--expected-manifest", publish)
        self.assertIn("Final verification of every expected asset", publish)
        final = publish.split("      - name: Final verification of every expected asset\n", 1)[1]
        self.assertIn("verify_remote_tag_commit", final)
        self.assertIn("--require-all-existing", final)
        self.assertIn("missing or duplicated", final)
        self.assertIn("extension-release/expected-assets.txt", final)

    def test_mutation_rechecks_release_and_retries_upload_without_clobber(self) -> None:
        publish = self.job("publish-extensions", None)
        mutation = publish.split(
            "      - name: Create release and upload only new assets\n", 1
        )[1]
        self.assertIn("refresh_release_state", mutation)
        self.assertIn("verify_remote_tag_commit\n            gh release create", mutation)
        self.assertIn("verify_remote_tag_commit\n              if ! gh release upload", mutation)
        self.assertIn("compare_published_asset", mutation)
        self.assertIn("upload raced with a different asset", mutation)
        self.assertGreaterEqual(mutation.count("verify_remote_tag_commit"), 3)
        self.assertIn("compare_status=$?", mutation)
        self.assertIn("cannot inspect published asset", mutation)
        self.assertIn("return 2", mutation)
        self.assertIn("case \"$compare_status\" in", mutation)
        self.assertNotIn("--clobber", mutation)

    def test_fresh_release_refreshes_id_then_looks_up_and_uploads_asset(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            result, log = self.run_script_for_step(
                "Create release and upload only new assets",
                Path(temporary).resolve(),
                fail_refresh=False,
            )

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(2, log.count("releases/tags/v0.2.0"), log)
        self.assertIn("release create v0.2.0", log)
        self.assertIn("releases/123/assets --paginate", log)
        self.assertIn("release upload v0.2.0", log)

    def test_fresh_release_stops_when_post_create_state_refresh_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            result, log = self.run_script_for_step(
                "Create release and upload only new assets",
                Path(temporary).resolve(),
                fail_refresh=True,
            )

        self.assertNotEqual(0, result.returncode)
        self.assertIn("API unavailable", result.stderr)
        self.assertIn("release create v0.2.0", log)
        self.assertNotIn("releases/123/assets", log)
        self.assertNotIn("release upload", log)

    def test_ci_python_workflow_tests_are_read_only(self) -> None:
        self.assertIn("permissions:\n  contents: read", self.ci_workflow)
        self.assertIn("test-release-workflow:", self.ci_workflow)
        ci_job = self.ci_workflow.split("  test-release-workflow:\n", 1)[1]
        self.assertIn("permissions:\n      contents: read", ci_job)
        self.assertIn("python3 -m unittest discover -s tests/ci -v", ci_job)
        self.assertNotIn("contents: write", ci_job)
        self.assertNotIn("secrets.", ci_job)
