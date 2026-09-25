//! [`SuiteDriver`]: the `TestDriver` for `morphir-bdd`'s own [`Suite`] acceptance scenarios.

use std::path::{Path, PathBuf};

use morphir_bdd::{Suite, SuiteResult, standard_extensions};
use morphir_gherkin::extension::Extensions;

/// Drives one [`Suite`] run at a time on behalf of a BDD scenario: writes `.feature` and
/// `.feature.md` files into a scenario-owned temporary features directory, configures the
/// suite's extensions and CLI program, runs it, and exposes its result and reports for
/// assertions. Domain methods are named `given_`/`when_`/`then_`, after the Gherkin steps they
/// stand in for.
pub struct SuiteDriver {
    /// Where every run's JSON and JUnit reports are written.
    out: tempfile::TempDir,
    /// Where [`SuiteDriver::given_a_feature`] writes `.feature` files, created on first use.
    features: Option<tempfile::TempDir>,
    /// The extensions the next run applies, if [`SuiteDriver::given_extensions`] or
    /// [`SuiteDriver::given_the_standard_extensions`] set one.
    extensions: Option<Extensions>,
    /// The CLI program the next run applies, if [`SuiteDriver::given_the_cli`] set one.
    cli: Option<PathBuf>,
    /// The last run's result, once a `when_the_suite_runs…` method has run one.
    result: Option<SuiteResult>,
    /// The last run's JSON report text.
    json: String,
    /// The last run's JUnit report text.
    junit: String,
}

impl SuiteDriver {
    /// A driver with a fresh, empty report output directory and no features written yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            out: tempfile::tempdir().expect("create a temporary report directory"),
            features: None,
            extensions: None,
            cli: None,
            result: None,
            json: String::new(),
            junit: String::new(),
        }
    }

    /// The temporary directory [`SuiteDriver::given_a_feature`] writes into, creating it the
    /// first time either is called.
    fn features_dir_mut(&mut self) -> &Path {
        self.features
            .get_or_insert_with(|| {
                tempfile::tempdir().expect("create a temporary features directory")
            })
            .path()
    }

    /// `Given a feature {name} containing:` writes `text` to `name` inside this driver's
    /// temporary features directory, creating that directory on first use.
    pub fn given_a_feature(&mut self, name: &str, text: &str) {
        let path = self.features_dir_mut().join(name);
        std::fs::write(&path, text)
            .unwrap_or_else(|e| panic!("write the feature file {}: {e}", path.display()));
    }

    /// `Given the standard extensions` sets the next run's extensions to
    /// [`standard_extensions`], the same ones a `Suite` gets by default: `@wip` skips a scenario.
    pub fn given_the_standard_extensions(&mut self) {
        self.extensions = Some(standard_extensions());
    }

    /// `Given these extensions` replaces the next run's extensions with `extensions`.
    pub fn given_extensions(&mut self, extensions: Extensions) {
        self.extensions = Some(extensions);
    }

    /// `Given the CLI at {path}` sets the program the next run's `When I run {string}` steps run,
    /// named `"morphir"` (see [`Suite::cli`]).
    pub fn given_the_cli(&mut self, path: impl AsRef<Path>) {
        self.cli = Some(path.as_ref().to_owned());
    }

    /// `When the suite {name} runs against {features_path}` runs a [`Suite`] named `name` against
    /// `features_path`, with no tag expression, applying whatever
    /// [`SuiteDriver::given_extensions`] / [`SuiteDriver::given_the_cli`] set, and records its
    /// result and reports for the `then_` methods to check.
    pub async fn when_the_suite_runs(&mut self, name: &str, features_path: impl AsRef<Path>) {
        self.run(name, features_path, None).await;
    }

    /// As [`SuiteDriver::when_the_suite_runs`], with the suite's tag expression set to `tags`.
    pub async fn when_the_suite_runs_with_tags(
        &mut self,
        name: &str,
        features_path: impl AsRef<Path>,
        tags: &str,
    ) {
        self.run(name, features_path, Some(tags.to_owned())).await;
    }

    /// `When the suite runs, configured by {configure}` builds a [`Suite`] from this driver's own
    /// temporary features directory and report output directory, applies whatever
    /// [`SuiteDriver::given_extensions`] / [`SuiteDriver::given_the_cli`] set, and then lets
    /// `configure` finish it — for example `.clear_tags().filter(…)`, `.with_component(…)` or
    /// `.max_concurrent_scenarios(…)` — before running it. As the other `when_the_suite_runs…`
    /// methods, it records the result and reports for the `then_` methods to check; it also
    /// returns the result directly, for a test that reads a specific count (for example
    /// `SuiteResult::passed`) straight off it. Panics if [`SuiteDriver::given_a_feature`] has not
    /// created a features directory yet.
    pub async fn when_the_suite_runs_with(
        &mut self,
        configure: impl FnOnce(Suite) -> Suite,
    ) -> SuiteResult {
        let features = self.features_dir().to_owned();
        let mut suite = Suite::new("driven")
            .features(features)
            .out_dir(self.out.path());
        if let Some(extensions) = self.extensions.take() {
            suite = suite.extensions(extensions);
        }
        if let Some(cli) = self.cli.take() {
            suite = suite.cli(cli);
        }
        let result = configure(suite).run().await;
        self.json = std::fs::read_to_string(&result.json)
            .unwrap_or_else(|e| panic!("read the JSON report {}: {e}", result.json.display()));
        self.junit = std::fs::read_to_string(&result.junit)
            .unwrap_or_else(|e| panic!("read the JUnit report {}: {e}", result.junit.display()));
        self.result = Some(result.clone());
        result
    }

    /// The shared run implementation behind both `when_the_suite_runs…` methods.
    async fn run(&mut self, name: &str, features_path: impl AsRef<Path>, tags: Option<String>) {
        let mut suite = Suite::new(name)
            .features(features_path)
            .out_dir(self.out.path());
        suite = match tags {
            Some(tags) => suite.tags(tags),
            None => suite.clear_tags(),
        };
        if let Some(extensions) = self.extensions.take() {
            suite = suite.extensions(extensions);
        }
        if let Some(cli) = self.cli.take() {
            suite = suite.cli(cli);
        }
        let result = suite.run().await;
        self.json = std::fs::read_to_string(&result.json)
            .unwrap_or_else(|e| panic!("read the JSON report {}: {e}", result.json.display()));
        self.junit = std::fs::read_to_string(&result.junit)
            .unwrap_or_else(|e| panic!("read the JUnit report {}: {e}", result.junit.display()));
        self.result = Some(result);
    }

    /// `Then it succeeds` asserts that the last run [`SuiteResult::succeeded`].
    pub fn then_it_succeeds(&self) {
        assert!(self.result().succeeded(), "{:?}", self.result());
    }

    /// `Then it fails with {failed} failed steps and {errors} errors` asserts the last run's
    /// exact failed-step and error counts.
    pub fn then_it_fails_with(&self, failed: usize, errors: usize) {
        assert_eq!(self.result().failed, failed, "{:?}", self.result());
        assert_eq!(self.result().errors, errors, "{:?}", self.result());
    }

    /// `Then the JSON report contains {text}` asserts that the last run's JSON report contains
    /// `text`.
    pub fn then_the_json_report_contains(&self, text: &str) {
        assert!(
            self.json.contains(text),
            "expected {text:?} in the JSON report:\n{}",
            self.json
        );
    }

    /// `Then the JUnit report contains {text}` asserts that the last run's JUnit report contains
    /// `text`.
    pub fn then_the_junit_report_contains(&self, text: &str) {
        assert!(
            self.junit.contains(text),
            "expected {text:?} in the JUnit report:\n{}",
            self.junit
        );
    }

    /// The last run's [`SuiteResult`], for assertions none of this driver's `then_` methods
    /// cover (for example an exact `passed` count). Panics if no `when_the_suite_runs…` method
    /// has run one yet.
    pub fn result(&self) -> &SuiteResult {
        self.result.as_ref().expect("no suite has run yet")
    }

    /// The last run's JSON report text, for assertions [`SuiteDriver::then_the_json_report_contains`]
    /// does not cover (for example asserting text is absent).
    #[must_use]
    pub fn json(&self) -> &str {
        &self.json
    }

    /// The last run's JUnit report text, for assertions
    /// [`SuiteDriver::then_the_junit_report_contains`] does not cover.
    #[must_use]
    pub fn junit(&self) -> &str {
        &self.junit
    }

    /// This driver's report output directory: every run's JSON and JUnit reports are written
    /// here, at `{name}.json` and `{name}.xml`.
    #[must_use]
    pub fn out_dir(&self) -> &Path {
        self.out.path()
    }

    /// This driver's temporary features directory. Panics if [`SuiteDriver::given_a_feature`] has
    /// not created one yet.
    #[must_use]
    pub fn features_dir(&self) -> &Path {
        self.features
            .as_ref()
            .expect("call given_a_feature first")
            .path()
    }
}

impl Default for SuiteDriver {
    fn default() -> Self {
        Self::new()
    }
}
