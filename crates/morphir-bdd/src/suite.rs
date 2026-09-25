//! One way to run every Morphir suite: morphir-gherkin parsing, extension context, tag filtering,
//! and console, JSON and JUnit output.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cucumber::event;
use cucumber::writer::{self, Coloring, Stats as _, Verbosity};
use cucumber::{World as _, WriterExt as _, gherkin};
use morphir_gherkin::extension::{Component, Context, Extensions};

use crate::parser::{MorphirParser, Reader, panic_payload_text, prepare, skip_reason};
use crate::steps::cli::CliProgram;
use crate::tags::{TagExpr, WipTag};
use crate::world::MorphirWorld;

/// The extensions every Morphir suite runs with unless it overrides them: `@wip` skips a scenario.
#[must_use]
pub fn standard_extensions() -> Extensions {
    Extensions::new().with_tags(WipTag)
}

/// Whether [`Suite::run`] prints a console summary as it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Console {
    /// Prints cucumber's usual summarized console output, the way a `Suite` always has:
    /// [`writer::Basic::stdout`] over [`writer::Summarize`].
    Full,
    /// Prints nothing to the console. The JSON and JUnit reports are still written; only the
    /// console output is silenced.
    Off,
}

/// One finished scenario, reported by [`Suite::on_scenario_finished`] once per scenario that ran,
/// in completion order.
#[derive(Debug, Clone)]
pub struct ScenarioOutcome {
    /// The source file the scenario was read from ([`gherkin::Feature::path`]). For a
    /// `.feature.md` document this is the Markdown file, not a file lowering ever produced.
    pub feature: PathBuf,
    /// The scenario's name as cucumber ran it. For an expanded outline row this is the row's own
    /// substituted name, not the outline's `<placeholder>` template.
    pub name: String,
    /// The feature's, rule's (if any) and scenario's tags, without a leading `@`. For an outline
    /// row this includes the row's own `Examples` tags: cucumber already merges those into the
    /// row's [`gherkin::Scenario::tags`] before this hook sees it.
    pub tags: Vec<String>,
    /// How many steps the scenario ran: its background steps (the feature's, plus the rule's if
    /// the scenario is under one) plus its own steps.
    pub steps: usize,
    /// Why the scenario did not pass: the failing step's error message, a hook error message, or
    /// a message naming an undefined or otherwise skipped step. `None` only if the scenario
    /// passed: the suite runs with `fail_on_skipped()`, so a skipped step is always a failure too.
    pub failure: Option<String>,
}

/// Builds a [`ScenarioOutcome`] from an `after` hook call: the [`gherkin::Feature`],
/// [`gherkin::Rule`] and [`gherkin::Scenario`] cucumber ran, and the
/// [`event::ScenarioFinished`] explaining how it finished.
fn scenario_outcome(
    feature: &gherkin::Feature,
    rule: Option<&gherkin::Rule>,
    scenario: &gherkin::Scenario,
    ev: &event::ScenarioFinished,
) -> ScenarioOutcome {
    let mut tags: Vec<String> = feature.tags.clone();
    if let Some(rule) = rule {
        tags.extend(rule.tags.clone());
    }
    tags.extend(scenario.tags.clone());

    let background_steps = feature.background.as_ref().map_or(0, |b| b.steps.len())
        + rule
            .and_then(|r| r.background.as_ref())
            .map_or(0, |b| b.steps.len());

    let failure = match ev {
        event::ScenarioFinished::StepFailed(_, _, err) => Some(err.to_string()),
        event::ScenarioFinished::BeforeHookFailed(info) => Some(format!(
            "before hook failed: {}",
            panic_payload_text(&**info)
        )),
        // cucumber's own `StepSkipped` covers both a step deliberately skipped and a step that
        // matched no step definition (undefined). Nothing in this event or in `ExecutionFailure`
        // (its source in cucumber 0.23) says which `gherkin::Step` it was: `ExecutionFailure::StepSkipped`
        // carries only `Option<World>`, no step reference, so there is no step text to add here.
        // The suite always runs with `fail_on_skipped()` (see `Suite::run`), which reclassifies a
        // skipped step as a failed one in the writer chain (`SuiteResult::failed`), so this must
        // count as a failure too, not a pass.
        event::ScenarioFinished::StepSkipped => {
            Some("a step was skipped or matched no step definition".to_owned())
        }
        event::ScenarioFinished::StepPassed => None,
    };

    ScenarioOutcome {
        feature: feature.path.clone().unwrap_or_default(),
        name: scenario.name.clone(),
        tags,
        steps: background_steps + scenario.steps.len(),
        failure,
    }
}

/// Builds and runs one Morphir Gherkin suite: a feature directory, an extension context, an
/// optional tag expression, and where to write its JSON and JUnit reports.
///
/// A `Suite` never parses process arguments. It takes its configuration only from the builder
/// methods above and the `MORPHIR_BDD_*` environment variables; [`run`](Suite::run) hands cucumber
/// an explicit default `cucumber::cli::Opts` instead of letting it parse `std::env::args()`. This
/// keeps a `Suite` safe to run inside a normal `#[test]`, where the process's real argv holds
/// libtest's own filter and flags (for example `cargo test -- some_test --exact`), not cucumber's.
///
/// A downstream crate usually runs its suite from a test binary of its own. Give the binary a
/// `[[test]]` entry with `harness = false` in `Cargo.toml` (for example `name = "bdd"`,
/// `harness = false`), so that its `main` runs instead of libtest's, and let `main` run the
/// suite in a Tokio runtime:
///
/// ```no_run
/// use morphir_bdd::Suite;
///
/// #[tokio::main]
/// async fn main() {
///     Suite::new("x").features("tests/features").run_and_exit().await
/// }
/// ```
///
/// The runtime must be Tokio: `When I run {string}` waits for its program on `tokio::process`.
pub struct Suite {
    name: String,
    features: PathBuf,
    extensions: Extensions,
    tags: Option<String>,
    out_dir: Option<PathBuf>,
    #[allow(clippy::type_complexity)]
    filter: Option<
        Arc<
            dyn Fn(&gherkin::Feature, Option<&gherkin::Rule>, &gherkin::Scenario) -> bool
                + Send
                + Sync,
        >,
    >,
    max_concurrent: Option<usize>,
    #[allow(clippy::type_complexity)]
    components: Vec<Arc<dyn Fn(&mut Context) + Send + Sync>>,
    console: Console,
    #[allow(clippy::type_complexity)]
    on_scenario_finished: Option<Arc<dyn Fn(&ScenarioOutcome) + Send + Sync>>,
    readers: HashMap<String, Reader>,
}

/// The counts and report paths from one [`Suite::run`].
#[derive(Debug, Clone)]
pub struct SuiteResult {
    /// How many steps passed.
    pub passed: usize,
    /// How many steps failed, including undefined steps (the suite runs with `fail_on_skipped`).
    pub failed: usize,
    /// How many steps were skipped without being counted as failed (for example `@allow.skipped`).
    pub skipped: usize,
    /// Parsing errors, plus scenario hook errors (for example an extension error while building a
    /// scenario's context), plus suite errors: a features path that does not exist or cannot be
    /// read, and a run that selects no scenario at all while no tag expression is set.
    pub errors: usize,
    /// Where the JSON report was written.
    pub json: PathBuf,
    /// Where the JUnit report was written.
    pub junit: PathBuf,
}

impl SuiteResult {
    /// Reports whether the suite had no failed steps and no errors of any kind (see
    /// [`SuiteResult::errors`]).
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.failed == 0 && self.errors == 0
    }
}

impl Suite {
    /// Starts a suite named `name`. Its features default to `tests/features`, its extensions to
    /// [`standard_extensions`], its tag expression to the `MORPHIR_BDD_TAGS` environment variable
    /// (unset means every scenario is a candidate), and its output directory to `MORPHIR_BDD_OUT`
    /// (unset means the workspace's `.dev/out/bdd`).
    ///
    /// `name` is joined onto the output directory to build the JSON and JUnit report paths (see
    /// [`Suite::run`]), so it must be a valid single path component: not empty, not `.` or `..`,
    /// and free of `/`, `\` and NUL. Panics otherwise, naming the invalid `name`. A `Result`-
    /// returning builder would be a larger API change than this check warrants; `Suite::new`
    /// already panics on other misuses of the builder, such as an invalid tag expression given to
    /// [`Suite::run`].
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        assert_valid_name(&name);
        Self {
            name,
            features: PathBuf::from("tests/features"),
            extensions: standard_extensions(),
            tags: std::env::var("MORPHIR_BDD_TAGS").ok(),
            out_dir: std::env::var_os("MORPHIR_BDD_OUT").map(PathBuf::from),
            filter: None,
            max_concurrent: None,
            components: Vec::new(),
            console: Console::Full,
            on_scenario_finished: None,
            readers: HashMap::new(),
        }
    }

    /// Sets where to discover `.feature` and `.feature.md` documents, plus any file whose exact
    /// name was registered with [`Suite::reader`]: discovery finds those the same way, anywhere
    /// under this root.
    #[must_use]
    pub fn features(mut self, path: impl AsRef<Path>) -> Self {
        self.features = path.as_ref().to_owned();
        self
    }

    /// Replaces the extensions this suite's scenarios build their context with.
    #[must_use]
    pub fn extensions(mut self, extensions: Extensions) -> Self {
        self.extensions = extensions;
        self
    }

    /// Sets the tag expression a scenario's tags (feature, rule and scenario) must satisfy to run.
    #[must_use]
    pub fn tags(mut self, expr: impl Into<String>) -> Self {
        self.tags = Some(expr.into());
        self
    }

    /// Clears the tag expression, including one taken from `MORPHIR_BDD_TAGS`, so every scenario is
    /// a candidate.
    #[must_use]
    pub fn clear_tags(mut self) -> Self {
        self.tags = None;
        self
    }

    /// Sets where the JSON and JUnit reports are written.
    #[must_use]
    pub fn out_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.out_dir = Some(dir.as_ref().to_owned());
        self
    }

    /// Sets an extra predicate a scenario must satisfy to be selected, on top of the tag
    /// expression and the skip reason a scenario's own tags may give it: a scenario runs only if
    /// the tag expression matches it (or none is set), `f` returns `true` for it, and it is not
    /// skipped. Later calls replace an earlier one; they do not combine with each other.
    ///
    /// A filter that selects no scenario while no tag expression is set is still the "empty run"
    /// error [`Suite::run`] documents, the same as an unfiltered run that happens to select
    /// nothing: a filter meant to allow zero scenarios must be paired with a tag expression that
    /// selects nothing on its own terms, for example `.tags("@nothing")`.
    #[must_use]
    pub fn filter(
        mut self,
        f: impl Fn(&gherkin::Feature, Option<&gherkin::Rule>, &gherkin::Scenario) -> bool
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.filter = Some(Arc::new(f));
        self
    }

    /// Sets how many scenarios cucumber runs at the same time. `1` runs scenarios one at a time,
    /// in parse order: the order [`MorphirParser`] discovers `.feature` and `.feature.md` files
    /// (sorted path order) and, within a file, the order its scenarios and outline rows appear.
    /// Leaving this unset lets cucumber run scenarios concurrently, with no ordering guarantee
    /// between them.
    #[must_use]
    pub fn max_concurrent_scenarios(mut self, n: usize) -> Self {
        self.max_concurrent = Some(n);
        self
    }

    /// Inserts a clone of `value` into every scenario's [`Context`] before its first step, after
    /// the suite's own extensions have built that context from the scenario's tags, fences and
    /// prose. Multiple calls apply in the order they were made, each free to overwrite an earlier
    /// component of the same type ([`Context::insert`]'s own last-write-wins rule).
    #[must_use]
    pub fn with_component<T: Component + Clone>(mut self, value: T) -> Self {
        self.components
            .push(Arc::new(move |ctx: &mut Context| ctx.insert(value.clone())));
        self
    }

    /// Sets whether `run` prints a console summary as it runs. `Console::Off` still writes the
    /// JSON and JUnit reports; only the console output is silenced.
    #[must_use]
    pub fn console(mut self, console: Console) -> Self {
        self.console = console;
        self
    }

    /// Registers a callback that runs once per finished scenario, in completion order, with a
    /// [`ScenarioOutcome`] describing it. cucumber allows only one `after` hook; a later call to
    /// this method replaces an earlier one rather than adding to it.
    #[must_use]
    pub fn on_scenario_finished(
        mut self,
        f: impl Fn(&ScenarioOutcome) + Send + Sync + 'static,
    ) -> Self {
        self.on_scenario_finished = Some(Arc::new(f));
        self
    }

    /// Registers `reader` for every file named exactly `file_name` (for example `scenarios.md`),
    /// found anywhere under [`Suite::features`], passed through to
    /// [`MorphirParser::with_reader`](crate::parser::MorphirParser::with_reader). See its docs for
    /// how such a file is discovered and loaded, and how a reader's `Err(message)` becomes a
    /// parsing error counted in [`SuiteResult::errors`]. A later call for the same `file_name`
    /// replaces an earlier one.
    #[must_use]
    pub fn reader(mut self, file_name: &str, reader: Reader) -> Self {
        self.readers.insert(file_name.to_owned(), reader);
        self
    }

    /// Sets the program `When I run {string}` runs, named `name`: the command line's first word
    /// must equal `name`. Use this for a tool other than `morphir` itself; [`Suite::cli`] covers
    /// `morphir`.
    #[must_use]
    pub fn cli_named(self, name: impl Into<String>, path: impl AsRef<Path>) -> Self {
        self.with_component(CliProgram {
            name: name.into(),
            path: path.as_ref().to_owned(),
        })
    }

    /// Sets the program `When I run {string}` runs, named `"morphir"`.
    #[must_use]
    pub fn cli(self, path: impl AsRef<Path>) -> Self {
        self.cli_named("morphir", path)
    }

    /// Runs the suite: parses its features, filters scenarios by tag expression and skip reason,
    /// runs every remaining step, and writes a console summary, a JSON report and a JUnit report.
    /// An undefined step fails its scenario, so [`SuiteResult::succeeded`] is `false` unless every
    /// selected scenario's steps were defined and passed.
    ///
    /// Two more cases count in [`SuiteResult::errors`] and print a message to stderr: a features
    /// path that does not exist or cannot be read, and a run with no tag expression that selects
    /// no scenario at all (every scenario was skipped, or none was found) and has no parsing
    /// error to explain it. A run that a tag expression narrows to no scenario is not an error.
    ///
    /// The first of those is checked up front, against `self.features` itself, before cucumber
    /// ever discovers anything: if it fires, the run stops there with an empty, all-zero result
    /// (the JSON and JUnit reports are still created, empty, so [`SuiteResult::json`] and
    /// [`SuiteResult::junit`] always name real files). Running the parser's own discovery over a
    /// path already known to be unreadable would report the very same failure a second time, once
    /// here and once as one of [`MorphirParser`]'s own per-directory parsing errors.
    pub async fn run(self) -> SuiteResult {
        crate::link();
        let out = self.out_dir.clone().unwrap_or_else(default_out_dir);
        std::fs::create_dir_all(&out).expect("create the output directory");
        let json_path = out.join(format!("{}.json", self.name));
        let junit_path = out.join(format!("{}.xml", self.name));
        if let Some(message) = unreadable_features(&self.features) {
            eprintln!("morphir-bdd: suite `{}`: {message}", self.name);
            std::fs::File::create(&json_path).expect("create the JSON report");
            std::fs::File::create(&junit_path).expect("create the JUnit report");
            return SuiteResult {
                passed: 0,
                failed: 0,
                skipped: 0,
                errors: 1,
                json: json_path,
                junit: junit_path,
            };
        }
        let mut suite_errors = 0;
        let has_tags = self.tags.is_some();
        let selected = Arc::new(AtomicUsize::new(0));
        let json = std::fs::File::create(&json_path).expect("create the JSON report");
        let junit = std::fs::File::create(&junit_path).expect("create the JUnit report");
        let expr = self
            .tags
            .as_deref()
            .map(|t| TagExpr::parse(t).expect("a valid tag expression"));
        let mut extensions = self.extensions;
        if !self.components.is_empty() {
            extensions = extensions.with_processor(InsertComponents(self.components));
        }
        let extensions = Arc::new(extensions);
        let filter = self.filter;
        let max_concurrent = self.max_concurrent;
        let on_scenario_finished = self.on_scenario_finished.clone();
        let mut parser = MorphirParser::new(extensions.clone());
        for (file_name, reader) in self.readers {
            parser = parser.with_reader(&file_name, reader);
        }

        // `Console::Off` keeps the same writer chain as `Console::Full`; only the `Basic` writer's
        // output target changes, from real stdout to a sink that discards every byte. `Stats`
        // (via `summarized()`) still counts steps either way, since it wraps the same chain.
        let console_target: Box<dyn std::io::Write + Send> = match self.console {
            Console::Full => Box::new(std::io::stdout()),
            Console::Off => Box::new(std::io::sink()),
        };

        let cucumber = MorphirWorld::cucumber::<PathBuf>()
            .with_parser(parser)
            .with_writer(
                writer::Basic::new(console_target, Coloring::Auto, Verbosity::Default)
                    .summarized()
                    .tee::<MorphirWorld, _>(writer::Json::for_tee(json))
                    .tee::<MorphirWorld, _>(writer::JUnit::for_tee(junit, 0))
                    .normalized(),
            )
            .fail_on_skipped()
            .max_concurrent_scenarios(max_concurrent)
            .with_default_cli()
            .before({
                let extensions = extensions.clone();
                move |feature, _rule, scenario, world| {
                    let extensions = extensions.clone();
                    Box::pin(async move {
                        if let Err(message) = prepare(world, feature, scenario, &extensions) {
                            panic!("{message}");
                        }
                    })
                }
            });

        let filter_scenarios = {
            let selected = selected.clone();
            move |feature: &gherkin::Feature,
                  rule: Option<&gherkin::Rule>,
                  scenario: &gherkin::Scenario| {
                let mut all_tags: Vec<String> = feature.tags.clone();
                if let Some(rule) = rule {
                    all_tags.extend(rule.tags.clone());
                }
                all_tags.extend(scenario.tags.clone());
                let keep = expr.as_ref().is_none_or(|e| e.matches(&all_tags))
                    && filter.as_ref().is_none_or(|f| f(feature, rule, scenario))
                    && skip_reason(feature, rule, scenario, &extensions).is_none();
                if keep {
                    selected.fetch_add(1, Ordering::SeqCst);
                }
                keep
            }
        };

        // cucumber allows only one `after` hook, so it is registered only when
        // `Suite::on_scenario_finished` actually set a callback.
        let writer = if let Some(on_finished) = on_scenario_finished {
            cucumber
                .after(move |feature, rule, scenario, ev, _world| {
                    let outcome = scenario_outcome(feature, rule, scenario, ev);
                    on_finished(&outcome);
                    Box::pin(async {})
                })
                .filter_run(self.features.clone(), filter_scenarios)
                .await
        } else {
            cucumber
                .filter_run(self.features.clone(), filter_scenarios)
                .await
        };
        // The features path itself is known readable at this point: an unreadable path returned
        // early above, before cucumber ran at all.
        if writer.parsing_errors() == 0 && !has_tags && selected.load(Ordering::SeqCst) == 0 {
            eprintln!(
                "morphir-bdd: suite `{}`: no scenario was selected from {} and no tag expression \
                 is set: every scenario was skipped, or none was found",
                self.name,
                self.features.display()
            );
            suite_errors += 1;
        }
        SuiteResult {
            passed: writer.passed_steps(),
            failed: writer.failed_steps(),
            skipped: writer.skipped_steps(),
            errors: writer.parsing_errors() + writer.hook_errors() + suite_errors,
            json: json_path,
            junit: junit_path,
        }
    }

    /// Runs the suite and exits the process with code `1` unless it [`SuiteResult::succeeded`].
    pub async fn run_and_exit(self) {
        let result = self.run().await;
        if !result.succeeded() {
            std::process::exit(1);
        }
    }
}

/// Panics unless `name` is safe to join onto an output directory as a bare file name: not empty,
/// not `.` or `..`, free of `/`, `\` and NUL, and a single [`std::path::Component::Normal`]. A
/// name that fails the first checks but somehow still parsed as one `Normal` component would be
/// redundant with them; a name that passes the character checks but is not a single `Normal`
/// component (for example a Windows drive prefix such as `C:`) would not be caught by them.
/// Checking both keeps the guarantee independent of how `Path::components` treats any one
/// platform's separators.
fn assert_valid_name(name: &str) {
    let single_normal_component = matches!(
        Path::new(name).components().collect::<Vec<_>>().as_slice(),
        [std::path::Component::Normal(_)]
    );
    let valid = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && single_normal_component;
    assert!(
        valid,
        "invalid suite name {name:?}: a suite name is joined into report file paths, so it must \
         be a single path component: not empty, not `.` or `..`, and free of `/`, `\\` and NUL"
    );
}

/// Why the features path cannot give a suite anything to run: it does not exist, or it (or, for a
/// directory, its listing) cannot be read. `None` when it can be read.
fn unreadable_features(path: &Path) -> Option<String> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(format!(
            "the features path {} does not exist",
            path.display()
        )),
        Err(e) => Some(format!(
            "the features path {} cannot be read: {e}",
            path.display()
        )),
        Ok(meta) if meta.is_dir() => std::fs::read_dir(path).err().map(|e| {
            format!(
                "the features directory {} cannot be read: {e}",
                path.display()
            )
        }),
        Ok(_) => None,
    }
}

/// A [`morphir_gherkin::extension::Processor`] that applies every closure a [`Suite::with_component`]
/// call registered, in the order they were registered, into every scenario's context. It is the
/// last processor a suite registers, so its components can see (and overwrite) whatever earlier
/// extensions already put there.
#[allow(clippy::type_complexity)]
struct InsertComponents(Vec<Arc<dyn Fn(&mut Context) + Send + Sync>>);

impl morphir_gherkin::extension::Processor for InsertComponents {
    fn process(
        &self,
        _doc: &morphir_gherkin::Document,
        _at: &morphir_gherkin::NodePath,
        ctx: &mut morphir_gherkin::extension::Context,
    ) -> Result<(), String> {
        for insert in &self.0 {
            insert(ctx);
        }
        Ok(())
    }
}

/// The default output directory: `.dev/out/bdd` under the nearest ancestor of the current
/// directory that holds a `Cargo.lock` (the workspace root), or `./.dev/out/bdd` if none is found.
fn default_out_dir() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    workspace_out_dir(&cwd)
}

/// `.dev/out/bdd` under the nearest ancestor of `start` (inclusive) that holds a `Cargo.lock`, or
/// `./.dev/out/bdd` if no ancestor does. Pure and side-effect-free so it can be unit tested without
/// touching the real current directory.
fn workspace_out_dir(start: &Path) -> PathBuf {
    let root = start
        .ancestors()
        .find(|dir| dir.join("Cargo.lock").is_file());
    root.unwrap_or(Path::new(".")).join(".dev/out/bdd")
}

#[cfg(test)]
mod tests {
    use super::{Suite, workspace_out_dir};

    #[test]
    #[should_panic(expected = "invalid suite name")]
    fn new_refuses_an_empty_name() {
        Suite::new("");
    }

    #[test]
    #[should_panic(expected = "invalid suite name")]
    fn new_refuses_a_single_dot() {
        Suite::new(".");
    }

    #[test]
    #[should_panic(expected = "invalid suite name")]
    fn new_refuses_a_double_dot() {
        Suite::new("..");
    }

    #[test]
    #[should_panic(expected = "invalid suite name")]
    fn new_refuses_a_name_with_a_parent_component() {
        Suite::new("../x");
    }

    #[test]
    #[should_panic(expected = "invalid suite name")]
    fn new_refuses_a_name_with_a_forward_slash() {
        Suite::new("a/b");
    }

    #[test]
    #[should_panic(expected = "invalid suite name")]
    fn new_refuses_a_name_with_a_backslash() {
        Suite::new(r"a\b");
    }

    #[test]
    #[should_panic(expected = "invalid suite name")]
    fn new_refuses_a_name_with_a_nul_byte() {
        Suite::new("a\0b");
    }

    #[test]
    fn new_accepts_an_ordinary_name() {
        Suite::new("a-normal-suite-name");
    }

    #[test]
    fn workspace_out_dir_finds_the_nearest_ancestor_with_a_cargo_lock() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("Cargo.lock"), "").unwrap();
        let nested = workspace.path().join("crates").join("some-crate");
        std::fs::create_dir_all(&nested).unwrap();

        let out = workspace_out_dir(&nested);

        assert_eq!(out, workspace.path().join(".dev/out/bdd"));
    }

    #[test]
    fn workspace_out_dir_falls_back_when_no_ancestor_has_a_cargo_lock() {
        let orphan = tempfile::tempdir().unwrap();
        // A fresh temp directory has no `Cargo.lock` in any of its ancestors up to `/`, so the
        // search must fall back to the current-directory-relative default.
        let out = workspace_out_dir(orphan.path());

        assert_eq!(out, std::path::Path::new(".").join(".dev/out/bdd"));
    }
}
