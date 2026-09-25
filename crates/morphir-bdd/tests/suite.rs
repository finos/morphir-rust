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

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use cucumber::then;
use drivers::suite_driver::SuiteDriver;
use morphir_bdd::parser::Reader;
use morphir_bdd::steps::cli::{CliRequest, CliRunner, CustomCliRunner};
use morphir_bdd::steps::files::Workspace;
use morphir_bdd::steps::output::LastOutput;
use morphir_bdd::world::MorphirWorld;
use morphir_bdd::{Console, ScenarioOutcome, Suite};
use morphir_gherkin::Tag;
use morphir_gherkin::extension::{Context, Effect, Extensions, Scope, TagExtension};
use morphir_gherkin::visit::Node;

const FEATURE: &str = "Feature: S\n  @keep\n  Scenario: kept\n    Given the step library is linked\n  Scenario: dropped\n    Given a step that no library defines\n";

/// A test-only component `Suite::with_component` can insert into every scenario's context. Used
/// by [`a_component_reaches_every_scenario`] to prove that `with_component` reaches every
/// scenario, not just one.
#[derive(Debug, Clone, PartialEq)]
struct Marker(u32);

/// `Then the marker is {int}` asserts that a [`Marker`] `with_component` inserted into this
/// scenario's context carries `expected`.
#[then(expr = "the marker is {int}")]
fn marker_is(world: &mut MorphirWorld, expected: u32) {
    assert_eq!(world.context.get::<Marker>(), Some(&Marker(expected)));
}

/// A test-only component: a shared log every `Then I record my name` step appends the running
/// scenario's name to. Used by [`a_sequential_run_keeps_parse_order`] to prove that
/// `Suite::max_concurrent_scenarios(1)` runs scenarios one at a time, in parse order.
#[derive(Debug, Clone)]
struct NameLog(Arc<Mutex<Vec<String>>>);

/// The running scenario's name, read from its node in the document: [`ScenarioRef`] carries no
/// name of its own. A plain scenario's [`ScenarioRef::path`] already names the scenario; one row
/// of an expanded outline names its own `Examples` block instead, so the scenario's name is read
/// from that path's parent.
///
/// [`ScenarioRef`]: morphir_bdd::world::ScenarioRef
/// [`ScenarioRef::path`]: morphir_bdd::world::ScenarioRef::path
fn running_scenario_name(world: &MorphirWorld) -> String {
    let scenario = world
        .scenario
        .as_ref()
        .expect("no running scenario: `prepare` has not filled the world yet");
    let path = if scenario.row.is_some() {
        scenario
            .path
            .parent()
            .expect("an outline row's examples path has a parent: the outline's own scenario path")
    } else {
        scenario.path.clone()
    };
    match scenario.document.node(&path) {
        Some(Node::Scenario(s)) => s.name.clone(),
        other => panic!("expected a scenario node at {path}, found {other:?}"),
    }
}

/// `Then I record my name` appends the running scenario's name to the scenario's [`NameLog`]
/// component.
#[then("I record my name")]
fn record_my_name(world: &mut MorphirWorld) {
    let name = running_scenario_name(world);
    let log = world
        .context
        .get::<NameLog>()
        .expect("no NameLog component: the suite must call `.with_component(NameLog(…))`")
        .0
        .clone();
    log.lock().expect("name log").push(name);
}

/// A key that tells every scenario and outline row apart, even two rows from different
/// `Examples` blocks of the same outline: those share both their (unsubstituted)
/// [`running_scenario_name`] and their block-local [`ScenarioRef::row`], so neither alone
/// distinguishes them. Combining the document's file name with the scenario name and the row
/// does: two different files never share a name, and two rows that share a name only collide
/// when they are also in the same block (where `row` alone already tells them apart).
///
/// [`ScenarioRef::row`]: morphir_bdd::world::ScenarioRef::row
fn running_scenario_key(world: &MorphirWorld) -> String {
    let name = running_scenario_name(world);
    let scenario = world
        .scenario
        .as_ref()
        .expect("no running scenario: `prepare` has not filled the world yet");
    let file = scenario
        .document
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("<unknown file>");
    format!("{file}:{name}:{:?}", scenario.row)
}

/// `Then I record my scenario` appends [`running_scenario_key`] to the scenario's [`NameLog`]
/// component. Unlike `Then I record my name`, which records only the bare scenario name (fine
/// when every scenario in a run has a distinct one), this also tells apart outline rows from
/// different `Examples` blocks of the same outline.
#[then("I record my scenario")]
fn record_my_scenario(world: &mut MorphirWorld) {
    let key = running_scenario_key(world);
    let log = world
        .context
        .get::<NameLog>()
        .expect("no NameLog component: the suite must call `.with_component(NameLog(…))`")
        .0
        .clone();
    log.lock().expect("name log").push(key);
}

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

/// `Suite::filter` selects scenarios by a predicate over the feature, rule and scenario, combined
/// with (AND) the tag expression and skip effects: here it keeps only the scenario named
/// `keep me`, so the other scenario's undefined step never runs and never fails the suite.
#[tokio::test]
async fn a_filter_selects_scenarios_by_name() {
    let mut driver = SuiteDriver::new();
    driver.given_a_feature(
        "f.feature",
        "Feature: F\n  Scenario: keep me\n    Given the step library is linked\n  Scenario: drop me\n    Given a step that no library defines\n",
    );
    let result = driver
        .when_the_suite_runs_with(|s| {
            s.clear_tags()
                .filter(|_, _, sc| sc.name.starts_with("keep"))
        })
        .await;
    driver.then_it_succeeds();
    assert_eq!(result.passed, 1, "{result:?}");
}

/// `Suite::with_component` inserts a clone of its value into every scenario's context, not just
/// one: both scenarios here read the same `Marker(7)`.
#[tokio::test]
async fn a_component_reaches_every_scenario() {
    let mut driver = SuiteDriver::new();
    driver.given_a_feature(
        "m.feature",
        "Feature: M\n  Scenario: a\n    Then the marker is 7\n  Scenario: b\n    Then the marker is 7\n",
    );
    let result = driver
        .when_the_suite_runs_with(|s| s.clear_tags().with_component(Marker(7)))
        .await;
    driver.then_it_succeeds();
    assert_eq!(result.passed, 2, "{result:?}");
}

/// `Suite::max_concurrent_scenarios(1)` runs scenarios one at a time, in parse order: the order
/// this single feature file lists them in.
#[tokio::test]
async fn a_sequential_run_keeps_parse_order() {
    let names = Arc::new(Mutex::new(Vec::new()));
    let mut driver = SuiteDriver::new();
    let body: String = (0..20)
        .map(|i| format!("  Scenario: s{i:02}\n    Then I record my name\n"))
        .collect();
    driver.given_a_feature("o.feature", &format!("Feature: O\n{body}"));
    let result = driver
        .when_the_suite_runs_with(|s| {
            s.clear_tags()
                .max_concurrent_scenarios(1)
                .with_component(NameLog(names.clone()))
        })
        .await;
    driver.then_it_succeeds();
    assert_eq!(result.passed, 20, "{result:?}");
    let seen = names.lock().expect("name log").clone();
    let expected: Vec<String> = (0..20).map(|i| format!("s{i:02}")).collect();
    assert_eq!(seen, expected);
}

/// `Suite::max_concurrent_scenarios(1)` keeps parse order not only within one file's plain
/// scenarios ([`a_sequential_run_keeps_parse_order`]), but also across feature files and across
/// an outline's `Examples` blocks. `a.feature` holds three plain scenarios; `b.feature` holds two
/// plain scenarios and a `Scenario Outline` with two `Examples` blocks (2 rows, then 3 rows). The
/// expected order is every scenario of `a.feature`, then every scenario of `b.feature`, with the
/// outline's rows in block-then-row order — the same order [`MorphirParser`](morphir_bdd::parser::MorphirParser)
/// discovers files (sorted path order: `a.feature` before `b.feature`) and registers each block's
/// rows (block order, then row order within a block).
///
/// Each step records [`running_scenario_key`], not the bare name `a_sequential_run_keeps_parse_order`
/// uses: two rows from different `Examples` blocks of this outline share both an (unsubstituted)
/// scenario name (`row <n>`) and a block-local row index, so the bare name or the row alone would
/// not tell a first-block row from a second-block row with the same index.
#[tokio::test]
async fn a_sequential_run_keeps_parse_order_across_files_and_outline_blocks() {
    let names = Arc::new(Mutex::new(Vec::new()));
    let mut driver = SuiteDriver::new();
    driver.given_a_feature(
        "a.feature",
        "Feature: A\n  Scenario: a0\n    Then I record my scenario\n  Scenario: a1\n    Then I record my scenario\n  Scenario: a2\n    Then I record my scenario\n",
    );
    driver.given_a_feature(
        "b.feature",
        "Feature: B\n  Scenario: b0\n    Then I record my scenario\n  Scenario: b1\n    Then I record my scenario\n\n  Scenario Outline: row <n>\n    Then I record my scenario\n\n    Examples: first\n      | n |\n      | 1 |\n      | 2 |\n\n    Examples: second\n      | n |\n      | 3 |\n      | 4 |\n      | 5 |\n",
    );
    let result = driver
        .when_the_suite_runs_with(|s| {
            s.clear_tags()
                .max_concurrent_scenarios(1)
                .with_component(NameLog(names.clone()))
        })
        .await;
    driver.then_it_succeeds();
    assert_eq!(result.passed, 10, "{result:?}");
    let seen = names.lock().expect("name log").clone();
    let expected: Vec<String> = [
        "a.feature:a0:None",
        "a.feature:a1:None",
        "a.feature:a2:None",
        "b.feature:b0:None",
        "b.feature:b1:None",
        "b.feature:row <n>:Some(0)",
        "b.feature:row <n>:Some(1)",
        "b.feature:row <n>:Some(0)",
        "b.feature:row <n>:Some(1)",
        "b.feature:row <n>:Some(2)",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(seen, expected);
}

/// A2: `Suite::on_scenario_finished` calls back once per scenario that ran, in completion order,
/// with a [`ScenarioOutcome`] carrying its name, step count, tags and failure. `good` passes with
/// one step and only the feature's own tag; `bad`'s `Then` step fails, and its failure message
/// carries the unmatched text.
#[tokio::test]
async fn every_finished_scenario_is_reported_once_with_its_failure() {
    let seen = Arc::new(Mutex::new(Vec::<ScenarioOutcome>::new()));
    let mut driver = SuiteDriver::new();
    driver.given_a_feature(
        "r.feature",
        "@area:x\nFeature: R\n  Scenario: good\n    Given the step library is linked\n  Scenario: bad\n    Given the output:\n      \"\"\"\n      a\n      \"\"\"\n    Then stdout should contain \"zzz\"\n",
    );
    let sink = seen.clone();
    let result = driver
        .when_the_suite_runs_with(move |s| {
            s.clear_tags()
                .console(Console::Off)
                .on_scenario_finished(move |o| sink.lock().unwrap().push(o.clone()))
        })
        .await;
    assert!(!result.succeeded());
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let good = seen.iter().find(|o| o.name == "good").unwrap();
    assert_eq!(
        (good.steps, good.failure.is_none(), good.tags.clone()),
        (1, true, vec!["area:x".to_owned()])
    );
    let bad = seen.iter().find(|o| o.name == "bad").unwrap();
    assert!(
        bad.failure.as_deref().unwrap().contains("zzz"),
        "{:?}",
        bad.failure
    );
}

/// A quiet console for a run that would otherwise print something recognizable: a feature named
/// distinctively, run with `Console::Off`. This test is not run directly by `cargo test`'s normal
/// selection in isolation from its own assertions; [`a_quiet_console_prints_nothing_the_basic_writer_would`]
/// runs it as a subprocess and inspects that process's real stdout, the same way
/// [`a_suite_ignores_the_process_arguments_a_test_filter_adds`] does for its own target.
#[tokio::test]
async fn a_console_off_run_prints_nothing_recognizable() {
    let mut driver = SuiteDriver::new();
    driver.given_a_feature(
        "r.feature",
        "Feature: R\n  Scenario: quiet\n    Given the step library is linked\n",
    );
    driver
        .when_the_suite_runs_with(|s| s.clear_tags().console(Console::Off))
        .await;
    driver.then_it_succeeds();
}

/// A2: `Console::Off` writes no console output. Cucumber's `Basic` writer writes straight to
/// `io::Stdout`, which bypasses libtest's own output capture (that capture only intercepts the
/// `print!`/`println!` macros), so a run with `Console::Full` would put `Feature: R` in this test
/// binary's real stdout even though `cargo test` runs it. This spawns the same compiled test
/// binary as a subprocess with an exact libtest filter targeting
/// [`a_console_off_run_prints_nothing_recognizable`], the same shape
/// [`a_suite_ignores_the_process_arguments_a_test_filter_adds`] uses, and asserts that subprocess's
/// stdout has none of it.
#[test]
fn a_quiet_console_prints_nothing_the_basic_writer_would() {
    let exe = std::env::current_exe().expect("this test binary's own path");
    let output = std::process::Command::new(exe)
        .args(["a_console_off_run_prints_nothing_recognizable", "--exact"])
        .output()
        .expect("run this test binary as a subprocess with a libtest filter");
    assert!(
        output.status.success(),
        "status: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Feature: R"),
        "Console::Off must print nothing cucumber's Basic writer would, got:\n{stdout}"
    );
}

/// A2 fix round 1: an undefined step gives `event::ScenarioFinished::StepSkipped`, not
/// `StepFailed` — cucumber's own vocabulary for "no step definition matched" is the same
/// `StepSkipped` event a deliberately skipped step gets, only reclassified as a failure by
/// `fail_on_skipped()` inside the writer chain. `scenario_outcome` must reclassify it the same
/// way, or a caller reading `ScenarioOutcome::failure` (such as `morphir itest`'s PASS/FAIL line)
/// would see a pass where `SuiteResult::failed` already counts one.
#[tokio::test]
async fn an_undefined_step_reports_one_outcome_with_a_failure() {
    let seen = Arc::new(Mutex::new(Vec::<ScenarioOutcome>::new()));
    let mut driver = SuiteDriver::new();
    driver.given_a_feature(
        "u.feature",
        "Feature: U\n  Scenario: undefined\n    Given a step that no library defines\n",
    );
    let sink = seen.clone();
    let result = driver
        .when_the_suite_runs_with(move |s| {
            s.clear_tags()
                .console(Console::Off)
                .on_scenario_finished(move |o| sink.lock().unwrap().push(o.clone()))
        })
        .await;
    assert!(!result.succeeded(), "{result:?}");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(seen[0].failure.is_some(), "{:?}", seen[0]);
}

/// A2 fix round 1: a before-hook failure (here, an extension's `apply` returning `Err`) reaches
/// the callback as a `ScenarioOutcome` whose `failure` carries the extension's own message, not
/// just a generic "hook failed" text.
#[tokio::test]
async fn a_before_hook_failure_is_reported_with_the_extensions_message() {
    let seen = Arc::new(Mutex::new(Vec::<ScenarioOutcome>::new()));
    let mut driver = SuiteDriver::new();
    driver.given_a_feature(
        "e.feature",
        "Feature: E\n  @refuse:x\n  Scenario: refused\n    Given the step library is linked\n",
    );
    driver.given_extensions(Extensions::new().with_tags(Refuse));
    let sink = seen.clone();
    driver
        .when_the_suite_runs_with(move |s| {
            s.clear_tags()
                .console(Console::Off)
                .on_scenario_finished(move |o| sink.lock().unwrap().push(o.clone()))
        })
        .await;
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(
        seen[0]
            .failure
            .as_deref()
            .unwrap()
            .contains("refused on purpose"),
        "{:?}",
        seen[0].failure
    );
}

/// A2 fix round 1: a `@wip` scenario next to a normal one is skipped before it starts (the
/// default extensions' `WipTag`), so cucumber never runs it and the `after` hook never fires for
/// it: only the normal scenario is reported.
#[tokio::test]
async fn a_wip_scenario_next_to_a_normal_one_is_not_reported() {
    let seen = Arc::new(Mutex::new(Vec::<ScenarioOutcome>::new()));
    let mut driver = SuiteDriver::new();
    driver.given_a_feature(
        "w.feature",
        "Feature: W\n  @wip\n  Scenario: not ready\n    Given the step library is linked\n\n  Scenario: ready\n    Given the step library is linked\n",
    );
    let sink = seen.clone();
    driver
        .when_the_suite_runs_with(move |s| {
            s.clear_tags()
                .console(Console::Off)
                .on_scenario_finished(move |o| sink.lock().unwrap().push(o.clone()))
        })
        .await;
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].name, "ready");
}

/// A2 fix round 1: an outline's two `Examples` rows each get their own `ScenarioOutcome`, with
/// the row's own expanded name (`<x>` substituted, not the template) and the row's `Examples`
/// tags, already merged into `scenario.tags` by cucumber before the `after` hook sees it.
#[tokio::test]
async fn an_outline_reports_each_row_with_its_expanded_name_and_examples_tags() {
    let seen = Arc::new(Mutex::new(Vec::<ScenarioOutcome>::new()));
    let mut driver = SuiteDriver::new();
    driver.given_a_feature(
        "o.feature",
        "Feature: O\n  Scenario Outline: row <x>\n    Given the step library is linked\n\n    @ex:a\n    Examples: only\n      | x |\n      | 1 |\n      | 2 |\n",
    );
    let sink = seen.clone();
    driver
        .when_the_suite_runs_with(move |s| {
            s.clear_tags()
                .console(Console::Off)
                .on_scenario_finished(move |o| sink.lock().unwrap().push(o.clone()))
        })
        .await;
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "{seen:?}");
    let mut names: Vec<&str> = seen.iter().map(|o| o.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["row 1", "row 2"]);
    for outcome in seen.iter() {
        assert!(
            outcome.tags.contains(&"ex:a".to_owned()),
            "{:?}",
            outcome.tags
        );
    }
}

/// A3: a reader registered by exact file name (`Suite::reader`) lowers its own file format into
/// a `morphir_gherkin::Document`, in place of the built-in `.feature`/`.feature.md` suffix rules.
/// This toy format turns each `step: <text>` line into a `Given` step of one scenario.
#[tokio::test]
async fn a_custom_reader_lowers_its_own_file_format() {
    let reader: Reader = Arc::new(|path: &Path| {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let steps: String = text
            .lines()
            .filter_map(|l| l.strip_prefix("step: "))
            .map(|s| format!("    Given {s}\n"))
            .collect();
        morphir_gherkin::read_str(
            path.to_str().unwrap().replace("toy.txt", "toy.feature"),
            &format!("Feature: Toy\n  Scenario: from toy\n{steps}"),
        )
        .map(|(doc, _)| doc)
        .map_err(|e| e.to_string())
    });
    let mut driver = SuiteDriver::new();
    driver.given_a_feature("toy.txt", "step: the step library is linked\n");
    let result = driver
        .when_the_suite_runs_with(move |s| s.clear_tags().reader("toy.txt", reader))
        .await;
    driver.then_it_succeeds();
    assert_eq!(result.passed, 1);
}

/// A3: a reader's `Err(message)` becomes a parsing error, the same way a `ReadError` does: it
/// counts in `SuiteResult::errors` and its message reaches the JSON report.
#[tokio::test]
async fn a_reader_error_fails_the_run_with_its_message() {
    let reader: Reader = Arc::new(|_path: &Path| Err("bad toy".to_owned()));
    let mut driver = SuiteDriver::new();
    driver.given_a_feature("toy.txt", "step: the step library is linked\n");
    let result = driver
        .when_the_suite_runs_with(move |s| s.clear_tags().reader("toy.txt", reader))
        .await;
    assert!(result.errors >= 1, "{result:?}");
    assert!(!result.succeeded(), "{result:?}");
    driver.then_the_json_report_contains("bad toy");
}

/// Fix round 1, Important 1: a reader that panics instead of returning `Err` must not take the
/// whole run down with it. The panic is caught and turned into a parsing error naming the file
/// and the panic's own message, and a sibling feature file in the same run still runs and passes.
/// The panicking file is named so it shares no substring with the panic message, so a report that
/// contains both names proves both reached it, not just one via the other.
#[tokio::test]
async fn a_panicking_reader_fails_only_its_own_file_and_reports_the_panic() {
    let reader: Reader = Arc::new(|_path: &Path| panic!("boom"));
    let mut driver = SuiteDriver::new();
    driver.given_a_feature("panicky.txt", "irrelevant content\n");
    driver.given_a_feature(
        "ok.feature",
        "Feature: K\n  Scenario: k\n    Given the step library is linked\n",
    );
    let result = driver
        .when_the_suite_runs_with(move |s| s.clear_tags().reader("panicky.txt", reader))
        .await;
    assert!(!result.succeeded(), "{result:?}");
    assert!(result.errors >= 1, "{result:?}");
    assert_eq!(
        result.passed, 1,
        "the sibling feature file must still run and pass: {result:?}"
    );
    driver.then_the_json_report_contains("panicky.txt");
    driver.then_the_json_report_contains("boom");
}

/// Fix round 1, Important 2: a reader registered for an exact file name wins over the built-in
/// `.feature.md` suffix rule for that same name, not just for a name the built-in rules would
/// never have matched at all. `x.feature.md`'s real content is malformed by the built-in
/// Markdown-Gherkin grammar (a step continuation line, the same shape
/// `a_malformed_feature_md_fails_the_run_with_its_message` proves is refused elsewhere); the
/// custom reader ignores that content entirely and always lowers to its own one passing scenario.
/// A run that succeeds, with the reader's own scenario name in the report, proves the reader ran
/// in its place — the built-in grammar would have failed the run on this content instead.
#[tokio::test]
async fn a_reader_wins_over_the_built_in_feature_md_grammar_for_its_exact_name() {
    let malformed_markdown_gherkin =
        "# Feature: F\n\n## Scenario: S\n\n* Given a step\n  that goes on\n";
    let reader: Reader = Arc::new(|path: &Path| {
        // A `.feature` name, not `.feature.md`: the text below is plain Gherkin, and
        // `read_str` picks its grammar from the path's own suffix.
        let synthetic_path = path.with_extension("feature");
        morphir_gherkin::read_str(
            synthetic_path,
            "Feature: Custom\n  Scenario: from the reader\n    Given the step library is linked\n",
        )
        .map(|(doc, _)| doc)
        .map_err(|e| e.to_string())
    });
    let mut driver = SuiteDriver::new();
    driver.given_a_feature("x.feature.md", malformed_markdown_gherkin);
    let result = driver
        .when_the_suite_runs_with(move |s| s.clear_tags().reader("x.feature.md", reader))
        .await;
    driver.then_it_succeeds();
    assert_eq!(result.passed, 1, "{result:?}");
    driver.then_the_json_report_contains("from the reader");
}

/// A4 test-only [`CliRunner`]: instead of running a real process, it records the request's own
/// arguments as [`LastOutput::stdout`]. Used by
/// [`a_custom_cli_runner_is_used_and_no_workspace_is_created`] to prove `CustomCliRunner` is
/// called in place of the default `run_program`, and that its own directories (here, none) are
/// used instead of a scenario [`Workspace`].
#[derive(Debug)]
struct EchoArgsRunner;

impl CliRunner for EchoArgsRunner {
    fn run<'a>(
        &'a self,
        request: CliRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LastOutput, String>> + Send + 'a>> {
        Box::pin(async move {
            Ok(LastOutput {
                stdout: format!("ran {:?}", request.args),
                status: Some(0),
                ..LastOutput::default()
            })
        })
    }
}

/// `Then no workspace was created` (test-only): asserts that `When I run {string}` did not
/// create a [`Workspace`] for this scenario, because a [`CustomCliRunner`] handled the command
/// instead.
#[then("no workspace was created")]
fn no_workspace_was_created(world: &mut MorphirWorld) {
    assert!(
        world.context.get::<Workspace>().is_none(),
        "a Workspace must not be created when a CustomCliRunner is present"
    );
}

/// A4: `Suite::with_component` installs a [`CustomCliRunner`], and `When I run {string}` calls
/// it in place of the default `run_program`, without ever creating a [`Workspace`].
#[tokio::test]
async fn a_custom_cli_runner_is_used_and_no_workspace_is_created() {
    let mut driver = SuiteDriver::new();
    driver.given_a_feature(
        "c.feature",
        "Feature: C\n  Scenario: custom runner\n    When I run \"morphir a b\"\n    Then stdout should contain 'ran [\"a\", \"b\"]'\n    Then no workspace was created\n",
    );
    let result = driver
        .when_the_suite_runs_with(|s| {
            s.clear_tags()
                .cli("unused")
                .with_component(CustomCliRunner(Arc::new(EchoArgsRunner)))
        })
        .await;
    driver.then_it_succeeds();
    assert_eq!(result.passed, 3, "{result:?}");
}
