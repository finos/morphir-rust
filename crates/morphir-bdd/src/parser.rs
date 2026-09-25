//! Feeds morphir-gherkin documents to cucumber-rs and finds each running scenario's model node.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use cucumber::feature::Ext as _;
use cucumber::{gherkin, parser};
use futures::stream;
use morphir_gherkin::extension::{Effect, Extensions};
use morphir_gherkin::{Document, NodePath, Segment, StepArgument, StepKind};

use crate::world::{MorphirWorld, ScenarioRef};

type Key = (PathBuf, String, usize);
type ModelNode = (Arc<Document>, NodePath);

static REGISTRY: LazyLock<Mutex<HashMap<Key, ModelNode>>> = LazyLock::new(Default::default);

pub struct MorphirParser {
    #[allow(dead_code)]
    extensions: Arc<Extensions>,
}

impl MorphirParser {
    pub fn new(extensions: Arc<Extensions>) -> Self {
        Self { extensions }
    }
}

impl<I: AsRef<Path>> cucumber::Parser<I> for MorphirParser {
    type Cli = cucumber::cli::Empty;
    type Output = stream::Iter<std::vec::IntoIter<parser::Result<gherkin::Feature>>>;

    fn parse(self, input: I, _cli: Self::Cli) -> Self::Output {
        let features: Vec<_> = discover(input.as_ref())
            .into_iter()
            .map(|path| load(&path))
            .collect();
        stream::iter(features)
    }
}

fn discover(root: &Path) -> Vec<PathBuf> {
    if root.is_file() {
        return vec![root.to_owned()];
    }
    let mut found = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if path.is_dir() {
                stack.push(path);
            } else if name.ends_with(".feature") || name.ends_with(".feature.md") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn load(path: &Path) -> parser::Result<gherkin::Feature> {
    let (document, _) = morphir_gherkin::read_document(path).map_err(|error| {
        parser::Error::Parsing(Arc::new(gherkin::ParseFileError::Reading {
            path: path.to_owned(),
            source: std::io::Error::other(error.to_string()),
        }))
    })?;
    let document = Arc::new(document);
    let feature = document.feature.as_ref().ok_or_else(|| {
        parser::Error::Parsing(Arc::new(gherkin::ParseFileError::Reading {
            path: path.to_owned(),
            source: std::io::Error::other("no Feature heading"),
        }))
    })?;
    let lowered = lower_feature(path, feature, &document);
    // Expanding a Scenario Outline's Examples is the parser's job, not the runner's: `cucumber`'s
    // own `parser::Basic` does it here too, in `parser::basic::Basic::parse`.
    lowered
        .expand_examples()
        .map_err(|error| parser::Error::ExampleExpansion(Arc::new(error)))
}

fn register(path: &Path, name: &str, line: usize, document: &Arc<Document>, node: NodePath) {
    REGISTRY.lock().expect("registry").insert(
        (path.to_owned(), name.to_owned(), line),
        (document.clone(), node),
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

/// Registers every line an expanded outline row might land on. `cucumber` expands an outline into
/// one scenario per examples row and sets that scenario's `position` to the *lowered* examples
/// block's position, plus the row's 0-based index, plus 2 (as if the row sat two lines below an
/// `Examples:` keyword, header included) — see `cucumber::feature::expand_scenario` in
/// cucumber 0.23. It never looks at the source file again, so this mirrors that formula against
/// the same lowered position we hand it, rather than the row's real line in a `.feature.md` file.
fn register_outline_rows(
    path: &Path,
    s: &morphir_gherkin::Scenario,
    document: &Arc<Document>,
    node: &NodePath,
) {
    for example in &s.examples {
        let Some(table) = &example.table else {
            continue;
        };
        let data_rows = table.rows.len().saturating_sub(1);
        for row in 0..data_rows {
            let row_line = example.position.line + row + 2;
            register(path, &s.name, row_line, document, node.clone());
        }
    }
}

fn lower_scenario(
    path: &Path,
    s: &morphir_gherkin::Scenario,
    document: &Arc<Document>,
    node: NodePath,
) -> gherkin::Scenario {
    register(path, &s.name, s.position.line, document, node.clone());
    register_outline_rows(path, s, document, &node);
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
                position: position(e.position),
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

fn lower_feature(
    path: &Path,
    f: &morphir_gherkin::Feature,
    document: &Arc<Document>,
) -> gherkin::Feature {
    let root = NodePath::feature();
    gherkin::Feature {
        keyword: f.keyword.clone(),
        name: f.name.clone(),
        description: None,
        background: f.background.as_ref().map(lower_background),
        scenarios: f
            .scenarios
            .iter()
            .enumerate()
            .map(|(i, s)| lower_scenario(path, s, document, root.push(Segment::Scenario(i))))
            .collect(),
        rules: f
            .rules
            .iter()
            .enumerate()
            .map(|(r, rule)| gherkin::Rule {
                keyword: rule.keyword.clone(),
                name: rule.name.clone(),
                description: None,
                background: rule.background.as_ref().map(lower_background),
                scenarios: rule
                    .scenarios
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        lower_scenario(
                            path,
                            s,
                            document,
                            root.push(Segment::Rule(r)).push(Segment::Scenario(i)),
                        )
                    })
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
) -> Option<(Arc<Document>, NodePath)> {
    let path = feature.path.clone()?;
    REGISTRY
        .lock()
        .expect("registry")
        .get(&(path, scenario.name.clone(), scenario.position.line))
        .cloned()
}

/// Fills the world before the first step: the running scenario and the context its extensions build.
pub fn prepare(
    world: &mut MorphirWorld,
    feature: &gherkin::Feature,
    scenario: &gherkin::Scenario,
    extensions: &Extensions,
) -> Result<(), String> {
    let (document, path) = lookup(feature, scenario)
        .ok_or_else(|| format!("no model node for scenario `{}`", scenario.name))?;
    let (context, _) = extensions.context_for(&document, &path).map_err(|errors| {
        errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    world.context = context;
    world.scenario = Some(ScenarioRef { document, path });
    Ok(())
}

/// Why a scenario is skipped, decided from its static tags before it starts.
pub fn skip_reason(
    feature: &gherkin::Feature,
    _rule: Option<&gherkin::Rule>,
    scenario: &gherkin::Scenario,
    extensions: &Extensions,
) -> Option<String> {
    let (document, path) = lookup(feature, scenario)?;
    match extensions.context_for(&document, &path) {
        Ok((_, Effect::Skip(reason))) => Some(reason),
        _ => None,
    }
}
