//! Reads a `.feature.md` file (Markdown with Gherkin).
//!
//! The reader makes two passes. The first pass parses the file as Markdown and lists its
//! top-level blocks: headings, bullet lists, tables, fences, tag lines and other blocks. The
//! second pass walks those blocks and builds the same model that the `.feature` reader gives.
//!
//! The rules follow Cucumber's Markdown with Gherkin:
//!
//! - A heading whose text starts with a Gherkin keyword and `:` opens that node. The heading level
//!   does not matter.
//! - A bullet list item (`*` or `-`) whose first line starts with a step keyword is a step.
//! - A table or a fenced block inside a step's list item is the step's argument. A table or a fence
//!   right after a step list is the argument of the last step, if that step has none.
//! - A table right after an `Examples` heading is the examples table.
//! - A paragraph that holds only inline code spans that start with `@` is a tag line. A `#`
//!   comment can follow the tags on a line. A tag line that is the first block after a heading
//!   belongs to that heading. There is one exception: a tag line on the line right above a
//!   Gherkin heading, with no blank line between them, leads that heading. Other tag lines right
//!   before a Gherkin heading lead that heading too.
//! - Any other Markdown between a heading and its first child is the node's description.
//!
//! Keywords, names, step text and table cells come from the source text, so `<x>` placeholders
//! stay as written.

use std::path::Path;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag as MdTag, TagEnd};

use crate::error::ReadError;
use crate::markdown::parse_blocks;
use crate::model::{
    Background, Description, DocString, Document, Examples, Feature, Format, Rule, Scenario, Step,
    StepArgument, StepKind, Table, Tag,
};
use crate::span::{SourceText, Span};

/// Reads the Markdown with Gherkin text in `source`. A file without a `Feature` heading has no
/// feature.
pub fn read(path: &Path, source: &SourceText) -> Result<Document, ReadError> {
    let blocks = Scanner::scan(source);
    let mut builder = Builder {
        path,
        source,
        blocks: &blocks,
        at: 0,
    };
    let feature = builder.feature()?;
    Ok(Document {
        path: path.to_owned(),
        format: Format::Markdown,
        feature,
    })
}

// ---------------------------------------------------------------------------------------------
// The first pass: top-level Markdown blocks.
// ---------------------------------------------------------------------------------------------

/// A top-level Markdown block.
#[derive(Debug)]
enum Block {
    /// A heading. `text` is the heading's source text without its `#` marks.
    Heading {
        text: String,
        span: Span,
    },
    /// A bullet list.
    List {
        items: Vec<Item>,
        span: Span,
    },
    Table(RawTable),
    Fence(RawFence),
    /// A paragraph of `@` code spans. Each tag has its name without `@` and the span of `@name`.
    TagLine {
        tags: Vec<(String, Span)>,
        span: Span,
    },
    /// Any other block.
    Other {
        span: Span,
    },
}

impl Block {
    fn span(&self) -> Span {
        match self {
            Block::Heading { span, .. }
            | Block::List { span, .. }
            | Block::TagLine { span, .. }
            | Block::Other { span } => *span,
            Block::Table(table) => table.span,
            Block::Fence(fence) => fence.span,
        }
    }
}

/// A list item: the step on its first line, if any, and the first table or fence in it.
#[derive(Debug)]
struct Item {
    span: Span,
    step: Option<StepLine>,
    argument: Option<RawArgument>,
}

/// The parts of a step line.
#[derive(Debug)]
struct StepLine {
    keyword: &'static str,
    /// `None` for `And`, `But` and `*`, which take the kind of the step before.
    kind: Option<StepKind>,
    text: String,
}

#[derive(Debug)]
enum RawArgument {
    Table(RawTable),
    Fence(RawFence),
}

/// A table's rows. Markdown gives no event for the delimiter row, so the rows hold only data.
#[derive(Debug)]
struct RawTable {
    rows: Vec<Vec<String>>,
    span: Span,
}

/// A fenced block. `body` has the fence's indent removed.
#[derive(Debug)]
struct RawFence {
    info: String,
    body: String,
    span: Span,
}

/// A top-level block that is still open.
enum Open {
    Heading { span: Span, content: Option<Span> },
    Paragraph { span: Span, scan: TagScan },
    List { span: Span, items: Vec<Item> },
    Table,
    Fence,
    Other { span: Span },
}

/// Checks whether a paragraph is a tag line while its inline events arrive.
#[derive(Default)]
struct TagScan {
    tags: Vec<(String, Span)>,
    not_tags: bool,
    line_has_tag: bool,
    in_comment: bool,
}

impl TagScan {
    fn code(&mut self, source: &SourceText, code: &str, span: Span) {
        if self.in_comment {
            return;
        }
        // Upstream reads `` `@a``@b` `` as two tags. Markdown gives it as one code span.
        let parts: Vec<&str> = code.split('`').filter(|part| !part.is_empty()).collect();
        let is_tag = |part: &&str| {
            part.len() > 1 && part.starts_with('@') && !part.contains(char::is_whitespace)
        };
        if parts.is_empty() || !parts.iter().all(is_tag) {
            self.not_tags = true;
            return;
        }
        let raw = source.slice(span);
        let mut from = 0;
        for part in parts {
            let Some(at) = raw[from..].find(part) else {
                self.not_tags = true;
                return;
            };
            let start = span.start + from + at;
            let tag_span = Span {
                start,
                end: start + part.len(),
            };
            self.tags.push((part[1..].to_owned(), tag_span));
            from += at + part.len();
        }
        self.line_has_tag = true;
    }

    fn text(&mut self, text: &str) {
        let text = text.trim();
        if self.in_comment || text.is_empty() {
            return;
        }
        if text.starts_with('#') && self.line_has_tag {
            self.in_comment = true;
        } else {
            self.not_tags = true;
        }
    }

    fn line_break(&mut self) {
        self.in_comment = false;
        self.line_has_tag = false;
    }

    fn into_tags(self) -> Option<Vec<(String, Span)>> {
        (!self.not_tags && !self.tags.is_empty()).then_some(self.tags)
    }
}

/// A table being read, at the top level or inside a list item.
struct TableFrame {
    rows: Vec<Vec<String>>,
    row: Vec<String>,
    span: Span,
}

/// Walks the Markdown events and collects the top-level blocks.
struct Scanner<'s> {
    source: &'s SourceText,
    blocks: Vec<Block>,
    depth: usize,
    open: Option<Open>,
    item: Option<Item>,
    table: Option<TableFrame>,
    fence: Option<RawFence>,
}

impl<'s> Scanner<'s> {
    fn scan(source: &'s SourceText) -> Vec<Block> {
        let mut scanner = Scanner {
            source,
            blocks: Vec::new(),
            depth: 0,
            open: None,
            item: None,
            table: None,
            fence: None,
        };
        let parser = Parser::new_ext(source.text(), Options::ENABLE_TABLES).into_offset_iter();
        for (event, range) in parser {
            let span = Span {
                start: range.start,
                end: range.end,
            };
            match event {
                Event::Start(tag) => {
                    scanner.start(&tag, span);
                    scanner.depth += 1;
                }
                Event::End(end) => {
                    scanner.depth -= 1;
                    scanner.end(end, span);
                }
                other => scanner.inline(&other, span),
            }
        }
        scanner.blocks
    }

    /// Handles the start of an element at level `self.depth`. Level 0 is a top-level block, level
    /// 1 is a list item, and level 2 is a block inside a list item.
    fn start(&mut self, tag: &MdTag<'_>, span: Span) {
        let level = self.depth;
        if level == 0 {
            self.open = Some(match tag {
                MdTag::Heading { .. } => Open::Heading {
                    span,
                    content: None,
                },
                MdTag::Paragraph => Open::Paragraph {
                    span,
                    scan: TagScan::default(),
                },
                MdTag::List(None) => Open::List {
                    span,
                    items: Vec::new(),
                },
                MdTag::Table(_) => {
                    self.table = Some(TableFrame::new(span));
                    Open::Table
                }
                MdTag::CodeBlock(CodeBlockKind::Fenced(info)) => {
                    self.fence = Some(RawFence::new(info, span));
                    Open::Fence
                }
                _ => Open::Other { span },
            });
            return;
        }
        if level == 1 && matches!(tag, MdTag::Item) && matches!(self.open, Some(Open::List { .. }))
        {
            self.item = Some(Item {
                span,
                step: step_line(self.source, span),
                argument: None,
            });
            return;
        }
        let free_item = self
            .item
            .as_ref()
            .is_some_and(|item| item.argument.is_none())
            && self.table.is_none()
            && self.fence.is_none();
        if level == 2 && free_item {
            match tag {
                MdTag::Table(_) => self.table = Some(TableFrame::new(span)),
                MdTag::CodeBlock(CodeBlockKind::Fenced(info)) => {
                    self.fence = Some(RawFence::new(info, span));
                }
                _ => {}
            }
        }
        match tag {
            MdTag::TableHead | MdTag::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.row.clear();
                }
            }
            MdTag::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    table.row.push(cell_text(self.source.slice(span)));
                }
            }
            _ => self.inline_span(span, true),
        }
    }

    /// Handles the end of an element at level `self.depth`.
    fn end(&mut self, end: TagEnd, span: Span) {
        let level = self.depth;
        match end {
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    let row = std::mem::take(&mut table.row);
                    table.rows.push(row);
                }
            }
            TagEnd::Table if level == 2 => {
                if let (Some(table), Some(item)) = (self.table.take(), self.item.as_mut()) {
                    item.argument = Some(RawArgument::Table(table.finish()));
                }
            }
            TagEnd::CodeBlock if level == 2 => {
                if let (Some(fence), Some(item)) = (self.fence.take(), self.item.as_mut()) {
                    item.argument = Some(RawArgument::Fence(fence));
                }
            }
            TagEnd::Item if level == 1 => {
                if let (Some(item), Some(Open::List { items, .. })) =
                    (self.item.take(), self.open.as_mut())
                {
                    items.push(item);
                }
            }
            _ if level == 0 => self.finish(),
            _ => self.inline_span(span, true),
        }
    }

    /// Closes the open top-level block and adds it to the list.
    fn finish(&mut self) {
        let block = match self.open.take() {
            Some(Open::Heading { span, content }) => Block::Heading {
                text: content
                    .map_or_else(String::new, |content| heading_text(self.source, content)),
                span,
            },
            Some(Open::Paragraph { span, scan }) => match scan.into_tags() {
                Some(tags) => Block::TagLine { tags, span },
                None => Block::Other { span },
            },
            Some(Open::List { span, items }) => Block::List { items, span },
            Some(Open::Table) => match self.table.take() {
                Some(table) => Block::Table(table.finish()),
                None => return,
            },
            Some(Open::Fence) => match self.fence.take() {
                Some(fence) => Block::Fence(fence),
                None => return,
            },
            Some(Open::Other { span }) => Block::Other { span },
            None => return,
        };
        self.blocks.push(block);
    }

    /// Handles an event that neither starts nor ends an element.
    fn inline(&mut self, event: &Event<'_>, span: Span) {
        if let (Event::Text(text), Some(fence)) = (event, self.fence.as_mut()) {
            fence.body.push_str(text);
            return;
        }
        if self.depth == 0 {
            if let Event::Rule = event {
                self.blocks.push(Block::Other { span });
            }
            return;
        }
        let direct = self.depth == 1;
        if let (true, Some(Open::Paragraph { scan, .. })) = (direct, self.open.as_mut()) {
            match event {
                Event::Code(code) => scan.code(self.source, code, span),
                Event::Text(text) => scan.text(text),
                Event::SoftBreak | Event::HardBreak => scan.line_break(),
                _ => scan.not_tags = true,
            }
            return;
        }
        self.inline_span(span, false);
    }

    /// Notes the range of inline content: it widens a heading's text, and an element inside a
    /// paragraph other than a code span means the paragraph is not a tag line.
    fn inline_span(&mut self, span: Span, element: bool) {
        match self.open.as_mut() {
            Some(Open::Heading { content, .. }) => {
                *content = Some(match content {
                    Some(seen) => Span {
                        start: seen.start.min(span.start),
                        end: seen.end.max(span.end),
                    },
                    None => span,
                });
            }
            Some(Open::Paragraph { scan, .. }) if element => scan.not_tags = true,
            _ => {}
        }
    }
}

impl TableFrame {
    fn new(span: Span) -> Self {
        Self {
            rows: Vec::new(),
            row: Vec::new(),
            span,
        }
    }

    fn finish(self) -> RawTable {
        RawTable {
            rows: self.rows,
            span: self.span,
        }
    }
}

impl RawFence {
    fn new(info: &str, span: Span) -> Self {
        Self {
            info: info.to_owned(),
            body: String::new(),
            span,
        }
    }
}

/// A table cell as written, without its padding. `\|` stands for `|`.
fn cell_text(raw: &str) -> String {
    raw.trim().replace("\\|", "|")
}

/// A heading's text: its source content with each line trimmed and the lines joined by a space.
fn heading_text(source: &SourceText, content: Span) -> String {
    source
        .slice(content)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

const STEP_KEYWORDS: [(&str, Option<StepKind>); 6] = [
    ("Given ", Some(StepKind::Given)),
    ("When ", Some(StepKind::When)),
    ("Then ", Some(StepKind::Then)),
    ("And ", None),
    ("But ", None),
    ("* ", None),
];

/// The step on the first line of a list item, if the item has a `*` or `-` marker and its text
/// starts with a step keyword. The item's span starts at its marker.
fn step_line(source: &SourceText, item: Span) -> Option<StepLine> {
    let line = source.text()[item.start..].lines().next()?;
    let rest = line.trim_start().strip_prefix(['*', '-'])?;
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    let rest = rest.trim_start();
    STEP_KEYWORDS.iter().find_map(|&(keyword, kind)| {
        rest.strip_prefix(keyword).map(|text| StepLine {
            keyword,
            kind,
            text: text.trim().to_owned(),
        })
    })
}

// ---------------------------------------------------------------------------------------------
// The second pass: the model.
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeKind {
    Feature,
    Rule,
    Background,
    Scenario,
    Examples,
}

/// The Gherkin keywords a heading can start with, as in the English dialect of Gherkin.
const NODE_KEYWORDS: [(&str, NodeKind); 11] = [
    ("Feature", NodeKind::Feature),
    ("Business Need", NodeKind::Feature),
    ("Ability", NodeKind::Feature),
    ("Rule", NodeKind::Rule),
    ("Background", NodeKind::Background),
    ("Scenario", NodeKind::Scenario),
    ("Example", NodeKind::Scenario),
    ("Scenario Outline", NodeKind::Scenario),
    ("Scenario Template", NodeKind::Scenario),
    ("Examples", NodeKind::Examples),
    ("Scenarios", NodeKind::Examples),
];

/// A heading that opens a Gherkin node.
struct NodeHeading {
    kind: NodeKind,
    keyword: &'static str,
    name: String,
    span: Span,
}

fn node_heading(block: &Block) -> Option<NodeHeading> {
    let Block::Heading { text, span } = block else {
        return None;
    };
    let (keyword, name) = text.split_once(':')?;
    let keyword = keyword.trim();
    let &(keyword, kind) = NODE_KEYWORDS.iter().find(|(k, _)| *k == keyword)?;
    Some(NodeHeading {
        kind,
        keyword,
        name: name.trim().to_owned(),
        span: *span,
    })
}

struct Builder<'a> {
    path: &'a Path,
    source: &'a SourceText,
    blocks: &'a [Block],
    at: usize,
}

impl<'a> Builder<'a> {
    fn peek(&self) -> Option<&'a Block> {
        self.blocks.get(self.at)
    }

    /// The end of the last block read so far.
    fn read_end(&self) -> usize {
        self.at
            .checked_sub(1)
            .and_then(|last| self.blocks.get(last))
            .map_or(0, |block| block.span().end)
    }

    fn syntax_error(&self, span: Span, message: &str) -> ReadError {
        ReadError::Syntax {
            path: self.path.to_owned(),
            position: self.source.line_col(span.start),
            message: message.to_owned(),
        }
    }

    fn tag(&self, name: &str, span: Span) -> Tag {
        Tag {
            name: name.to_owned(),
            span,
            position: self.source.line_col(span.start),
        }
    }

    /// The tags of the tag lines in `blocks[range]`.
    fn tags_in(&self, range: std::ops::Range<usize>) -> Vec<Tag> {
        self.blocks[range]
            .iter()
            .filter_map(|block| match block {
                Block::TagLine { tags, .. } => Some(tags),
                _ => None,
            })
            .flatten()
            .map(|(name, span)| self.tag(name, *span))
            .collect()
    }

    /// The index after the run of tag lines that starts at `index`.
    fn after_tag_lines(&self, index: usize) -> usize {
        let run = self.blocks[index.min(self.blocks.len())..]
            .iter()
            .take_while(|block| matches!(block, Block::TagLine { .. }))
            .count();
        index + run
    }

    /// Whether the block at `index` starts a run of tag lines right before a Gherkin heading.
    fn leads_heading(&self, index: usize) -> bool {
        let after = self.after_tag_lines(index);
        after > index && self.blocks.get(after).and_then(node_heading).is_some()
    }

    /// Whether the block at `index` ends on the line right above a Gherkin heading, with no blank
    /// line between them. Upstream Markdown with Gherkin writes tags this way.
    fn sits_right_above_heading(&self, index: usize) -> bool {
        let (Some(block), Some(next)) = (self.blocks.get(index), self.blocks.get(index + 1)) else {
            return false;
        };
        if node_heading(next).is_none() {
            return false;
        }
        let span = block.span();
        let last_line = self.source.line_col(span.end.max(span.start + 1) - 1).line;
        self.source.line_col(next.span().start).line == last_line + 1
    }

    /// The tags of a run of tag lines at the reading position that leads a Gherkin heading.
    fn leading_tags(&mut self) -> Vec<Tag> {
        if !self.leads_heading(self.at) {
            return Vec::new();
        }
        let after = self.after_tag_lines(self.at);
        let tags = self.tags_in(self.at..after);
        self.at = after;
        tags
    }

    fn feature(&mut self) -> Result<Option<Feature>, ReadError> {
        let Some((index, heading)) = self.blocks.iter().enumerate().find_map(|(index, block)| {
            node_heading(block)
                .filter(|heading| heading.kind == NodeKind::Feature)
                .map(|heading| (index, heading))
        }) else {
            return Ok(None);
        };
        let first_tag_line = (0..index)
            .rev()
            .take_while(|&i| matches!(self.blocks[i], Block::TagLine { .. }))
            .last()
            .unwrap_or(index);
        let mut tags = self.tags_in(first_tag_line..index);
        self.at = index + 1;
        let (own_tags, description) = self.header(true, false);
        tags.extend(own_tags);
        let mut feature = Feature {
            keyword: heading.keyword.to_owned(),
            name: heading.name,
            tags,
            description,
            background: None,
            rules: Vec::new(),
            scenarios: Vec::new(),
            span: heading.span,
            position: self.source.line_col(heading.span.start),
        };
        loop {
            let leading = self.leading_tags();
            let Some(block) = self.peek() else { break };
            self.at += 1;
            let Some(heading) = node_heading(block) else {
                continue;
            };
            match heading.kind {
                NodeKind::Feature => {
                    return Err(self.syntax_error(
                        heading.span,
                        "a document can hold only one Feature heading",
                    ));
                }
                NodeKind::Background => {
                    let background = self.background(heading);
                    match feature.rules.last_mut() {
                        Some(rule) => rule.background = Some(background),
                        None => feature.background = Some(background),
                    }
                }
                NodeKind::Rule => {
                    let rule = self.rule(heading, leading);
                    feature.rules.push(rule);
                }
                NodeKind::Scenario => {
                    let scenario = self.scenario(heading, leading);
                    match feature.rules.last_mut() {
                        Some(rule) => rule.scenarios.push(scenario),
                        None => feature.scenarios.push(scenario),
                    }
                }
                NodeKind::Examples => {
                    let examples = self.examples(heading, leading);
                    let scenarios = match feature.rules.last_mut() {
                        Some(rule) => &mut rule.scenarios,
                        None => &mut feature.scenarios,
                    };
                    let Some(scenario) = scenarios.last_mut() else {
                        return Err(self.syntax_error(
                            examples.span,
                            "an Examples heading must follow a Scenario Outline",
                        ));
                    };
                    scenario.span.end = examples.span.end;
                    scenario.examples.push(examples);
                }
            }
            if let Some(rule) = feature.rules.last_mut() {
                rule.span.end = self.read_end();
            }
        }
        feature.span.end = self.read_end();
        Ok(Some(feature))
    }

    /// Reads what follows a heading up to its first child: the heading's own tag line when
    /// `own_tags` is true, then the description. The description stops at a Gherkin heading, a
    /// step list, a tag line that leads a Gherkin heading, and a table when `stop_at_table` is
    /// true.
    fn header(&mut self, own_tags: bool, stop_at_table: bool) -> (Vec<Tag>, Description) {
        let mut tags = Vec::new();
        if own_tags
            && matches!(self.peek(), Some(Block::TagLine { .. }))
            && !self.sits_right_above_heading(self.at)
        {
            tags = self.tags_in(self.at..self.at + 1);
            self.at += 1;
        }
        let first = self.at;
        while let Some(block) = self.peek() {
            let ends = match block {
                Block::Heading { .. } => node_heading(block).is_some(),
                Block::List { items, .. } => items.iter().any(|item| item.step.is_some()),
                Block::Table(_) => stop_at_table,
                Block::TagLine { .. } => self.leads_heading(self.at),
                Block::Fence(_) | Block::Other { .. } => false,
            };
            if ends {
                break;
            }
            self.at += 1;
        }
        let description = if self.at > first {
            let start = self.blocks[first].span().start;
            let start = self.source.line_start(self.source.line_col(start).line);
            parse_blocks(
                self.source,
                Span {
                    start,
                    end: self.read_end(),
                },
                0,
            )
        } else {
            Description::default()
        };
        (tags, description)
    }

    fn rule(&mut self, heading: NodeHeading, mut tags: Vec<Tag>) -> Rule {
        let (own_tags, description) = self.header(true, false);
        tags.extend(own_tags);
        Rule {
            keyword: heading.keyword.to_owned(),
            name: heading.name,
            tags,
            description,
            background: None,
            scenarios: Vec::new(),
            span: Span {
                start: heading.span.start,
                end: self.read_end(),
            },
            position: self.source.line_col(heading.span.start),
        }
    }

    /// A background has no tags, so a tag line after its heading stays in its description.
    fn background(&mut self, heading: NodeHeading) -> Background {
        let (_, description) = self.header(false, false);
        let steps = self.steps();
        Background {
            keyword: heading.keyword.to_owned(),
            name: heading.name,
            description,
            steps,
            span: Span {
                start: heading.span.start,
                end: self.read_end(),
            },
            position: self.source.line_col(heading.span.start),
        }
    }

    fn scenario(&mut self, heading: NodeHeading, mut tags: Vec<Tag>) -> Scenario {
        let (own_tags, description) = self.header(true, false);
        tags.extend(own_tags);
        let steps = self.steps();
        Scenario {
            keyword: heading.keyword.to_owned(),
            name: heading.name,
            tags,
            description,
            steps,
            examples: Vec::new(),
            span: Span {
                start: heading.span.start,
                end: self.read_end(),
            },
            position: self.source.line_col(heading.span.start),
        }
    }

    fn examples(&mut self, heading: NodeHeading, mut tags: Vec<Tag>) -> Examples {
        let (own_tags, description) = self.header(true, true);
        tags.extend(own_tags);
        let table = match self.peek() {
            Some(Block::Table(table)) => {
                self.at += 1;
                Some(self.table(table))
            }
            _ => None,
        };
        Examples {
            keyword: heading.keyword.to_owned(),
            name: (!heading.name.is_empty()).then_some(heading.name),
            tags,
            description,
            table,
            span: Span {
                start: heading.span.start,
                end: self.read_end(),
            },
            position: self.source.line_col(heading.span.start),
        }
    }

    /// The steps of the step lists up to the next Gherkin heading or the tag lines that lead it.
    /// Other blocks between the lists are skipped. A table or fence right after a step list is
    /// the last step's argument, if that step has none.
    fn steps(&mut self) -> Vec<Step> {
        let mut steps: Vec<Step> = Vec::new();
        let mut previous = StepKind::Given;
        let mut after_steps = false;
        while let Some(block) = self.peek() {
            let argument = match block {
                Block::Heading { .. } if node_heading(block).is_some() => break,
                Block::TagLine { .. } if self.leads_heading(self.at) => break,
                Block::List { items, .. } => {
                    let count = steps.len();
                    for item in items {
                        let Some(line) = &item.step else { continue };
                        let kind = line.kind.unwrap_or(previous);
                        previous = kind;
                        steps.push(Step {
                            keyword: line.keyword.to_owned(),
                            kind,
                            text: line.text.clone(),
                            argument: item.argument.as_ref().map(|raw| self.argument(raw)),
                            span: item.span,
                            position: self.source.line_col(item.span.start),
                        });
                    }
                    self.at += 1;
                    after_steps = steps.len() > count;
                    continue;
                }
                Block::Table(table) if after_steps => Some(StepArgument::Table(self.table(table))),
                Block::Fence(fence) if after_steps => {
                    Some(StepArgument::DocString(self.doc_string(fence)))
                }
                _ => None,
            };
            if let (Some(argument), Some(step)) = (argument, steps.last_mut())
                && step.argument.is_none()
            {
                step.argument = Some(argument);
            }
            after_steps = false;
            self.at += 1;
        }
        steps
    }

    fn argument(&self, raw: &RawArgument) -> StepArgument {
        match raw {
            RawArgument::Table(table) => StepArgument::Table(self.table(table)),
            RawArgument::Fence(fence) => StepArgument::DocString(self.doc_string(fence)),
        }
    }

    fn table(&self, table: &RawTable) -> Table {
        Table {
            rows: table.rows.clone(),
            span: table.span,
            position: self.source.line_col(table.span.start),
        }
    }

    /// A fence as a doc string. The content type is the fence's info string, as the `.feature`
    /// reader takes the text after the opening delimiter. The span runs from the start of the
    /// opening line to the end of the closing line. The position is the opening fence's column.
    fn doc_string(&self, fence: &RawFence) -> DocString {
        let text = self.source.text();
        let start = self
            .source
            .line_start(self.source.line_col(fence.span.start).line);
        let mut end = fence.span.end;
        if text[end..].starts_with("\r\n") {
            end += 2;
        } else if text[end..].starts_with('\n') {
            end += 1;
        }
        let info = fence.info.trim();
        DocString {
            content_type: (!info.is_empty()).then(|| info.to_owned()),
            body: fence.body.clone(),
            span: Span { start, end },
            position: self.source.line_col(fence.span.start),
        }
    }
}
