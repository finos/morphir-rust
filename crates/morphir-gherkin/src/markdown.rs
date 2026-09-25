//! Markdown blocks inside a description, with spans into the original file.
//!
//! A description's text sits at some indent inside its `.feature` or `.feature.md` file. This
//! module strips that indent, parses the result as Markdown, and maps every block's range back to
//! the original file. A block's span always starts at the beginning of its original source line, so
//! it keeps the block's own leading indentation.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::model::{
    Description, DescriptionBlock, Fence, FenceInfo, Inline, ProseBlock, ProseKind,
};
use crate::span::{SourceText, Span};

/// The smallest leading-space count of the non-blank lines in `range`.
pub fn common_indent(source: &SourceText, range: Span) -> usize {
    source
        .slice(range)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start_matches(' ').len())
        .min()
        .unwrap_or(0)
}

/// Text with up to `indent` leading spaces removed from each line, and the original offset of each
/// byte of it (plus one entry for the end of the range).
struct Dedented {
    text: String,
    origin: Vec<usize>,
}

fn dedent(source: &SourceText, range: Span, indent: usize) -> Dedented {
    let mut text = String::new();
    let mut origin = Vec::new();
    let mut offset = range.start;
    for line in source.slice(range).split_inclusive('\n') {
        let removable = line.len() - line.trim_start_matches(' ').len();
        let skip = removable.min(indent);
        for (i, kept_char) in line[skip..].char_indices() {
            let start = offset + skip + i;
            for k in 0..kept_char.len_utf8() {
                origin.push(start + k);
            }
            text.push(kept_char);
        }
        offset += line.len();
    }
    origin.push(range.end);
    Dedented { text, origin }
}

/// Parses the lines in `range` as Markdown after removing up to `indent` leading spaces from each
/// line, and returns the description with every block's span pointing into the original file. A
/// block's span starts at the start of its source line, but never before `range.start`.
pub fn parse_blocks(source: &SourceText, range: Span, indent: usize) -> Description {
    let dedented = dedent(source, range, indent);
    let mut builder = Builder::new(source, range, &dedented);
    for (event, local) in Parser::new_ext(&dedented.text, Options::ENABLE_TABLES).into_offset_iter()
    {
        builder.handle(event, local);
    }
    Description {
        blocks: builder.blocks,
    }
}

/// A block-level element open at the top level: a paragraph, list, quote, heading, table or rule.
struct BlockFrame {
    kind: ProseKind,
    local: Range<usize>,
    inlines: Vec<Inline>,
}

/// A fenced code block open at the top level.
struct FenceFrame {
    info: String,
    local: Range<usize>,
    body: String,
}

/// An inline element (emphasis, strong or link) open inside the current block.
struct InlineFrame {
    open: InlineOpen,
    children: Vec<Inline>,
}

enum InlineOpen {
    Emphasis,
    Strong,
    Link(String),
}

impl InlineOpen {
    fn finish(self, children: Vec<Inline>) -> Inline {
        match self {
            InlineOpen::Emphasis => Inline::Emphasis(children),
            InlineOpen::Strong => Inline::Strong(children),
            InlineOpen::Link(destination) => Inline::Link {
                destination,
                content: children,
            },
        }
    }
}

/// Walks the Markdown event stream, tracking nesting depth so that only top-level blocks and
/// top-level fences become `DescriptionBlock`s. A fence nested inside a list or quote stays part
/// of that block's inline content, not a free fence.
struct Builder<'s> {
    source: &'s SourceText,
    range: Span,
    dedented: &'s Dedented,
    blocks: Vec<DescriptionBlock>,
    depth: usize,
    block: Option<BlockFrame>,
    fence: Option<FenceFrame>,
    inlines: Vec<InlineFrame>,
}

impl<'s> Builder<'s> {
    fn new(source: &'s SourceText, range: Span, dedented: &'s Dedented) -> Self {
        Self {
            source,
            range,
            dedented,
            blocks: Vec::new(),
            depth: 0,
            block: None,
            fence: None,
            inlines: Vec::new(),
        }
    }

    /// The span of a local range in the dedented text, mapped back to the original file. The span
    /// starts at the beginning of its original source line, so it keeps that line's indent, but
    /// never starts before `self.range.start`: a block whose first source line starts before
    /// `range` (because `range` itself begins mid-line) keeps its span, and `markdown`, inside
    /// `range`.
    fn span(&self, local: Range<usize>) -> Span {
        let start = self.dedented.origin[local.start];
        let end = self.dedented.origin[local.end.min(self.dedented.origin.len() - 1)];
        let start = self
            .source
            .line_start(self.source.line_col(start).line)
            .max(self.range.start);
        Span { start, end }
    }

    fn push_inline(&mut self, node: Inline) {
        if let Some(frame) = self.inlines.last_mut() {
            frame.children.push(node);
        } else if let Some(block) = self.block.as_mut() {
            block.inlines.push(node);
        }
    }

    fn handle(&mut self, event: Event<'_>, local: Range<usize>) {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) if self.depth == 0 => {
                self.fence = Some(FenceFrame {
                    info: info.to_string(),
                    local,
                    body: String::new(),
                });
                self.depth += 1;
            }
            Event::End(TagEnd::CodeBlock) if self.depth == 1 && self.fence.is_some() => {
                let frame = self.fence.take().expect("checked by the guard above");
                self.finish_fence(frame);
                self.depth -= 1;
            }
            Event::Text(text) if self.fence.is_some() => {
                self.fence
                    .as_mut()
                    .expect("checked by the guard above")
                    .body
                    .push_str(&text);
            }
            Event::Start(tag) => {
                if self.depth == 0 {
                    self.block = Some(BlockFrame {
                        kind: prose_kind(&tag),
                        local,
                        inlines: Vec::new(),
                    });
                }
                if let Some(open) = inline_open(&tag) {
                    self.inlines.push(InlineFrame {
                        open,
                        children: Vec::new(),
                    });
                }
                self.depth += 1;
            }
            Event::End(end) => {
                if is_inline_end(&end)
                    && let Some(frame) = self.inlines.pop()
                {
                    let node = frame.open.finish(frame.children);
                    self.push_inline(node);
                }
                self.depth -= 1;
                if self.depth == 0
                    && let Some(frame) = self.block.take()
                {
                    self.finish_block(frame);
                }
            }
            Event::Text(text) => self.push_inline(Inline::Text(text.to_string())),
            Event::Code(code) => self.push_inline(Inline::Code(code.to_string())),
            Event::SoftBreak | Event::HardBreak => self.push_inline(Inline::Text("\n".to_owned())),
            Event::Rule if self.depth == 0 => self.finish_block(BlockFrame {
                kind: ProseKind::Rule,
                local,
                inlines: Vec::new(),
            }),
            _ => {}
        }
    }

    fn finish_block(&mut self, frame: BlockFrame) {
        let span = self.span(frame.local);
        self.blocks.push(DescriptionBlock::Prose(ProseBlock {
            kind: frame.kind,
            markdown: self.source.slice(span).to_owned(),
            inlines: frame.inlines,
            span,
            position: self.source.line_col(span.start),
        }));
    }

    fn finish_fence(&mut self, frame: FenceFrame) {
        let span = self.span(frame.local);
        self.blocks.push(DescriptionBlock::Fence(Fence {
            info: FenceInfo::parse(&frame.info),
            body: frame.body,
            span,
            position: self.source.line_col(span.start),
        }));
    }
}

fn inline_open(tag: &Tag<'_>) -> Option<InlineOpen> {
    match tag {
        Tag::Emphasis => Some(InlineOpen::Emphasis),
        Tag::Strong => Some(InlineOpen::Strong),
        Tag::Link { dest_url, .. } => Some(InlineOpen::Link(dest_url.to_string())),
        _ => None,
    }
}

fn is_inline_end(end: &TagEnd) -> bool {
    matches!(end, TagEnd::Emphasis | TagEnd::Strong | TagEnd::Link)
}

fn prose_kind(tag: &Tag<'_>) -> ProseKind {
    match tag {
        Tag::List(_) => ProseKind::List,
        Tag::BlockQuote(_) => ProseKind::Quote,
        Tag::Heading { level, .. } => ProseKind::Heading(*level as u8),
        Tag::Table(_) => ProseKind::Table,
        _ => ProseKind::Paragraph,
    }
}
