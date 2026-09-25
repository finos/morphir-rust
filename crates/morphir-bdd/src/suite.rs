//! One way to run every Morphir suite: morphir-gherkin parsing, extension context, tag filtering,
//! and console, JSON and JUnit output.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cucumber::writer::{self, Stats as _};
use cucumber::{World as _, WriterExt as _};
use morphir_gherkin::extension::Extensions;

use crate::parser::{MorphirParser, prepare, skip_reason};
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
pub struct Suite {
    name: String,
    features: PathBuf,
    extensions: Extensions,
    tags: Option<String>,
    out_dir: Option<PathBuf>,
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
    /// Parsing errors plus scenario hook errors.
    pub errors: usize,
    /// Where the JSON report was written.
    pub json: PathBuf,
    /// Where the JUnit report was written.
    pub junit: PathBuf,
}

impl SuiteResult {
    /// Reports whether the suite had no failed steps and no parsing or hook errors.
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

    /// Runs the suite: parses its features, filters scenarios by tag expression and skip reason,
    /// runs every remaining step, and writes a console summary, a JSON report and a JUnit report.
    /// An undefined step fails its scenario, so [`SuiteResult::succeeded`] is `false` unless every
    /// selected scenario's steps were defined and passed.
    pub async fn run(self) -> SuiteResult {
        crate::link();
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
        let extensions = Arc::new(self.extensions);

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
            .filter_run(self.features.clone(), move |feature, rule, scenario| {
                let mut all_tags: Vec<String> = feature.tags.clone();
                if let Some(rule) = rule {
                    all_tags.extend(rule.tags.clone());
                }
                all_tags.extend(scenario.tags.clone());
                expr.as_ref().is_none_or(|e| e.matches(&all_tags))
                    && skip_reason(feature, rule, scenario, &extensions).is_none()
            })
            .await;
        SuiteResult {
            passed: writer.passed_steps(),
            failed: writer.failed_steps(),
            skipped: writer.skipped_steps(),
            errors: writer.parsing_errors() + writer.hook_errors(),
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
