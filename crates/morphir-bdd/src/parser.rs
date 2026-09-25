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
    let root = NodePath::feature();
    let feature_slots = scenario_slots(&root, &feature.scenarios);
    let rule_slots: Vec<_> = feature
        .rules
        .iter()
        .enumerate()
        .map(|(r, rule)| scenario_slots(&root.push(Segment::Rule(r)), &rule.scenarios))
        .collect();
    let lowered = lower_feature(path, feature);
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

/// How many expanded scenarios a scenario becomes, and the model path that describes it: 1 for a
/// plain scenario, or the sum of every `Examples` block's data rows for an outline — in the same
/// block-then-row order `gherkin::Feature::expand_examples` walks them in.
fn scenario_slots(
    root: &NodePath,
    scenarios: &[morphir_gherkin::Scenario],
) -> Vec<(NodePath, usize)> {
    scenarios
        .iter()
        .enumerate()
        .map(|(i, s)| (root.push(Segment::Scenario(i)), expanded_count(s)))
        .collect()
}

fn expanded_count(s: &morphir_gherkin::Scenario) -> usize {
    if s.examples.is_empty() {
        1
    } else {
        s.examples
            .iter()
            .map(|e| {
                e.table
                    .as_ref()
                    .map_or(0, |t| t.rows.len().saturating_sub(1))
            })
            .sum()
    }
}

/// Registers every scenario cucumber actually produced, by its real expanded name and line: never
/// predicted, always read back from `expand_examples`'s own output. `slots` and `expanded` line up
/// because `expand_examples` keeps each scenario's row order and never reorders scenarios past one
/// another; each slot claims exactly the `count` expanded scenarios that follow the ones before it.
fn register_expanded(
    path: &Path,
    document: &Arc<Document>,
    slots: &[(NodePath, usize)],
    expanded: &[gherkin::Scenario],
) {
    let mut at = 0;
    for (node, count) in slots {
        for scenario in &expanded[at..at + count] {
            register(
                path,
                &scenario.name,
                scenario.position.line,
                document,
                node.clone(),
            );
        }
        at += count;
    }
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

fn lower_scenario(s: &morphir_gherkin::Scenario) -> gherkin::Scenario {
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

fn lower_feature(path: &Path, f: &morphir_gherkin::Feature) -> gherkin::Feature {
    gherkin::Feature {
        keyword: f.keyword.clone(),
        name: f.name.clone(),
        description: None,
        background: f.background.as_ref().map(lower_background),
        scenarios: f.scenarios.iter().map(lower_scenario).collect(),
        rules: f
            .rules
            .iter()
            .map(|rule| gherkin::Rule {
                keyword: rule.keyword.clone(),
                name: rule.name.clone(),
                description: None,
                background: rule.background.as_ref().map(lower_background),
                scenarios: rule.scenarios.iter().map(lower_scenario).collect(),
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
