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
