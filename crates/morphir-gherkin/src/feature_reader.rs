//! Reads a `.feature` file.
//!
//! `gherkin` 0.16 gives the structure: keywords, names, tags, steps and table rows. The reader
//! reads descriptions and doc strings again from the source, because `gherkin` 0.16 trims each
//! description line, drops blank lines, and keeps a doc string's content type and indent in its
//! text. Tag spans also come from the source, because `gherkin` 0.16 gives only tag names.

use std::path::Path;

use crate::error::ReadError;
use crate::markdown::{common_indent, parse_blocks};
use crate::model::{
    Background, Description, DocString, Document, Examples, Feature, Format, Rule, Scenario, Step,
    StepArgument, StepKind, Table, Tag,
};
use crate::span::{LineCol, SourceText, Span};

/// Reads the `.feature` text in `source`. A file with only blank lines and comments has no
/// feature.
pub fn read(path: &Path, source: &SourceText) -> Result<Document, ReadError> {
    let reader = Reader { source };
    let feature = if (1..=source.line_count()).all(|line| reader.is_blank_or_comment(line)) {
        None
    } else {
        let parsed = gherkin::Feature::parse(source.text(), gherkin::GherkinEnv::default())
            .map_err(|error| syntax_error(path, &error))?;
        Some(reader.feature(&parsed))
    };
    Ok(Document {
        path: path.to_owned(),
        format: Format::Feature,
        preamble: Description::default(),
        feature,
    })
}

/// Turns a `gherkin` parse error into a `ReadError`. `gherkin` 0.16 keeps the error position
/// private, so the reader takes it from the message, which starts with `Error at <line>:<col>: `.
fn syntax_error(path: &Path, error: &gherkin::ParseError) -> ReadError {
    let text = error.to_string();
    let parsed = text.strip_prefix("Error at ").and_then(|rest| {
        let (line, rest) = rest.split_once(':')?;
        let (col, expected) = rest.split_once(':')?;
        let position = LineCol {
            line: line.parse().ok()?,
            col: col.parse().ok()?,
        };
        Some((position, expected.trim()))
    });
    let (position, message) = match parsed {
        Some((position, expected)) => (position, expected_message(expected)),
        None => (LineCol { line: 1, col: 1 }, text.clone()),
    };
    ReadError::Syntax {
        path: path.to_owned(),
        position,
        message,
    }
}

/// The message for a set of expected tokens, written by `gherkin` as `{"a", "b"}` in no fixed
/// order. The reader sorts the tokens so that the message is stable.
fn expected_message(expected: &str) -> String {
    let inner = expected
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .unwrap_or(expected);
    let mut tokens: Vec<&str> = inner
        .split(", ")
        .filter(|token| !token.is_empty())
        .collect();
    tokens.sort_unstable();
    if tokens.is_empty() {
        "not valid Gherkin".to_owned()
    } else {
        format!("not valid Gherkin, expected one of {}", tokens.join(", "))
    }
}

struct Reader<'s> {
    source: &'s SourceText,
}

impl Reader<'_> {
    /// A `gherkin` span as a span into the source. `gherkin` adds a final newline when the text
    /// has none, so an offset can be one past the end of the source.
    fn span(&self, span: gherkin::Span) -> Span {
        let len = self.source.text().len();
        Span {
            start: span.start.min(len),
            end: span.end.min(len),
        }
    }

    /// The line and column of a byte offset. The column counts characters.
    fn position(&self, offset: usize) -> LineCol {
        self.source.line_col(offset.min(self.source.text().len()))
    }

    /// The line where a `gherkin` node starts.
    fn header_line(&self, span: gherkin::Span) -> usize {
        self.position(span.start).line
    }

    /// The span from the start of `line` to the start of `end_line`. Both lines are 1-based, and
    /// a line after the last line stands for the end of the text.
    fn lines(&self, line: usize, end_line: usize) -> Span {
        let at = |line: usize| {
            if line > self.source.line_count() {
                self.source.text().len()
            } else {
                self.source.line_start(line)
            }
        };
        Span {
            start: at(line),
            end: at(end_line),
        }
    }

    /// The text of one line, with its line break.
    fn line(&self, line: usize) -> &str {
        self.source.slice(self.lines(line, line + 1))
    }

    fn is_blank_or_comment(&self, line: usize) -> bool {
        let text = self.line(line).trim();
        text.is_empty() || text.starts_with('#')
    }

    /// The first line after a node. A node's span ends after its last line, or after the indent
    /// of the next line.
    fn end_line(&self, end: usize) -> usize {
        let end = end.min(self.source.text().len());
        let position = self.position(end);
        let before = &self.source.text()[self.source.line_start(position.line)..end];
        if before.trim().is_empty() {
            position.line
        } else {
            position.line + 1
        }
    }

    /// The first line that belongs to a node with its header on `header_line`. The reader walks
    /// upward over tag lines, comments and blank lines, and gives the topmost tag line or comment.
    fn first_line_of(&self, header_line: usize) -> usize {
        let mut first = header_line;
        let mut line = header_line;
        while line > 1 {
            line -= 1;
            if self.line(line).trim().is_empty() {
                continue;
            }
            if self.is_blank_or_comment(line) || tag_words(self.line(line)).is_some() {
                first = line;
            } else {
                break;
            }
        }
        first
    }

    /// The description of a node: the lines after its header and before the first line of its
    /// first child. `node_end` is the end offset of the node, for a node without children.
    fn description(
        &self,
        header_line: usize,
        first_child: Option<usize>,
        node_end: usize,
    ) -> Description {
        let bound = first_child.unwrap_or_else(|| self.end_line(node_end));
        let end_line = self.first_line_of(bound);
        if end_line <= header_line + 1 {
            return Description::default();
        }
        let range = self.lines(header_line + 1, end_line);
        let indent = common_indent(self.source, range);
        parse_blocks(self.source, range, indent)
    }

    /// The tags on the lines above a header, in source order, each with its span.
    fn tags(&self, header_line: usize) -> Vec<Tag> {
        let mut tags = Vec::new();
        for line in self.first_line_of(header_line)..header_line {
            let start = self.source.line_start(line);
            for (at, word) in tag_words(self.line(line)).unwrap_or_default() {
                let mut offset = start + at;
                for part in word.split('@').skip(1) {
                    offset += 1;
                    if !part.is_empty() {
                        let span = Span {
                            start: offset - 1,
                            end: offset + part.len(),
                        };
                        tags.push(Tag {
                            name: part.to_owned(),
                            span,
                            position: self.position(span.start),
                        });
                    }
                    offset += part.len();
                }
            }
        }
        tags
    }

    fn feature(&self, f: &gherkin::Feature) -> Feature {
        let span = self.span(f.span);
        let header = self.position(span.start);
        let first_child = first_line([
            f.background.as_ref().map(|b| self.header_line(b.span)),
            f.scenarios.first().map(|s| self.header_line(s.span)),
            f.rules.first().map(|r| self.header_line(r.span)),
        ]);
        Feature {
            keyword: f.keyword.clone(),
            name: f.name.clone(),
            tags: self.tags(header.line),
            description: self.description(header.line, first_child, span.end),
            background: f.background.as_ref().map(|b| self.background(b)),
            rules: f.rules.iter().map(|r| self.rule(r)).collect(),
            scenarios: f.scenarios.iter().map(|s| self.scenario(s)).collect(),
            span,
            position: header,
        }
    }

    fn rule(&self, r: &gherkin::Rule) -> Rule {
        let span = self.span(r.span);
        let header = self.position(span.start);
        let first_child = first_line([
            r.background.as_ref().map(|b| self.header_line(b.span)),
            r.scenarios.first().map(|s| self.header_line(s.span)),
        ]);
        Rule {
            keyword: r.keyword.clone(),
            name: r.name.clone(),
            tags: self.tags(header.line),
            description: self.description(header.line, first_child, span.end),
            background: r.background.as_ref().map(|b| self.background(b)),
            scenarios: r.scenarios.iter().map(|s| self.scenario(s)).collect(),
            span,
            position: header,
        }
    }

    fn background(&self, b: &gherkin::Background) -> Background {
        let span = self.span(b.span);
        let header = self.position(span.start);
        let first_child = first_line([b.steps.first().map(|s| self.header_line(s.span))]);
        Background {
            keyword: b.keyword.clone(),
            name: b.name.clone(),
            description: self.description(header.line, first_child, span.end),
            steps: self.steps(&b.steps),
            span,
            position: header,
        }
    }

    fn scenario(&self, s: &gherkin::Scenario) -> Scenario {
        let span = self.span(s.span);
        let header = self.position(span.start);
        let first_child = first_line([
            s.steps.first().map(|step| self.header_line(step.span)),
            s.examples.first().map(|e| self.header_line(e.span)),
        ]);
        Scenario {
            keyword: s.keyword.clone(),
            name: s.name.clone(),
            tags: self.tags(header.line),
            description: self.description(header.line, first_child, span.end),
            steps: self.steps(&s.steps),
            examples: s.examples.iter().map(|e| self.examples(e)).collect(),
            span,
            position: header,
        }
    }

    fn examples(&self, e: &gherkin::Examples) -> Examples {
        let span = self.span(e.span);
        let header = self.position(span.start);
        let first_child = first_line([e.table.as_ref().map(|t| self.header_line(t.span))]);
        Examples {
            keyword: e.keyword.clone(),
            name: e.name.clone(),
            tags: self.tags(header.line),
            description: self.description(header.line, first_child, span.end),
            table: e.table.as_ref().map(|t| self.table(t)),
            notes: Description::default(),
            span,
            position: header,
        }
    }

    /// `gherkin` 0.16 already resolves `And` and `But` to the kind of the step before them.
    fn steps(&self, steps: &[gherkin::Step]) -> Vec<Step> {
        steps.iter().map(|s| self.step(s)).collect()
    }

    fn step(&self, s: &gherkin::Step) -> Step {
        let span = self.span(s.span);
        let position = self.position(span.start);
        let kind = match s.ty {
            gherkin::StepType::Given => StepKind::Given,
            gherkin::StepType::When => StepKind::When,
            gherkin::StepType::Then => StepKind::Then,
        };
        let argument = if s.docstring.is_some() {
            Some(StepArgument::DocString(self.doc_string(position.line)))
        } else {
            s.table.as_ref().map(|t| StepArgument::Table(self.table(t)))
        };
        Step {
            keyword: s.keyword.clone(),
            kind,
            text: s.value.clone(),
            argument,
            notes: Description::default(),
            span,
            position,
        }
    }

    /// Reads the doc string of the step on `step_line`. Its opening delimiter is the first line
    /// after the step that is not blank and not a comment. The content type follows the opening
    /// delimiter. Each body line loses the delimiter's indent, but keeps any deeper indent. The body
    /// stops at the first line that starts with the same delimiter.
    fn doc_string(&self, step_line: usize) -> DocString {
        let last = self.source.line_count();
        let opening_line = (step_line + 1..=last)
            .find(|&line| !self.is_blank_or_comment(line))
            .unwrap_or(last);
        let opening = self.line(opening_line);
        let indent = leading_whitespace(opening);
        let opening_text = opening.trim();
        let marker = if opening_text.starts_with("```") {
            "```"
        } else {
            "\"\"\""
        };
        let content_type = opening_text.strip_prefix(marker).unwrap_or_default().trim();
        let mut body = String::new();
        let mut line = opening_line + 1;
        while line <= last {
            let text = self.line(line);
            if text.trim_start().starts_with(marker) {
                break;
            }
            let kept = &text[leading_whitespace(text).min(indent)..];
            body.push_str(&unescape_doc_string(kept, marker));
            line += 1;
        }
        let start = self.source.line_start(opening_line);
        let span = Span {
            start,
            end: self.lines(line, line + 1).end,
        };
        DocString {
            content_type: (!content_type.is_empty()).then(|| content_type.to_owned()),
            body,
            span,
            position: self.position(start + indent),
        }
    }

    fn table(&self, t: &gherkin::Table) -> Table {
        let span = self.span(t.span);
        Table {
            rows: t.rows.clone(),
            span,
            position: self.position(span.start),
        }
    }
}

/// Inside a doc string, a backslash before each character of the delimiter stands for the
/// delimiter: `\"\"\"` gives `"""` and `` \`\`\` `` gives three backticks. This follows the
/// Gherkin reference parsers.
fn unescape_doc_string(line: &str, marker: &str) -> String {
    let escaped: String = marker.chars().flat_map(|c| ['\\', c]).collect();
    line.replace(&escaped, marker)
}

/// The number of bytes of leading spaces and tabs.
fn leading_whitespace(text: &str) -> usize {
    text.len() - text.trim_start_matches([' ', '\t']).len()
}

/// The smallest line of the candidates, if any.
fn first_line<const N: usize>(candidates: [Option<usize>; N]) -> Option<usize> {
    candidates.into_iter().flatten().min()
}

/// The `@` words of a tag line, each with its byte offset in the line. A tag line holds only `@`
/// words, and can end with a `#` comment. Any other line gives `None`.
fn tag_words(line: &str) -> Option<Vec<(usize, &str)>> {
    let mut words = Vec::new();
    for word in line.split_whitespace() {
        if word.starts_with('#') {
            break;
        }
        if !word.starts_with('@') {
            return None;
        }
        words.push((word.as_ptr() as usize - line.as_ptr() as usize, word));
    }
    (!words.is_empty()).then_some(words)
}
