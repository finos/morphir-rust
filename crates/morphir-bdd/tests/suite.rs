//! Runs the `Suite` runner against a small feature: a tag expression keeps one scenario and drops
//! the other, and an undefined step fails the suite once no tag expression protects it.

use morphir_bdd::suite::Suite;

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
