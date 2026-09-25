//! Runs the BDD suites for `morphir-bdd` itself. It proves the spike question: a step library
//! defined in this crate links into this integration test binary.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cucumber::writer::Stats as _;
use cucumber::{World as _, then};
use morphir_bdd::parser::MorphirParser;
use morphir_bdd::steps::probe::Linked;
use morphir_bdd::suite::Suite;
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
/// filtered out before it starts.
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
            move |_feature, _rule, scenario, _event, _world| {
                if scenario.name == "A failing step reports the markdown line"
                    && let Some(step) = scenario.steps.last()
                {
                    failing_step_line.store(step.position.line, Ordering::SeqCst);
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
}

/// Runs `tests/features/files_and_output.feature` through [`Suite`], the same runner the CLI
/// uses: file steps write inside a scenario-owned temporary directory, and output steps read a
/// `LastOutput` a step sets directly, without running a command. Its reports go to a fresh
/// temporary directory, never into the source tree.
async fn files_and_output_run() {
    let out_dir = tempfile::tempdir().expect("create a temporary report directory");
    let result = Suite::new("files-and-output")
        .features(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/features/files_and_output.feature"
        ))
        .out_dir(out_dir.path())
        .run()
        .await;
    assert!(result.succeeded(), "{result:?}");
    assert_eq!(result.failed, 0, "{result:?}");
    assert!(result.passed > 0, "{result:?}");
}

/// Runs `tests/features/cli.feature` through [`Suite`] with `Suite::cli` pointed at
/// `tests/fixtures/echo.sh`: a stand-in for `morphir` that echoes its arguments and `$HOME`, and
/// exits 3 for `fail`. Its reports go to a fresh temporary directory, never into the source tree.
/// `.sh` does not run on Windows, so the caller only runs this under `cfg!(unix)`.
async fn cli_run() {
    let echo = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/echo.sh");
    let out_dir = tempfile::tempdir().expect("create a temporary report directory");
    let result = Suite::new("cli")
        .features(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/features/cli.feature"
        ))
        .cli(echo)
        .out_dir(out_dir.path())
        .run()
        .await;
    assert!(result.succeeded(), "{result:?}");
    assert_eq!(result.failed, 0, "{result:?}");
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
