//! The document model. Every node carries its span and the position of its first character.

use std::path::PathBuf;

use crate::span::{LineCol, Span};

/// The format a `Document` was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Plain Gherkin, from a `.feature` file.
    Feature,
    /// Markdown with Gherkin, from a `.feature.md` file.
    Markdown,
}

/// A `.feature` or `.feature.md` file, read into one model.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// The path the document was read from.
    pub path: PathBuf,
    /// The format the document was read from.
    pub format: Format,
    /// The Markdown before the `Feature` heading of a `.feature.md` file. Without a `Feature`
    /// heading, it is the Markdown before the first Gherkin heading, or all of it when the file
    /// has no Gherkin heading. A `.feature` file has an empty preamble.
    pub preamble: Description,
    /// The document's feature, or `None` for a file with no Gherkin structure (a comment-only
    /// `.feature` file, or a `.feature.md` file with no Gherkin heading at all).
    pub feature: Option<Feature>,
}

/// A `@name` tag on a feature, a rule, a scenario or an examples block.
#[derive(Debug, Clone, PartialEq)]
pub struct Tag {
    /// The tag without its `@`, for example `syntax:elm`.
    pub name: String,
    /// The span of the tag, including its `@`.
    pub span: Span,
    /// The position of the tag's `@`.
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
    /// The description's blocks, in source order.
    pub blocks: Vec<DescriptionBlock>,
}

/// One block of a description: prose, or a fence that is not a step's doc string.
#[derive(Debug, Clone, PartialEq)]
pub enum DescriptionBlock {
    /// A Markdown block that is not a fence.
    Prose(ProseBlock),
    /// A fenced code block.
    Fence(Fence),
}

impl Description {
    /// The description's fences, in source order.
    pub fn fences(&self) -> impl Iterator<Item = &Fence> {
        self.blocks.iter().filter_map(|block| match block {
            DescriptionBlock::Fence(fence) => Some(fence),
            DescriptionBlock::Prose(_) => None,
        })
    }

    /// The description's prose blocks, in source order.
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
    /// What kind of Markdown block this is.
    pub kind: ProseKind,
    /// The block's source text, including its original indent.
    pub markdown: String,
    /// The block's parsed inline content, in source order.
    pub inlines: Vec<Inline>,
    /// The block's span.
    pub span: Span,
    /// The position of the block's first character.
    pub position: LineCol,
}

/// The kind of Markdown block a `ProseBlock` is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProseKind {
    /// A plain paragraph.
    Paragraph,
    /// A bullet or ordered list.
    List,
    /// A block quote.
    Quote,
    /// A heading, with its level (`1` for `#`, up to `6` for `######`).
    Heading(u8),
    /// A Markdown table.
    Table,
    /// A thematic break (`---`).
    Rule,
}

/// A parsed inline Markdown node.
#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    /// Plain text.
    Text(String),
    /// An inline code span.
    Code(String),
    /// Emphasized (`*italic*`) content.
    Emphasis(Vec<Inline>),
    /// Strong (`**bold**`) content.
    Strong(Vec<Inline>),
    /// A link.
    Link {
        /// The link's destination URL.
        destination: String,
        /// The link's inline content.
        content: Vec<Inline>,
    },
}

/// A fenced block that is not a step's doc string.
#[derive(Debug, Clone, PartialEq)]
pub struct Fence {
    /// The fence's info string, parsed.
    pub info: FenceInfo,
    /// The body exactly as written, with the fence's own indent removed.
    pub body: String,
    /// The fence's span, from its opening delimiter to the end of its closing delimiter's line.
    pub span: Span,
    /// The position of the fence's opening delimiter.
    pub position: LineCol,
}

/// A fence info string: `<language> [key=value …]`, with bare words kept in `words`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FenceInfo {
    /// The first word of the info string, taken as the fence's language.
    pub language: String,
    /// The bare words of the info string after the language, in source order.
    pub words: Vec<String>,
    /// The `key=value` pairs of the info string, in source order.
    pub options: Vec<(String, String)>,
    /// The info string exactly as written, trimmed, with the original order of `language`,
    /// `words` and `options` kept. A writer that needs the fence's original info text back,
    /// rather than one it reassembles, uses this field.
    pub raw: String,
}

impl FenceInfo {
    /// Parses a fence info string into its language, bare words and `key=value` options.
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
            raw: info.trim().to_owned(),
        }
    }
}

/// A Gherkin `Feature`.
#[derive(Debug, Clone, PartialEq)]
pub struct Feature {
    /// The keyword as written, for example `Feature` or `Ability`. Empty for the implicit
    /// feature of a `.feature.md` file with no `Feature` heading.
    pub keyword: String,
    /// The feature's name. Empty for an implicit feature.
    pub name: String,
    /// The feature's own tags, in source order.
    pub tags: Vec<Tag>,
    /// The Markdown before the feature's first child.
    pub description: Description,
    /// The feature's background, if it has one.
    pub background: Option<Background>,
    /// The feature's rules, in source order.
    pub rules: Vec<Rule>,
    /// The feature's scenarios that are not under a rule, in source order.
    pub scenarios: Vec<Scenario>,
    /// The feature's span, from its first tag or comment lead-in to the end of its last child.
    pub span: Span,
    /// The position of the feature's keyword, or of its first child for an implicit feature.
    pub position: LineCol,
}

/// A Gherkin `Rule`.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// The keyword as written, always `Rule`.
    pub keyword: String,
    /// The rule's name.
    pub name: String,
    /// The rule's own tags, in source order.
    pub tags: Vec<Tag>,
    /// The Markdown before the rule's first child.
    pub description: Description,
    /// The rule's background, if it has one.
    pub background: Option<Background>,
    /// The rule's scenarios, in source order.
    pub scenarios: Vec<Scenario>,
    /// The rule's span.
    pub span: Span,
    /// The position of the rule's keyword.
    pub position: LineCol,
}

/// A Gherkin `Background`.
#[derive(Debug, Clone, PartialEq)]
pub struct Background {
    /// The keyword as written, always `Background`.
    pub keyword: String,
    /// The background's name, if it has one.
    pub name: String,
    /// The Markdown before the background's first step.
    pub description: Description,
    /// The background's steps, in source order.
    pub steps: Vec<Step>,
    /// The background's span.
    pub span: Span,
    /// The position of the background's keyword.
    pub position: LineCol,
}

/// A Gherkin `Scenario`, `Example`, `Scenario Outline` or `Scenario Template`.
#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    /// `Scenario`, `Example`, `Scenario Outline` or `Scenario Template`, as written.
    pub keyword: String,
    /// The scenario's name.
    pub name: String,
    /// The scenario's own tags, in source order.
    pub tags: Vec<Tag>,
    /// The Markdown before the scenario's first step.
    pub description: Description,
    /// The scenario's steps, in source order.
    pub steps: Vec<Step>,
    /// The scenario's `Examples` blocks, in source order. Empty for a plain `Scenario`.
    pub examples: Vec<Examples>,
    /// The scenario's span.
    pub span: Span,
    /// The position of the scenario's keyword.
    pub position: LineCol,
}

/// A Gherkin `Examples` (or `Scenarios`) block under a scenario outline.
#[derive(Debug, Clone, PartialEq)]
pub struct Examples {
    /// The keyword as written, `Examples` or `Scenarios`.
    pub keyword: String,
    /// The examples block's name, if it has one.
    pub name: Option<String>,
    /// The examples block's own tags, in source order.
    pub tags: Vec<Tag>,
    /// The Markdown before the examples table.
    pub description: Description,
    /// The examples table, if the block has one.
    pub table: Option<Table>,
    /// The Markdown after the examples table in a `.feature.md` file, up to the next Gherkin
    /// heading. A `.feature` file gives empty notes.
    pub notes: Description,
    /// The examples block's span.
    pub span: Span,
    /// The position of the examples block's keyword.
    pub position: LineCol,
}

/// Which kind of step a `Step` is, once `And` and `But` are resolved to the step before them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    /// A `Given` step.
    Given,
    /// A `When` step.
    When,
    /// A `Then` step.
    Then,
}

/// One step of a scenario or background.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// The keyword as written, for example `And `.
    pub keyword: String,
    /// The kind the keyword resolves to (`And` and `But` take the previous kind).
    pub kind: StepKind,
    /// The step's text, after its keyword.
    pub text: String,
    /// The step's doc string or table, if it has one.
    pub argument: Option<StepArgument>,
    /// The Markdown after the step in a `.feature.md` file, up to the next step or Gherkin
    /// heading. A `.feature` step has no notes.
    pub notes: Description,
    /// The step's span, covering its own line only (not its argument or notes).
    pub span: Span,
    /// The position of the step's keyword.
    pub position: LineCol,
}

/// A step's argument: a doc string or a data table.
#[derive(Debug, Clone, PartialEq)]
pub enum StepArgument {
    /// A `"""` or ` ``` ` doc string.
    DocString(DocString),
    /// A `|`-delimited data table.
    Table(Table),
}

/// A step's `"""` or ` ``` ` doc string argument.
#[derive(Debug, Clone, PartialEq)]
pub struct DocString {
    /// The content type after the opening delimiter (`ion` in `"""ion`), if any.
    pub content_type: Option<String>,
    /// The body with the delimiter's indent removed from each line.
    pub body: String,
    /// The doc string's span, from its opening delimiter to the end of its closing delimiter's
    /// line.
    pub span: Span,
    /// The position of the doc string's opening delimiter.
    pub position: LineCol,
}

/// A `|`-delimited data table: a step's argument, or an examples block's rows.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// The table's rows, each a list of cell values in column order. A step's table has no
    /// header row of its own; an examples table's first row is its header.
    pub rows: Vec<Vec<String>>,
    /// The table's span.
    pub span: Span,
    /// The position of the table's first cell.
    pub position: LineCol,
}
