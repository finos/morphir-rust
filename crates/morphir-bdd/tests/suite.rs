//! Runs the `Suite` runner against small features written to temporary directories: tag
//! expressions, undefined steps, outline rows, parse errors, missing paths, empty runs, `.feature.md`
//! files, extension errors, `@wip` and diffs in reports.
//!
//! Every run here sets its tag expression explicitly (`.tags(…)` or `.clear_tags()`), so a
//! `MORPHIR_BDD_TAGS` in the developer's shell cannot change the result. Orchestration goes
//! through [`SuiteDriver`] (see AGENTS.md's "BDD Test Drivers" rule); a few tests that need raw
//! filesystem control (permissions) or a features path deliberately outside the driver's own
//! directory build that part by hand and still run the suite through the driver.

mod drivers;

use drivers::suite_driver::SuiteDriver;
use morphir_bdd::Suite;
use morphir_gherkin::Tag;
use morphir_gherkin::extension::{Context, Effect, Extensions, Scope, TagExtension};

const FEATURE: &str = "Feature: S\n  @keep\n  Scenario: kept\n    Given the step library is linked\n  Scenario: dropped\n    Given a step that no library defines\n";

#[tokio::test]
async fn a_suite_writes_json_and_junit_and_honours_a_tag_expression() {
    let mut driver = SuiteDriver::new();
    driver.given_a_feature("s.feature", FEATURE);
    let features = driver.features_dir().to_owned();
    driver
        .when_the_suite_runs_with_tags("sample", &features, "@keep")
        .await;
    assert_eq!(
        (driver.result().passed, driver.result().failed),
        (1, 0),
        "{:?}",
        driver.result()
    );
    driver.then_it_succeeds();
    driver.then_the_json_report_contains("\"kept\"");
    assert!(!driver.json().contains("\"dropped\""), "{}", driver.json());
    driver.then_the_junit_report_contains("<testsuite");
    driver.then_the_junit_report_contains("kept");
    assert_eq!(driver.result().json, driver.out_dir().join("sample.json"));
}

/// R1: an undefined step must fail a run. With no tag expression narrowing the scenarios, the
/// suite selects both, and the `dropped` scenario's undefined step must fail it. The tag filter is
/// cleared explicitly so this does not depend on whether `MORPHIR_BDD_TAGS` is set in the
/// environment `cargo test` runs in.
#[tokio::test]
async fn an_undefined_step_fails_the_suite_with_no_tag_expression() {
    let mut driver = SuiteDriver::new();
    driver.given_a_feature("s.feature", FEATURE);
    let features = driver.features_dir().to_owned();
    driver
        .when_the_suite_runs("sample-untagged", &features)
        .await;
    assert!(driver.result().failed >= 1, "{:?}", driver.result());
    assert!(!driver.result().succeeded());
}

/// T10-c regression: a `Suite` must not let cucumber parse process arguments. `cargo test -- <name>`
/// (a libtest filter) leaves that filter in the test binary's real `std::env::args()`; before this
/// fix, `Suite::run` let cucumber's own `clap` parser read that argv, see an argument it didn't
/// define, and abort the process with exit code 2. This spawns the same compiled test binary as a
/// subprocess with an exact libtest filter — the same shape as `cargo test -p morphir-bdd --test
/// suite -- a_suite`, which is how the coordinator's repro reached `Suite::run` — and asserts the
/// process exits successfully instead of aborting on an unrecognized argument. It runs the binary
/// as a subprocess rather than through `SuiteDriver`: the thing under test is this test binary's
/// own process arguments, which a driver call from inside that same process cannot exercise.
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

/// F1: a tag on one `Examples` block reaches only that block's rows. `@wip` on block 1 skips its
/// row, and block 2's rows still run; before, `@wip` reached every row and the run was empty.
#[tokio::test]
async fn a_wip_examples_block_skips_only_its_own_rows() {
    let text = "Feature: O\n  Scenario Outline: row <x>\n    Given the step library is linked\n\n    @wip\n    Examples: first\n      | x |\n      | 1 |\n\n    Examples: second\n      | x |\n      | 2 |\n      | 3 |\n";
    let mut driver = SuiteDriver::new();
    driver.given_a_feature("o.feature", text);
    let features = driver.features_dir().to_owned();
    driver.when_the_suite_runs("o-feature", &features).await;
    driver.then_it_succeeds();
    assert_eq!(driver.result().passed, 2, "{:?}", driver.result());
    driver.then_the_json_report_contains("row 2");
    driver.then_the_json_report_contains("row 3");
    assert!(!driver.json().contains("row 1"), "{}", driver.json());
}

/// F2: a document that does not read fails the run, and the reader's message and line reach the
/// reports, not only the file's path.
#[tokio::test]
async fn a_malformed_feature_md_fails_the_run_with_its_message() {
    let text = "# Feature: F\n\n## Scenario: S\n\n* Given a step\n  that goes on\n";
    let mut driver = SuiteDriver::new();
    driver.given_a_feature("bad.feature.md", text);
    let features = driver.features_dir().to_owned();
    driver
        .when_the_suite_runs("bad-feature-md", &features)
        .await;
    assert!(!driver.result().succeeded(), "{:?}", driver.result());
    assert!(driver.result().errors >= 1, "{:?}", driver.result());
    for report in [driver.json(), driver.junit()] {
        assert!(report.contains("a step is one line"), "{report}");
        assert!(report.contains("bad.feature.md:6:"), "{report}");
    }
}

/// F3: a features path that does not exist is an error, not an empty green run.
#[tokio::test]
async fn a_missing_features_path_fails_the_run() {
    // The missing path lives under its own temporary directory, unrelated to the driver's own
    // report output directory, since this test's point is that the path itself does not exist.
    let missing_parent = tempfile::tempdir().unwrap();
    let mut driver = SuiteDriver::new();
    driver
        .when_the_suite_runs("missing", missing_parent.path().join("no-such-features"))
        .await;
    assert_eq!(driver.result().errors, 1, "{:?}", driver.result());
    assert!(!driver.result().succeeded(), "{:?}", driver.result());
}

/// F3: with no tag expression, a run that selects no scenario is an error. With a tag expression
/// that selects none, it is not: the filter asked for that.
#[tokio::test]
async fn an_empty_run_fails_only_without_a_tag_expression() {
    let text = "Feature: W\n  @wip\n  Scenario: only wip\n    Given the step library is linked\n";

    let mut driver = SuiteDriver::new();
    driver.given_a_feature("w.feature", text);
    let features = driver.features_dir().to_owned();
    driver.when_the_suite_runs("w-feature", &features).await;
    assert_eq!(driver.result().errors, 1, "{:?}", driver.result());
    assert!(!driver.result().succeeded(), "{:?}", driver.result());

    let mut driver = SuiteDriver::new();
    driver.given_a_feature("w.feature", text);
    let features = driver.features_dir().to_owned();
    driver
        .when_the_suite_runs_with_tags("w-feature", &features, "@nothing")
        .await;
    driver.then_it_succeeds();
}

/// A `.feature.md` document runs through `Suite` like a `.feature` one.
#[tokio::test]
async fn a_feature_md_runs_through_a_suite() {
    let text =
        "# Feature: M\n\n## Scenario: markdown scenario\n\n* Given the step library is linked\n";
    let mut driver = SuiteDriver::new();
    driver.given_a_feature("m.feature.md", text);
    let features = driver.features_dir().to_owned();
    driver.when_the_suite_runs("m-feature-md", &features).await;
    driver.then_it_succeeds();
    assert_eq!(driver.result().passed, 1, "{:?}", driver.result());
    driver.then_the_json_report_contains("markdown scenario");
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
    let mut driver = SuiteDriver::new();
    driver.given_a_feature("e.feature", text);
    driver.given_extensions(Extensions::new().with_tags(Refuse));
    let features = driver.features_dir().to_owned();
    driver.when_the_suite_runs("e-feature", &features).await;
    assert!(driver.result().errors >= 1, "{:?}", driver.result());
    assert!(!driver.result().succeeded(), "{:?}", driver.result());
}

/// `standard_extensions()` skips a `@wip` scenario before it starts, even one whose step no
/// library defines, and the rest of the run still passes.
#[tokio::test]
async fn standard_extensions_skip_a_wip_scenario() {
    let text = "Feature: W\n  @wip\n  Scenario: not ready\n    Given a step that no library defines\n\n  Scenario: ready\n    Given the step library is linked\n";
    let mut driver = SuiteDriver::new();
    driver.given_a_feature("w.feature", text);
    driver.given_the_standard_extensions();
    let features = driver.features_dir().to_owned();
    driver.when_the_suite_runs("w-feature", &features).await;
    driver.then_it_succeeds();
    assert_eq!(
        (driver.result().passed, driver.result().failed),
        (1, 0),
        "{:?}",
        driver.result()
    );
    driver.then_the_json_report_contains("\"ready\"");
    assert!(!driver.json().contains("not ready"), "{}", driver.json());
}

/// A failing comparison step's message carries a unified diff, and the JSON report keeps it.
#[tokio::test]
async fn a_failing_comparison_puts_a_unified_diff_in_the_json_report() {
    let text = "Feature: D\n  Scenario: differs\n    Given the output:\n      \"\"\"\n      alpha\n      beta\n      \"\"\"\n    Then stdout should be:\n      \"\"\"\n      alpha\n      gamma\n      \"\"\"\n";
    let mut driver = SuiteDriver::new();
    driver.given_a_feature("d.feature", text);
    let features = driver.features_dir().to_owned();
    driver.when_the_suite_runs("d-feature", &features).await;
    assert_eq!(driver.result().failed, 1, "{:?}", driver.result());
    assert!(!driver.result().succeeded(), "{:?}", driver.result());
    for part in [
        "--- expected stdout",
        "+++ actual stdout",
        "@@ -1,2 +1,2 @@",
        "-gamma",
        "+beta",
    ] {
        driver.then_the_json_report_contains(part);
    }
}

/// C2: a nested directory that discovery cannot read must not be skipped in silence. A
/// `read_dir` failure on a subtree used to be dropped without a word, so a run could still pass
/// with part of its features quietly missing. This locks a nested directory with `chmod 000`,
/// leaving a sibling `.feature` file readable, and asserts the run reports an error for the
/// locked directory while still running the sibling's scenario. Root can read a directory with no
/// permission bits at all, so the test skips itself (after restoring permissions) when that turns
/// out to be true here, rather than asserting a Unix permission it cannot actually create. The
/// locked directory is created by hand, alongside the driver's own `given_a_feature` file, since
/// `SuiteDriver` has no domain method for corrupting a directory's permissions — that is
/// deliberately outside its given/when/then vocabulary.
#[cfg(unix)]
#[tokio::test]
async fn discovery_reports_a_directory_it_cannot_read() {
    use std::os::unix::fs::PermissionsExt;

    let mut driver = SuiteDriver::new();
    driver.given_a_feature(
        "ok.feature",
        "Feature: K\n  Scenario: k\n    Given the step library is linked\n",
    );
    let features = driver.features_dir().to_owned();
    let locked = features.join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::write(locked.join("s.feature"), FEATURE).unwrap();

    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_dir(&locked).is_ok() {
        // Running as root (or some other override that can read anyway): chmod 000 does not
        // actually block reading here, so there is nothing this test can exercise.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }

    driver.when_the_suite_runs("locked", &features).await;

    // Restore permissions before any assertion can fail, so the tempdir's own cleanup can still
    // remove `locked`.
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert!(driver.result().errors >= 1, "{:?}", driver.result());
    assert!(!driver.result().succeeded(), "{:?}", driver.result());
    assert_eq!(
        driver.result().passed,
        1,
        "the readable sibling still ran: {:?}",
        driver.result()
    );
}

/// `Suite::new` refuses a name that cannot safely be joined into a report file path.
#[test]
#[should_panic(expected = "invalid suite name")]
fn suite_new_refuses_a_name_with_a_parent_component() {
    Suite::new("../escape");
}

#[test]
#[should_panic(expected = "invalid suite name")]
fn suite_new_refuses_a_name_with_a_path_separator() {
    Suite::new("a/b");
}
