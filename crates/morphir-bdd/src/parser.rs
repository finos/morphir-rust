//! Feeds morphir-gherkin documents to cucumber-rs and finds each running scenario's model node.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use cucumber::feature::Ext as _;
use cucumber::{gherkin, parser};
use futures::stream;
use morphir_gherkin::extension::{Effect, Extensions};
use morphir_gherkin::{Document, Format, NodePath, Segment, StepArgument, StepKind};

use crate::world::{MorphirWorld, ScenarioRef};

type Key = (PathBuf, String, usize);
type ModelNode = (Arc<Document>, NodePath, Option<usize>);

static REGISTRY: LazyLock<Mutex<HashMap<Key, ModelNode>>> = LazyLock::new(Default::default);

/// A custom way to read one file format into a [`morphir_gherkin::Document`], registered by exact
/// file name through [`MorphirParser::with_reader`] or [`Suite::reader`](crate::suite::Suite::reader).
///
/// A reader is given the file's path and reads it however it likes (its own text format, its own
/// I/O errors and all); on success it returns the [`Document`] the file lowers to. `Err(message)`
/// becomes a parsing error the same way a built-in [`morphir_gherkin::ReadError`] does: `message`
/// is folded into a parsing error that names the file, so it reaches the console, JSON and JUnit
/// reports and counts in [`SuiteResult::errors`](crate::suite::SuiteResult::errors).
pub type Reader = Arc<dyn Fn(&Path) -> Result<Document, String> + Send + Sync>;

/// A cucumber-rs [`cucumber::Parser`] that reads `.feature` and `.feature.md` documents through
/// morphir-gherkin instead of cucumber's own `gherkin` parser.
///
/// Given a file, it reads that file; given a directory, it reads every `.feature` and
/// `.feature.md` file under it, recursively, in sorted path order. Each document is lowered to a
/// `gherkin::Feature` and its outlines are expanded to one scenario per data row, as cucumber's
/// own parser does. Each expanded scenario is registered with its model node, so [`prepare`] and
/// [`skip_reason`] can build its context later. A document that cannot be read gives a parsing
/// error that names the file, line and column.
///
/// Discovery itself can also fail, short of any document read: a nested directory that cannot be
/// listed (for example one with its read permission removed), or a directory entry whose metadata
/// cannot be read. Neither one is skipped in silence; each becomes a parsing error of its own, so
/// a dropped subtree is a failed, not a passed, run. [`Suite`](crate::suite::Suite) is the checked
/// path: it sums every parsing error, this crate's own included, into
/// [`SuiteResult::errors`](crate::suite::SuiteResult::errors) and its writer prints each one. A
/// raw cucumber chain that uses this parser directly sees the same one-error-per-directory
/// behavior, since it comes from the parser, not from `Suite`.
///
/// [`Suite`](crate::suite::Suite) is the usual entry point: it builds a `MorphirParser`, wires its
/// `before` hook and scenario filter, and writes the console, JSON and JUnit reports for you. Use
/// `MorphirParser` directly only when a suite needs its own cucumber chain, for example to add
/// writers or hooks `Suite` does not expose. The chain still needs [`prepare`] in a `before` hook,
/// so the world's context is built from the scenario's tags before its first step runs, and
/// [`skip_reason`] in the scenario filter, so a `@wip`-style skip decided before the run starts
/// stays a skip instead of running the scenario and then discarding it:
///
/// ```no_run
/// use std::sync::Arc;
///
/// use cucumber::World as _;
/// use morphir_bdd::parser::{MorphirParser, prepare, skip_reason};
/// use morphir_bdd::world::MorphirWorld;
/// use morphir_gherkin::extension::Extensions;
///
/// #[tokio::main]
/// async fn main() {
///     let extensions = Arc::new(Extensions::new());
///     MorphirWorld::cucumber::<&str>()
///         .with_parser(MorphirParser::new(extensions.clone()))
///         .before({
///             let extensions = extensions.clone();
///             move |feature, _rule, scenario, world| {
///                 let extensions = extensions.clone();
///                 Box::pin(async move {
///                     if let Err(message) = prepare(world, feature, scenario, &extensions) {
///                         panic!("{message}");
///                     }
///                 })
///             }
///         })
///         .filter_run("tests/features", {
///             let extensions = extensions.clone();
///             move |feature, rule, scenario| {
///                 skip_reason(feature, rule, scenario, &extensions).is_none()
///             }
///         })
///         .await;
/// }
/// ```
pub struct MorphirParser {
    #[allow(dead_code)]
    extensions: Arc<Extensions>,
    readers: HashMap<String, Reader>,
}

impl MorphirParser {
    /// A parser for a suite that builds its scenario contexts with `extensions`, with no custom
    /// readers registered yet.
    pub fn new(extensions: Arc<Extensions>) -> Self {
        Self {
            extensions,
            readers: HashMap::new(),
        }
    }

    /// Registers `reader` for every file named exactly `file_name` (for example `scenarios.md`),
    /// found anywhere under a run's features root. `reader` wins over the built-in `.feature` and
    /// `.feature.md` suffix rules for that exact file name: discovery finds a matching file the
    /// same way it finds a `.feature` file, and loading calls `reader` instead of
    /// [`morphir_gherkin::read_document`] for it. A later call for the same `file_name` replaces
    /// an earlier one.
    #[must_use]
    pub fn with_reader(mut self, file_name: &str, reader: Reader) -> Self {
        self.readers.insert(file_name.to_owned(), reader);
        self
    }
}

impl<I: AsRef<Path>> cucumber::Parser<I> for MorphirParser {
    type Cli = cucumber::cli::Empty;
    type Output = stream::Iter<std::vec::IntoIter<parser::Result<gherkin::Feature>>>;

    fn parse(self, input: I, _cli: Self::Cli) -> Self::Output {
        let (paths, errors) = discover(input.as_ref(), &self.readers);
        let mut features: Vec<_> = paths
            .into_iter()
            .map(|path| load(&path, &self.readers))
            .collect();
        features.extend(errors.into_iter().map(|message| Err(parse_error(message))));
        stream::iter(features)
    }
}

/// Walks `root` (a file, or a directory searched recursively) for `.feature` and `.feature.md`
/// documents, plus any file whose name exactly matches a key of `readers`, in sorted path order.
/// A directory that cannot be listed, or an entry whose metadata cannot be read, is not skipped in
/// silence: it is collected as an error message instead, naming the path and the underlying I/O
/// error, so the caller can turn it into a parsing error rather than let the subtree it would have
/// held drop unnoticed.
fn discover(root: &Path, readers: &HashMap<String, Reader>) -> (Vec<PathBuf>, Vec<String>) {
    if root.is_file() {
        return (vec![root.to_owned()], Vec::new());
    }
    let mut found = Vec::new();
    let mut errors = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                errors.push(format!(
                    "the directory {} cannot be read: {e}",
                    dir.display()
                ));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    errors.push(format!("an entry in {} cannot be read: {e}", dir.display()));
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(e) => {
                    errors.push(format!("{} cannot be read: {e}", path.display()));
                    continue;
                }
            };
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if metadata.is_dir() {
                stack.push(path);
            } else if name.ends_with(".feature")
                || name.ends_with(".feature.md")
                || readers.contains_key(name)
            {
                found.push(path);
            }
        }
    }
    found.sort();
    (found, errors)
}

/// Wraps `message` as a cucumber parsing error whose `Display` carries that message.
///
/// cucumber 0.23 prints a [`parser::Error::Parsing`] through `gherkin::ParseFileError`'s own
/// `Display`, which shows only the path (`Could not read path: {path}`) and never its source.
/// Neither `ParseFileError` variant has a free-text field, and the one other error variant
/// (`ExampleExpansion`) would print the message as an unresolved `<placeholder>`. So the message
/// travels in the `path` field itself: `message` (which already names the file, line and column,
/// as [`morphir_gherkin::ReadError`] does) becomes the path, and the console, JSON and JUnit
/// reports all print it.
fn parse_error(message: String) -> parser::Error {
    parser::Error::Parsing(Arc::new(gherkin::ParseFileError::Reading {
        path: PathBuf::from(&message),
        source: std::io::Error::other(message),
    }))
}

/// Reads `path` into a [`Document`]: through the reader registered for its exact file name, if
/// any, else through [`morphir_gherkin::read_document`]. Either way, a read failure becomes a
/// parsing error through [`parse_error`]; for a custom reader, that error's text carries both
/// `path` and the reader's own message, since a bare `Err(message)` names no file on its own.
fn read_one(path: &Path, readers: &HashMap<String, Reader>) -> parser::Result<Document> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    match readers.get(name) {
        Some(reader) => {
            reader(path).map_err(|message| parse_error(format!("{}: {message}", path.display())))
        }
        None => morphir_gherkin::read_document(path)
            .map(|(document, _)| document)
            .map_err(|error| parse_error(error.to_string())),
    }
}

fn load(path: &Path, readers: &HashMap<String, Reader>) -> parser::Result<gherkin::Feature> {
    let document = read_one(path, readers)?;
    let document = Arc::new(document);
    let feature = document
        .feature
        .as_ref()
        .ok_or_else(|| parse_error(format!("{}: no Feature heading", path.display())))?;
    let root = NodePath::feature();
    let feature_slots = scenario_slots(&root, &feature.scenarios);
    let rule_slots: Vec<_> = feature
        .rules
        .iter()
        .enumerate()
        .map(|(r, rule)| scenario_slots(&root.push(Segment::Rule(r)), &rule.scenarios))
        .collect();
    let lowered = lower_feature(path, feature, document.format);
    // Expanding a Scenario Outline's Examples is the parser's job, not the runner's: `cucumber`'s
    // own `parser::Basic` does it here too, in `parser::basic::Basic::parse`.
    let expanded = lowered
        .expand_examples()
        .map_err(|error| parser::Error::ExampleExpansion(Arc::new(error)))?;
    register_expanded(path, &document, &feature_slots, &expanded.scenarios);
    for (slots, rule) in rule_slots.iter().zip(&expanded.rules) {
        register_expanded(path, &document, slots, &rule.scenarios);
    }
    Ok(expanded)
}

/// The model paths the scenarios expand to, each with how many expanded scenarios it covers, in
/// the order `gherkin::Feature::expand_examples` produces them: a plain scenario is one slot of
/// 1 at its scenario path; an outline is one slot per `Examples` block, at that block's
/// `…/scenario[i]/examples[j]` path, covering the block's data rows. cucumber expands block by
/// block and row by row, and skips a block with no table, which is a slot of 0 here.
fn scenario_slots(
    root: &NodePath,
    scenarios: &[morphir_gherkin::Scenario],
) -> Vec<(NodePath, usize)> {
    scenarios
        .iter()
        .enumerate()
        .flat_map(|(i, s)| {
            let scenario = root.push(Segment::Scenario(i));
            if s.examples.is_empty() {
                vec![(scenario, 1)]
            } else {
                s.examples
                    .iter()
                    .enumerate()
                    .map(|(j, e)| {
                        let rows = e
                            .table
                            .as_ref()
                            .map_or(0, |t| t.rows.len().saturating_sub(1));
                        (scenario.push(Segment::Examples(j)), rows)
                    })
                    .collect()
            }
        })
        .collect()
}

/// Registers every scenario cucumber actually produced, by its real expanded name and line: never
/// predicted, always read back from `expand_examples`'s own output. `slots` and `expanded` line up
/// because `expand_examples` keeps each scenario's block and row order and never reorders
/// scenarios past one another; each slot claims exactly the `count` expanded scenarios that follow
/// the ones before it. An outline's row registers with its own examples block's path, so the
/// context built for it sees only that block's tags.
fn register_expanded(
    path: &Path,
    document: &Arc<Document>,
    slots: &[(NodePath, usize)],
    expanded: &[gherkin::Scenario],
) {
    let mut at = 0;
    for (node, count) in slots {
        // A slot is plain (a `Scenario` with no `Examples` block of its own) when its path ends
        // in `Segment::Scenario`; an outline row's slot ends in `Segment::Examples`.
        let plain = matches!(node.last(), Segment::Scenario(_));
        for (k, scenario) in expanded[at..at + count].iter().enumerate() {
            register(
                path,
                &scenario.name,
                scenario.position.line,
                document,
                node.clone(),
                if plain { None } else { Some(k) },
            );
        }
        at += count;
    }
}

fn register(
    path: &Path,
    name: &str,
    line: usize,
    document: &Arc<Document>,
    node: NodePath,
    row: Option<usize>,
) {
    REGISTRY.lock().expect("registry").insert(
        (path.to_owned(), name.to_owned(), line),
        (document.clone(), node, row),
    );
}

fn position(p: morphir_gherkin::LineCol) -> gherkin::LineCol {
    gherkin::LineCol {
        line: p.line,
        col: p.col,
    }
}

fn span(s: morphir_gherkin::Span) -> gherkin::Span {
    gherkin::Span {
        start: s.start,
        end: s.end,
    }
}

fn tags(tags: &[morphir_gherkin::Tag]) -> Vec<String> {
    tags.iter().map(|t| t.name.clone()).collect()
}

fn lower_steps(steps: &[morphir_gherkin::Step]) -> Vec<gherkin::Step> {
    steps
        .iter()
        .map(|s| gherkin::Step {
            keyword: s.keyword.clone(),
            ty: match s.kind {
                StepKind::Given => gherkin::StepType::Given,
                StepKind::When => gherkin::StepType::When,
                StepKind::Then => gherkin::StepType::Then,
            },
            value: s.text.clone(),
            docstring: match &s.argument {
                Some(StepArgument::DocString(d)) => Some(d.body.clone()),
                _ => None,
            },
            table: match &s.argument {
                Some(StepArgument::Table(t)) => Some(gherkin::Table {
                    rows: t.rows.clone(),
                    span: span(t.span),
                    position: position(t.position),
                }),
                _ => None,
            },
            span: span(s.span),
            position: position(s.position),
        })
        .collect()
}

/// The line cucumber should see for an `Examples` block, so that its row formula (`Examples`
/// line + row index + 2, in `cucumber::feature::Ext::expand_examples`) names each data row's real
/// source line in the reports.
///
/// In a `.feature` file the table starts on the line after the keyword, so the keyword's own
/// line is right and is kept. In a `.feature.md` file a blank line (and maybe tags or prose)
/// comes between the heading and the table, and a separator row comes after the header, so the
/// first data row is at the header's line + 2: the line to report is the header's line, which is
/// the first data row's line − 2.
fn examples_line(e: &morphir_gherkin::Examples, format: Format) -> usize {
    match (format, &e.table) {
        (Format::Markdown, Some(table)) => table.position.line,
        _ => e.position.line,
    }
}

fn lower_scenario(s: &morphir_gherkin::Scenario, format: Format) -> gherkin::Scenario {
    gherkin::Scenario {
        keyword: s.keyword.clone(),
        name: s.name.clone(),
        description: None,
        steps: lower_steps(&s.steps),
        examples: s
            .examples
            .iter()
            .map(|e| gherkin::Examples {
                keyword: e.keyword.clone(),
                name: e.name.clone(),
                description: None,
                table: e.table.as_ref().map(|t| gherkin::Table {
                    rows: t.rows.clone(),
                    span: span(t.span),
                    position: position(t.position),
                }),
                tags: tags(&e.tags),
                span: span(e.span),
                position: gherkin::LineCol {
                    line: examples_line(e, format),
                    col: e.position.col,
                },
            })
            .collect(),
        tags: tags(&s.tags),
        span: span(s.span),
        position: position(s.position),
    }
}

fn lower_background(b: &morphir_gherkin::Background) -> gherkin::Background {
    gherkin::Background {
        keyword: b.keyword.clone(),
        name: b.name.clone(),
        description: None,
        steps: lower_steps(&b.steps),
        span: span(b.span),
        position: position(b.position),
    }
}

fn lower_feature(path: &Path, f: &morphir_gherkin::Feature, format: Format) -> gherkin::Feature {
    gherkin::Feature {
        keyword: f.keyword.clone(),
        name: f.name.clone(),
        description: None,
        background: f.background.as_ref().map(lower_background),
        scenarios: f
            .scenarios
            .iter()
            .map(|s| lower_scenario(s, format))
            .collect(),
        rules: f
            .rules
            .iter()
            .map(|rule| gherkin::Rule {
                keyword: rule.keyword.clone(),
                name: rule.name.clone(),
                description: None,
                background: rule.background.as_ref().map(lower_background),
                scenarios: rule
                    .scenarios
                    .iter()
                    .map(|s| lower_scenario(s, format))
                    .collect(),
                tags: tags(&rule.tags),
                span: span(rule.span),
                position: position(rule.position),
            })
            .collect(),
        tags: tags(&f.tags),
        span: span(f.span),
        position: position(f.position),
        path: Some(path.to_owned()),
    }
}

fn lookup(
    feature: &gherkin::Feature,
    scenario: &gherkin::Scenario,
) -> Option<(Arc<Document>, NodePath, Option<usize>)> {
    let path = feature.path.clone()?;
    REGISTRY
        .lock()
        .expect("registry")
        .get(&(path, scenario.name.clone(), scenario.position.line))
        .cloned()
}

/// Fills the world before the first step: the running scenario and the context its extensions build.
///
/// For one row of a scenario outline, the context comes from the row's own `Examples` block, not
/// from the outline's other blocks, and [`ScenarioRef::path`] is that block's path.
pub fn prepare(
    world: &mut MorphirWorld,
    feature: &gherkin::Feature,
    scenario: &gherkin::Scenario,
    extensions: &Extensions,
) -> Result<(), String> {
    let (document, path, row) = lookup(feature, scenario)
        .ok_or_else(|| format!("no model node for scenario `{}`", scenario.name))?;
    let (context, _) = extensions.context_for(&document, &path).map_err(|errors| {
        errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    world.context = context;
    world.scenario = Some(ScenarioRef {
        document,
        path,
        row,
    });
    Ok(())
}

/// Why a scenario is skipped, decided from its static tags before it starts. For one row of a
/// scenario outline, only the tags of the row's own `Examples` block count, so `@wip` on one block
/// skips only that block's rows.
pub fn skip_reason(
    feature: &gherkin::Feature,
    _rule: Option<&gherkin::Rule>,
    scenario: &gherkin::Scenario,
    extensions: &Extensions,
) -> Option<String> {
    let (document, path, _row) = lookup(feature, scenario)?;
    match extensions.context_for(&document, &path) {
        Ok((_, Effect::Skip(reason))) => Some(reason),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{REGISTRY, load};

    /// The lines cucumber gives the expanded rows of the first outline in `text`, read as `name`.
    fn row_lines(name: &str, text: &str) -> Vec<usize> {
        let dir = tempfile::tempdir().expect("create a temporary directory");
        let path = dir.path().join(name);
        std::fs::write(&path, text).expect("write the document");
        let feature = load(&path, &HashMap::new()).expect("the document reads");
        feature.scenarios.iter().map(|s| s.position.line).collect()
    }

    #[test]
    fn a_plain_feature_outline_row_keeps_cucumbers_own_line() {
        let text = "Feature: F\n  Scenario Outline: o <x>\n    Given a\n    Examples:\n      | x |\n      | 1 |\n      | 2 |\n";
        assert_eq!(row_lines("f.feature", text), vec![6, 7]);
    }

    #[test]
    fn a_markdown_outline_row_reports_its_own_data_row_line() {
        let text = "# Feature: F\n\n## Scenario Outline: o <x>\n\n* Given a\n\n### Examples: E\n\n`@t`\n\n| x |\n| - |\n| 1 |\n| 2 |\n";
        assert_eq!(row_lines("f.feature.md", text), vec![13, 14]);
    }

    /// The registered `row` of every scenario `load` produces for `text`, in the order
    /// `expand_examples` gives them.
    fn registered_rows(name: &str, text: &str) -> Vec<Option<usize>> {
        let dir = tempfile::tempdir().expect("create a temporary directory");
        let path = dir.path().join(name);
        std::fs::write(&path, text).expect("write the document");
        let feature = load(&path, &HashMap::new()).expect("the document reads");
        let registry = REGISTRY.lock().expect("registry");
        feature
            .scenarios
            .iter()
            .map(|s| {
                registry
                    .get(&(path.clone(), s.name.clone(), s.position.line))
                    .expect("the scenario is registered")
                    .2
            })
            .collect()
    }

    #[test]
    fn an_outline_row_registers_its_0_based_row_within_its_own_examples_block() {
        let text = "Feature: F\n  Scenario Outline: o <x>\n    Given a\n\n    Examples: first\n      | x |\n      | 1 |\n      | 2 |\n\n    Examples: second\n      | x |\n      | 3 |\n      | 4 |\n      | 5 |\n";
        assert_eq!(
            registered_rows("rows.feature", text),
            vec![Some(0), Some(1), Some(0), Some(1), Some(2)]
        );
    }

    #[test]
    fn a_plain_scenario_registers_with_no_row() {
        let text = "Feature: F\n  Scenario: s\n    Given a\n";
        assert_eq!(registered_rows("plain.feature", text), vec![None]);
    }
}
