//! Output steps over the last command's output.

use cucumber::gherkin::Step;
use cucumber::{given, then};

use crate::diff::unified_diff;
use crate::world::MorphirWorld;

/// The stdout, stderr and exit status of the last command a scenario ran.
///
/// Task 12's CLI steps fill this component when they run a command; this task's output steps
/// only read it. `Given the output:` also fills it, for testing the output steps on their own.
#[derive(Debug, Default, Clone)]
pub struct LastOutput {
    /// What the last command wrote to stdout.
    pub stdout: String,
    /// What the last command wrote to stderr.
    pub stderr: String,
    /// The last command's exit status, or `None` if it has not run yet.
    pub status: Option<i32>,
}

/// Returns the scenario's [`LastOutput`], panicking if no command has run yet.
fn last(world: &MorphirWorld) -> &LastOutput {
    world
        .context
        .get::<LastOutput>()
        .expect("no output yet: run a command first (`When I run \"…\"`)")
}

/// Reads a step's doc string, panicking if the step has none.
fn doc_string(step: &Step) -> &str {
    step.docstring
        .as_deref()
        .expect("this step takes a doc string")
}

/// `Given the output:` sets [`LastOutput::stdout`] to the doc string, leaving stderr empty and
/// the status unset. It lets a scenario test the other output steps without running a command.
#[given("the output:")]
fn the_output(world: &mut MorphirWorld, step: &Step) {
    world.context.insert(LastOutput {
        stdout: doc_string(step).to_owned(),
        ..LastOutput::default()
    });
}

/// `Then stdout should contain {string}` asserts that the last command's stdout contains `text`.
#[then(expr = "stdout should contain {string}")]
fn stdout_contains(world: &mut MorphirWorld, text: String) {
    let out = &last(world).stdout;
    assert!(
        out.contains(&text),
        "stdout does not contain {text:?}:\n{out}"
    );
}

/// `Then stderr should contain {string}` asserts that the last command's stderr contains `text`.
#[then(expr = "stderr should contain {string}")]
fn stderr_contains(world: &mut MorphirWorld, text: String) {
    let err = &last(world).stderr;
    assert!(
        err.contains(&text),
        "stderr does not contain {text:?}:\n{err}"
    );
}

/// `Then stdout should be:` asserts that the last command's stdout equals the doc string,
/// printing a unified diff on mismatch. Trailing newlines are not significant.
#[then("stdout should be:")]
fn stdout_is(world: &mut MorphirWorld, step: &Step) {
    let expected = doc_string(step);
    let actual = &last(world).stdout;
    if expected.trim_end() != actual.trim_end() {
        panic!(
            "stdout differs:\n{}",
            unified_diff(expected, actual, "expected stdout", "actual stdout")
        );
    }
}

/// `Then the JSON output at {string} should be:` parses the last command's stdout as JSON, reads
/// the value at the given JSON pointer, and compares it to the doc string, parsed as JSON too.
/// A mismatch prints a unified diff of the two values, pretty-printed.
#[then(expr = "the JSON output at {string} should be:")]
fn json_at(world: &mut MorphirWorld, pointer: String, step: &Step) {
    let actual: serde_json::Value =
        serde_json::from_str(&last(world).stdout).expect("stdout is JSON");
    let expected: serde_json::Value =
        serde_json::from_str(doc_string(step)).expect("the doc string is JSON");
    let found = actual
        .pointer(&pointer)
        .unwrap_or_else(|| panic!("no value at {pointer}"));
    if *found != expected {
        let pretty = |v: &serde_json::Value| {
            format!("{}\n", serde_json::to_string_pretty(v).expect("serialize"))
        };
        panic!(
            "JSON at {pointer} differs:\n{}",
            unified_diff(&pretty(&expected), &pretty(found), "expected", "actual")
        );
    }
}
