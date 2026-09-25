//! Runs the BDD suites for `morphir-bdd` itself. It proves the spike question: a step library
//! defined in this crate links into this integration test binary.
//!
//! `files_and_output_run` and `cli_run` orchestrate through `SuiteDriver` (see AGENTS.md's "BDD
//! Test Drivers" rule): both run a `Suite` against a fixed features path already checked into
//! `tests/features`, with no `SuiteDriver::given_a_feature` call needed. `context_run` stays on a
//! raw cucumber chain instead: it hooks `after` to read back each scenario's own reported source
//! line, which `Suite`/`SuiteResult` has no way to expose, so there is no `SuiteDriver` method
//! this run could sit behind.

mod drivers;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use cucumber::writer::Stats as _;
use cucumber::{World as _, then};
use drivers::suite_driver::SuiteDriver;
use morphir_bdd::parser::MorphirParser;
use morphir_bdd::steps::probe::Linked;
use morphir_bdd::world::MorphirWorld;
use morphir_gherkin::Tag;
use morphir_gherkin::extension::{Context, Effect, Extensions, Scope, TagExtension};

#[then("the linked flag is set")]
fn linked_flag(world: &mut MorphirWorld) {
    assert_eq!(world.context.get::<Linked>(), Some(&Linked(true)));
}

#[derive(Debug, PartialEq)]
struct Probe(String);

struct ProbeTags;
impl TagExtension for ProbeTags {
    fn namespace(&self) -> Option<&str> {
        Some("probe")
    }
    fn apply(&self, tag: &Tag, _scope: Scope, ctx: &mut Context) -> Result<Effect, String> {
        ctx.insert(Probe(
            tag.namespaced()
                .map(|(_, v)| v.to_owned())
                .unwrap_or_default(),
        ));
        Ok(Effect::Continue)
    }
}

#[then(expr = "the probe value is {string}")]
fn probe_value(world: &mut MorphirWorld, expected: String) {
    assert_eq!(world.context.get::<Probe>(), Some(&Probe(expected)));
}

/// Runs `tests/features/context`: extensions fill the context from feature and scenario tags, a
/// failing step still reports the source line its list item is on, and a `@wip` scenario is
/// filtered out before it starts. Each row of the outline sees only its own `Examples` block's
/// `@probe:` value (or the feature's, for the untagged block), and each row reports the line of
/// its own data row in the Markdown table.
async fn context_run() {
    let extensions = Arc::new(
        Extensions::new()
            .with_tags(ProbeTags)
            .with_tags(morphir_bdd::tags::WipTag),
    );
    // The failing step's list item is at line 17 of context.feature.md. An `after` hook is the
    // simplest way to check it in cucumber 0.23: it hands back the same `gherkin::Scenario` the
    // runner executed, so its last step's `position.line` is read directly, with no writer output
    // to parse.
    let failing_step_line = Arc::new(AtomicUsize::new(0));
    let row_lines = Arc::new(Mutex::new(Vec::<(String, usize)>::new()));
    let writer = MorphirWorld::cucumber::<&str>()
        .with_parser(MorphirParser::new(extensions.clone()))
        .before({
            let extensions = extensions.clone();
            move |feature, _rule, scenario, world| {
                let extensions = extensions.clone();
                Box::pin(async move {
                    morphir_bdd::parser::prepare(world, feature, scenario, &extensions)
                        .expect("extensions apply");
                })
            }
        })
        .after({
            let failing_step_line = failing_step_line.clone();
            let row_lines = row_lines.clone();
            move |_feature, _rule, scenario, _event, _world| {
                if scenario.name == "A failing step reports the markdown line"
                    && let Some(step) = scenario.steps.last()
                {
                    failing_step_line.store(step.position.line, Ordering::SeqCst);
                }
                if scenario.name.ends_with("tag reaches an outline row") {
                    row_lines
                        .lock()
                        .expect("row lines")
                        .push((scenario.name.clone(), scenario.position.line));
                }
                Box::pin(async {})
            }
        })
        .filter_run(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/features/context"),
            {
                let extensions = extensions.clone();
                move |feature, rule, scenario| {
                    morphir_bdd::parser::skip_reason(feature, rule, scenario, &extensions).is_none()
                }
            },
        )
        .await;
    assert_eq!(writer.failed_steps(), 1, "only the 'never' step fails");
    assert_eq!(
        failing_step_line.load(Ordering::SeqCst),
        17,
        "the failing step's report names context.feature.md at line 17"
    );
    let mut rows = row_lines.lock().expect("row lines").clone();
    rows.sort();
    let row =
        |source: &str, line: usize| (format!("The {source} tag reaches an outline row"), line);
    assert_eq!(
        rows,
        vec![
            row("again", 30),
            row("first", 29),
            row("second", 38),
            row("untagged", 44)
        ],
        "each outline row reports its own data row's line in context.feature.md"
    );
}

/// Runs `tests/features/files_and_output.feature` through [`Suite`](morphir_bdd::Suite), the
/// same runner the CLI uses, via [`SuiteDriver`]: file steps write inside a scenario-owned
/// temporary directory, and output steps read a `LastOutput` a step sets directly, without
/// running a command. Its reports go to the driver's own temporary directory, never into the
/// source tree.
async fn files_and_output_run() {
    let mut driver = SuiteDriver::new();
    driver
        .when_the_suite_runs(
            "files-and-output",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/features/files_and_output.feature"
            ),
        )
        .await;
    driver.then_it_succeeds();
    assert_eq!(driver.result().failed, 0, "{:?}", driver.result());
    assert!(driver.result().passed > 0, "{:?}", driver.result());
}

/// Runs `tests/features/cli.feature` through [`Suite`](morphir_bdd::Suite), via [`SuiteDriver`],
/// with its CLI program pointed at `tests/fixtures/echo.sh`: a stand-in for `morphir` that echoes
/// its arguments, `$HOME` and `$MORPHIR_HOME`, and exits 3 for `fail`. Its reports go to the
/// driver's own temporary directory, never into the source tree. `.sh` does not run on Windows,
/// so the caller only runs this under `cfg!(unix)`.
async fn cli_run() {
    let echo = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/echo.sh");
    let mut driver = SuiteDriver::new();
    driver.given_the_cli(echo);
    driver
        .when_the_suite_runs(
            "cli",
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/features/cli.feature"),
        )
        .await;
    driver.then_it_succeeds();
    assert_eq!(driver.result().failed, 0, "{:?}", driver.result());
}

#[tokio::main]
async fn main() {
    morphir_bdd::link();
    let writer = MorphirWorld::cucumber()
        .fail_on_skipped()
        .run(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/features/link.feature"
        ))
        .await;
    // The second scenario must fail: an undefined step is not a pass or a silent skip.
    assert_eq!(
        writer.passed_steps(),
        2,
        "the library step and the local step ran"
    );
    assert!(
        writer.execution_has_failed(),
        "a missing step must fail its scenario"
    );

    context_run().await;
    files_and_output_run().await;
    if cfg!(unix) {
        cli_run().await;
    }
}
