//! Runs the `Suite` runner against small features written to temporary directories: tag
//! expressions, undefined steps, outline rows, parse errors, missing paths, empty runs, `.feature.md`
//! files, extension errors, `@wip` and diffs in reports.
//!
//! Every run here sets its tag expression explicitly (`.tags(…)` or `.clear_tags()`), so a
//! `MORPHIR_BDD_TAGS` in the developer's shell cannot change the result.

use morphir_bdd::Suite;
use morphir_gherkin::Tag;
use morphir_gherkin::extension::{Context, Effect, Extensions, Scope, TagExtension};

const FEATURE: &str = "Feature: S\n  @keep\n  Scenario: kept\n    Given the step library is linked\n  Scenario: dropped\n    Given a step that no library defines\n";

#[tokio::test]
async fn a_suite_writes_json_and_junit_and_honours_a_tag_expression() {
    let out = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("s.feature"), FEATURE).unwrap();
    let result = Suite::new("sample")
        .features(dir.path())
        .tags("@keep")
        .out_dir(out.path())
        .run()
        .await;
    assert_eq!((result.passed, result.failed), (1, 0));
    assert!(result.succeeded());
    let json = std::fs::read_to_string(&result.json).unwrap();
    assert!(json.contains("\"kept\""), "{json}");
    assert!(!json.contains("\"dropped\""), "{json}");
    let junit = std::fs::read_to_string(&result.junit).unwrap();
    assert!(
        junit.contains("<testsuite") && junit.contains("kept"),
        "{junit}"
    );
    assert_eq!(result.json, out.path().join("sample.json"));
}

/// R1: an undefined step must fail a run. With no tag expression narrowing the scenarios, the
/// suite selects both, and the `dropped` scenario's undefined step must fail it. The tag filter is
/// cleared explicitly so this does not depend on whether `MORPHIR_BDD_TAGS` is set in the
/// environment `cargo test` runs in.
#[tokio::test]
async fn an_undefined_step_fails_the_suite_with_no_tag_expression() {
    let out = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("s.feature"), FEATURE).unwrap();
    let result = Suite::new("sample-untagged")
        .features(dir.path())
        .clear_tags()
        .out_dir(out.path())
        .run()
        .await;
    assert!(result.failed >= 1, "{result:?}");
    assert!(!result.succeeded());
}

/// T10-c regression: a `Suite` must not let cucumber parse process arguments. `cargo test -- <name>`
/// (a libtest filter) leaves that filter in the test binary's real `std::env::args()`; before this
/// fix, `Suite::run` let cucumber's own `clap` parser read that argv, see an argument it didn't
/// define, and abort the process with exit code 2. This spawns the same compiled test binary as a
/// subprocess with an exact libtest filter — the same shape as `cargo test -p morphir-bdd --test
/// suite -- a_suite`, which is how the coordinator's repro reached `Suite::run` — and asserts the
/// process exits successfully instead of aborting on an unrecognized argument.
#[test]
fn a_suite_ignores_the_process_arguments_a_test_filter_adds() {
    let exe = std::env::current_exe().expect("this test binary's own path");
    let output = std::process::Command::new(exe)
        .args([
            "a_suite_writes_json_and_junit_and_honours_a_tag_expression",
            "--exact",
        ])
        .output()
        .expect("run this test binary as a subprocess with a libtest filter");
    assert!(
        output.status.success(),
        "a libtest filter argument must not reach cucumber's CLI parser\nstatus: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("test result: ok"),
        "expected the filtered test to run and pass, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// Writes `text` to `name` in a fresh temporary features directory, runs it through a `Suite`
/// with no tag expression and reports under another fresh directory, and returns the result with
/// the JSON and JUnit report texts. The directories are returned too, so they outlive the checks.
async fn run_one(
    name: &str,
    text: &str,
    suite: impl FnOnce(Suite) -> Suite,
) -> (
    morphir_bdd::SuiteResult,
    String,
    String,
    [tempfile::TempDir; 2],
) {
    let out = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(name), text).unwrap();
    let result = suite(
        Suite::new(name.replace('.', "-"))
            .features(dir.path())
            .clear_tags()
            .out_dir(out.path()),
    )
    .run()
    .await;
    let json = std::fs::read_to_string(&result.json).unwrap();
    let junit = std::fs::read_to_string(&result.junit).unwrap();
    (result, json, junit, [out, dir])
}

/// F1: a tag on one `Examples` block reaches only that block's rows. `@wip` on block 1 skips its
/// row, and block 2's rows still run; before, `@wip` reached every row and the run was empty.
#[tokio::test]
async fn a_wip_examples_block_skips_only_its_own_rows() {
    let text = "Feature: O\n  Scenario Outline: row <x>\n    Given the step library is linked\n\n    @wip\n    Examples: first\n      | x |\n      | 1 |\n\n    Examples: second\n      | x |\n      | 2 |\n      | 3 |\n";
    let (result, json, _, _dirs) = run_one("o.feature", text, |s| s).await;
    assert!(result.succeeded(), "{result:?}");
    assert_eq!(result.passed, 2, "{result:?}");
    assert!(json.contains("row 2") && json.contains("row 3"), "{json}");
    assert!(!json.contains("row 1"), "{json}");
}

/// F2: a document that does not read fails the run, and the reader's message and line reach the
/// reports, not only the file's path.
#[tokio::test]
async fn a_malformed_feature_md_fails_the_run_with_its_message() {
    let text = "# Feature: F\n\n## Scenario: S\n\n* Given a step\n  that goes on\n";
    let (result, json, junit, _dirs) = run_one("bad.feature.md", text, |s| s).await;
    assert!(!result.succeeded(), "{result:?}");
    assert!(result.errors >= 1, "{result:?}");
    for report in [&json, &junit] {
        assert!(report.contains("a step is one line"), "{report}");
        assert!(report.contains("bad.feature.md:6:"), "{report}");
    }
}

/// F3: a features path that does not exist is an error, not an empty green run.
#[tokio::test]
async fn a_missing_features_path_fails_the_run() {
    let out = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let result = Suite::new("missing")
        .features(dir.path().join("no-such-features"))
        .clear_tags()
        .out_dir(out.path())
        .run()
        .await;
    assert_eq!(result.errors, 1, "{result:?}");
    assert!(!result.succeeded(), "{result:?}");
}

/// F3: with no tag expression, a run that selects no scenario is an error. With a tag expression
/// that selects none, it is not: the filter asked for that.
#[tokio::test]
async fn an_empty_run_fails_only_without_a_tag_expression() {
    let text = "Feature: W\n  @wip\n  Scenario: only wip\n    Given the step library is linked\n";
    let (result, _, _, _dirs) = run_one("w.feature", text, |s| s).await;
    assert_eq!(result.errors, 1, "{result:?}");
    assert!(!result.succeeded(), "{result:?}");

    let (result, _, _, _dirs) = run_one("w.feature", text, |s| s.tags("@nothing")).await;
    assert!(result.succeeded(), "{result:?}");
}

/// A `.feature.md` document runs through `Suite` like a `.feature` one.
#[tokio::test]
async fn a_feature_md_runs_through_a_suite() {
    let text =
        "# Feature: M\n\n## Scenario: markdown scenario\n\n* Given the step library is linked\n";
    let (result, json, _, _dirs) = run_one("m.feature.md", text, |s| s).await;
    assert!(result.succeeded(), "{result:?}");
    assert_eq!(result.passed, 1, "{result:?}");
    assert!(json.contains("markdown scenario"), "{json}");
}

struct Refuse;
impl TagExtension for Refuse {
    fn namespace(&self) -> Option<&str> {
        Some("refuse")
    }
    fn apply(&self, _tag: &Tag, _scope: Scope, _ctx: &mut Context) -> Result<Effect, String> {
        Err("refused on purpose".to_owned())
    }
}

/// An extension error while building a scenario's context fails the run.
#[tokio::test]
async fn an_extension_error_fails_the_run() {
    let text =
        "Feature: E\n  @refuse:x\n  Scenario: refused\n    Given the step library is linked\n";
    let (result, _, _, _dirs) = run_one("e.feature", text, |s| {
        s.extensions(Extensions::new().with_tags(Refuse))
    })
    .await;
    assert!(result.errors >= 1, "{result:?}");
    assert!(!result.succeeded(), "{result:?}");
}

/// `standard_extensions()` skips a `@wip` scenario before it starts, even one whose step no
/// library defines, and the rest of the run still passes.
#[tokio::test]
async fn standard_extensions_skip_a_wip_scenario() {
    let text = "Feature: W\n  @wip\n  Scenario: not ready\n    Given a step that no library defines\n\n  Scenario: ready\n    Given the step library is linked\n";
    let (result, json, _, _dirs) = run_one("w.feature", text, |s| {
        s.extensions(morphir_bdd::standard_extensions())
    })
    .await;
    assert!(result.succeeded(), "{result:?}");
    assert_eq!((result.passed, result.failed), (1, 0), "{result:?}");
    assert!(json.contains("\"ready\""), "{json}");
    assert!(!json.contains("not ready"), "{json}");
}

/// A failing comparison step's message carries a unified diff, and the JSON report keeps it.
#[tokio::test]
async fn a_failing_comparison_puts_a_unified_diff_in_the_json_report() {
    let text = "Feature: D\n  Scenario: differs\n    Given the output:\n      \"\"\"\n      alpha\n      beta\n      \"\"\"\n    Then stdout should be:\n      \"\"\"\n      alpha\n      gamma\n      \"\"\"\n";
    let (result, json, _, _dirs) = run_one("d.feature", text, |s| s).await;
    assert_eq!(result.failed, 1, "{result:?}");
    assert!(!result.succeeded(), "{result:?}");
    for part in [
        "--- expected stdout",
        "+++ actual stdout",
        "@@ -1,2 +1,2 @@",
        "-gamma",
        "+beta",
    ] {
        assert!(json.contains(part), "missing {part:?} in {json}");
    }
}
