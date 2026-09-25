//! Output steps over the last command's output.

use cucumber::gherkin::Step;
use cucumber::{given, then};

use crate::diff::{same_text, unified_diff};
use crate::world::MorphirWorld;

/// Caps how much raw text a panic message echoes back, so a huge document does not flood the
/// failure output.
const MAX_ECHOED_CHARS: usize = 2000;

/// Truncates `text` to at most [`MAX_ECHOED_CHARS`] characters, for a panic message that must
/// show the offending text without risking an unbounded one.
fn truncated(text: &str) -> String {
    if text.chars().count() <= MAX_ECHOED_CHARS {
        return text.to_owned();
    }
    let mut shown: String = text.chars().take(MAX_ECHOED_CHARS).collect();
    shown.push_str("… (truncated)");
    shown
}

/// The stdout, stderr and exit status of the last command a scenario ran.
///
/// `When I run {string}` fills this component when it runs a command; the output steps only read
/// it. `Given the output:` and `Given the error output:` also fill it, for testing the output
/// steps on their own.
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

/// `Given the error output:` sets [`LastOutput::stderr`] to the doc string. It keeps the stdout
/// and status a `Given the output:` before it set, or starts from an empty [`LastOutput`]. It
/// lets a scenario test the stderr steps without running a command.
#[given("the error output:")]
fn the_error_output(world: &mut MorphirWorld, step: &Step) {
    let stderr = doc_string(step).to_owned();
    match world.context.get_mut::<LastOutput>() {
        Some(out) => out.stderr = stderr,
        None => world.context.insert(LastOutput {
            stderr,
            ..LastOutput::default()
        }),
    }
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
/// printing a unified diff on mismatch. Trailing whitespace is not significant: Gherkin doc
/// strings never end with a newline, but real command output usually does.
#[then("stdout should be:")]
fn stdout_is(world: &mut MorphirWorld, step: &Step) {
    let expected = doc_string(step);
    let actual = &last(world).stdout;
    if !same_text(expected, actual) {
        panic!(
            "stdout differs:\n{}",
            unified_diff(expected, actual, "expected stdout", "actual stdout")
        );
    }
}

/// `Then stderr should be:` asserts that the last command's stderr equals the doc string,
/// printing a unified diff on mismatch. As with `stdout should be:`, trailing whitespace is not
/// significant.
#[then("stderr should be:")]
fn stderr_is(world: &mut MorphirWorld, step: &Step) {
    let expected = doc_string(step);
    let actual = &last(world).stderr;
    if !same_text(expected, actual) {
        panic!(
            "stderr differs:\n{}",
            unified_diff(expected, actual, "expected stderr", "actual stderr")
        );
    }
}

/// `Then the JSON output at {string} should be:` parses the last command's stdout as JSON, reads
/// the value at the given JSON pointer, and compares it to the doc string, parsed as JSON too.
/// A mismatch prints a unified diff of the two values, pretty-printed. Every panic (invalid
/// stdout, an invalid doc string, or a missing pointer) echoes the offending text back, truncated
/// past [`MAX_ECHOED_CHARS`].
#[then(expr = "the JSON output at {string} should be:")]
fn json_at(world: &mut MorphirWorld, pointer: String, step: &Step) {
    let stdout = &last(world).stdout;
    let actual: serde_json::Value = serde_json::from_str(stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON: {e}\nstdout:\n{}", truncated(stdout)));
    let doc = doc_string(step);
    let expected: serde_json::Value = serde_json::from_str(doc).unwrap_or_else(|e| {
        panic!(
            "the doc string is not JSON: {e}\ndoc string:\n{}",
            truncated(doc)
        )
    });
    let found = actual.pointer(&pointer).unwrap_or_else(|| {
        let document = serde_json::to_string_pretty(&actual).expect("serialize");
        panic!("no value at {pointer}\ndocument:\n{}", truncated(&document))
    });
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
