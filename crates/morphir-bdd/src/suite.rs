//! One way to run every Morphir suite: morphir-gherkin parsing, extension context, tag filtering,
//! and console, JSON and JUnit output.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cucumber::writer::{self, Stats as _};
use cucumber::{World as _, WriterExt as _};
use morphir_gherkin::extension::Extensions;

use crate::parser::{MorphirParser, prepare, skip_reason};
use crate::steps::cli::CliProgram;
use crate::tags::{TagExpr, WipTag};
use crate::world::MorphirWorld;

/// The extensions every Morphir suite runs with unless it overrides them: `@wip` skips a scenario.
#[must_use]
pub fn standard_extensions() -> Extensions {
    Extensions::new().with_tags(WipTag)
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
    cli: Option<CliProgram>,
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
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            features: PathBuf::from("tests/features"),
            extensions: standard_extensions(),
            tags: std::env::var("MORPHIR_BDD_TAGS").ok(),
            out_dir: std::env::var_os("MORPHIR_BDD_OUT").map(PathBuf::from),
            cli: None,
        }
    }

    /// Sets where to discover `.feature` and `.feature.md` documents.
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

    /// Sets the program `When I run {string}` runs, named `name`: the command line's first word
    /// must equal `name`. Use this for a tool other than `morphir` itself; [`Suite::cli`] covers
    /// `morphir`.
    #[must_use]
    pub fn cli_named(mut self, name: impl Into<String>, path: impl AsRef<Path>) -> Self {
        self.cli = Some(CliProgram {
            name: name.into(),
            path: path.as_ref().to_owned(),
        });
        self
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
    pub async fn run(self) -> SuiteResult {
        crate::link();
        let mut suite_errors = 0;
        let features_error = unreadable_features(&self.features);
        if let Some(message) = &features_error {
            eprintln!("morphir-bdd: suite `{}`: {message}", self.name);
            suite_errors += 1;
        }
        let has_tags = self.tags.is_some();
        let selected = Arc::new(AtomicUsize::new(0));
        let out = self.out_dir.clone().unwrap_or_else(default_out_dir);
        std::fs::create_dir_all(&out).expect("create the output directory");
        let json_path = out.join(format!("{}.json", self.name));
        let junit_path = out.join(format!("{}.xml", self.name));
        let json = std::fs::File::create(&json_path).expect("create the JSON report");
        let junit = std::fs::File::create(&junit_path).expect("create the JUnit report");
        let expr = self
            .tags
            .as_deref()
            .map(|t| TagExpr::parse(t).expect("a valid tag expression"));
        let mut extensions = self.extensions;
        if let Some(program) = self.cli {
            extensions = extensions.with_processor(InsertCli(program));
        }
        let extensions = Arc::new(extensions);

        let writer = MorphirWorld::cucumber::<PathBuf>()
            .with_parser(MorphirParser::new(extensions.clone()))
            .with_writer(
                writer::Basic::stdout()
                    .summarized()
                    .tee::<MorphirWorld, _>(writer::Json::for_tee(json))
                    .tee::<MorphirWorld, _>(writer::JUnit::for_tee(junit, 0))
                    .normalized(),
            )
            .fail_on_skipped()
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
            })
            .filter_run(self.features.clone(), {
                let selected = selected.clone();
                move |feature, rule, scenario| {
                    let mut all_tags: Vec<String> = feature.tags.clone();
                    if let Some(rule) = rule {
                        all_tags.extend(rule.tags.clone());
                    }
                    all_tags.extend(scenario.tags.clone());
                    let keep = expr.as_ref().is_none_or(|e| e.matches(&all_tags))
                        && skip_reason(feature, rule, scenario, &extensions).is_none();
                    if keep {
                        selected.fetch_add(1, Ordering::SeqCst);
                    }
                    keep
                }
            })
            .await;
        if features_error.is_none()
            && writer.parsing_errors() == 0
            && !has_tags
            && selected.load(Ordering::SeqCst) == 0
        {
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

/// A [`morphir_gherkin::extension::Processor`] that inserts a suite's [`CliProgram`] into every
/// scenario's context, so `When I run {string}` can find the program to run. [`Suite::cli`] and
/// [`Suite::cli_named`] register one of these when they are used.
struct InsertCli(CliProgram);

impl morphir_gherkin::extension::Processor for InsertCli {
    fn process(
        &self,
        _doc: &morphir_gherkin::Document,
        _at: &morphir_gherkin::NodePath,
        ctx: &mut morphir_gherkin::extension::Context,
    ) -> Result<(), String> {
        ctx.insert(self.0.clone());
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
    use super::workspace_out_dir;

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
