//! The document model. Every node carries its span and the position of its first character.

use std::path::PathBuf;

use crate::span::{LineCol, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Feature,
    Markdown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub path: PathBuf,
    pub format: Format,
    /// The Markdown before the `Feature` heading of a `.feature.md` file, or all of it when the
    /// file has no `Feature` heading. A `.feature` file has an empty preamble.
    pub preamble: Description,
    pub feature: Option<Feature>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tag {
    /// The tag without its `@`, for example `syntax:elm`.
    pub name: String,
    pub span: Span,
    pub position: LineCol,
}

impl Tag {
    /// The namespace and value of a namespaced tag: `syntax:elm` gives `("syntax", "elm")`.
    pub fn namespaced(&self) -> Option<(&str, &str)> {
        self.name.split_once(':')
    }
}

/// A description: prose and free fences, in source order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Description {
    pub blocks: Vec<DescriptionBlock>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DescriptionBlock {
    Prose(ProseBlock),
    Fence(Fence),
}

impl Description {
    pub fn fences(&self) -> impl Iterator<Item = &Fence> {
        self.blocks.iter().filter_map(|block| match block {
            DescriptionBlock::Fence(fence) => Some(fence),
            DescriptionBlock::Prose(_) => None,
        })
    }

    pub fn prose(&self) -> impl Iterator<Item = &ProseBlock> {
        self.blocks.iter().filter_map(|block| match block {
            DescriptionBlock::Prose(prose) => Some(prose),
            DescriptionBlock::Fence(_) => None,
        })
    }
}

/// One Markdown block that is not a fence: a paragraph, a list, a quote, a heading below the Gherkin
/// levels, a table or a thematic break. `markdown` is the block's source text, including its
/// indent; `inlines` holds its parsed inline content.
#[derive(Debug, Clone, PartialEq)]
pub struct ProseBlock {
    pub kind: ProseKind,
    pub markdown: String,
    pub inlines: Vec<Inline>,
    pub span: Span,
    pub position: LineCol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProseKind {
    Paragraph,
    List,
    Quote,
    Heading(u8),
    Table,
    Rule,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Code(String),
    Emphasis(Vec<Inline>),
    Strong(Vec<Inline>),
    Link {
        destination: String,
        content: Vec<Inline>,
    },
}

/// A fenced block that is not a step's doc string.
#[derive(Debug, Clone, PartialEq)]
pub struct Fence {
    pub info: FenceInfo,
    /// The body exactly as written, with the fence's own indent removed.
    pub body: String,
    pub span: Span,
    pub position: LineCol,
}

/// A fence info string: `<language> [key=value …]`, with bare words kept in `words`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FenceInfo {
    pub language: String,
    pub words: Vec<String>,
    pub options: Vec<(String, String)>,
}

impl FenceInfo {
    pub fn parse(info: &str) -> Self {
        let mut parts = info.split_whitespace();
        let language = parts.next().unwrap_or_default().to_owned();
        let (mut words, mut options) = (Vec::new(), Vec::new());
        for part in parts {
            match part.split_once('=') {
                Some((key, value)) => options.push((key.to_owned(), value.to_owned())),
                None => words.push(part.to_owned()),
            }
        }
        Self {
            language,
            words,
            options,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Feature {
    pub keyword: String,
    pub name: String,
    pub tags: Vec<Tag>,
    pub description: Description,
    pub background: Option<Background>,
    pub rules: Vec<Rule>,
    pub scenarios: Vec<Scenario>,
    pub span: Span,
    pub position: LineCol,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub keyword: String,
    pub name: String,
    pub tags: Vec<Tag>,
    pub description: Description,
    pub background: Option<Background>,
    pub scenarios: Vec<Scenario>,
    pub span: Span,
    pub position: LineCol,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Background {
    pub keyword: String,
    pub name: String,
    pub description: Description,
    pub steps: Vec<Step>,
    pub span: Span,
    pub position: LineCol,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    /// `Scenario`, `Example`, `Scenario Outline` or `Scenario Template`, as written.
    pub keyword: String,
    pub name: String,
    pub tags: Vec<Tag>,
    pub description: Description,
    pub steps: Vec<Step>,
    pub examples: Vec<Examples>,
    pub span: Span,
    pub position: LineCol,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Examples {
    pub keyword: String,
    pub name: Option<String>,
    pub tags: Vec<Tag>,
    pub description: Description,
    pub table: Option<Table>,
    /// The Markdown after the examples table in a `.feature.md` file, up to the next Gherkin
    /// heading. A `.feature` file gives empty notes.
    pub notes: Description,
    pub span: Span,
    pub position: LineCol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Given,
    When,
    Then,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// The keyword as written, for example `And `.
    pub keyword: String,
    /// The kind the keyword resolves to (`And` and `But` take the previous kind).
    pub kind: StepKind,
    pub text: String,
    pub argument: Option<StepArgument>,
    /// The Markdown after the step in a `.feature.md` file, up to the next step or Gherkin
    /// heading. A `.feature` step has no notes.
    pub notes: Description,
    pub span: Span,
    pub position: LineCol,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StepArgument {
    DocString(DocString),
    Table(Table),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DocString {
    /// The content type after the opening delimiter (`ion` in `"""ion`), if any.
    pub content_type: Option<String>,
    /// The body with the delimiter's indent removed from each line.
    pub body: String,
    pub span: Span,
    pub position: LineCol,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub rows: Vec<Vec<String>>,
    pub span: Span,
    pub position: LineCol,
}
