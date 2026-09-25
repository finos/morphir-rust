//! `run_program` keeps a developer's `MORPHIR_` variables away from the program under test.
//!
//! This test changes the process environment, so it lives in its own test binary: no other test
//! runs in this process while it sets and removes the variable.

#![cfg(unix)]

use std::path::PathBuf;

use morphir_bdd::steps::cli::{CliProgram, run_program};

#[tokio::test]
async fn run_program_removes_inherited_morphir_variables() {
    let dir = tempfile::tempdir().expect("create a temporary directory");
    let program = CliProgram {
        name: "morphir".to_owned(),
        path: PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/echo.sh"
        )),
    };

    // SAFETY: this binary holds only this test, and `#[tokio::test]` runs it on a current-thread
    // runtime, so no other thread reads the environment while it changes.
    unsafe { std::env::set_var("MORPHIR_BDD_LEAK_PROBE", "leaked") };
    let result = run_program(&program, &[], dir.path()).await;
    // SAFETY: as above.
    unsafe { std::env::remove_var("MORPHIR_BDD_LEAK_PROBE") };

    let output = result.expect("the fixture runs");
    assert!(
        output.stdout.contains("leak=\n"),
        "MORPHIR_BDD_LEAK_PROBE must not reach the program under test:\n{}",
        output.stdout
    );

    // C1: `MORPHIR_HOME` must point inside this scenario's own workspace, not at a developer's
    // real Morphir home, so the isolated run can never reach or pollute real registries.
    let expected_morphir_home = dir.path().join(".home").join(".morphir");
    let expected_line = format!("morphir_home={}\n", expected_morphir_home.display());
    assert!(
        output.stdout.contains(&expected_line),
        "MORPHIR_HOME must be {}:\n{}",
        expected_morphir_home.display(),
        output.stdout
    );
}
