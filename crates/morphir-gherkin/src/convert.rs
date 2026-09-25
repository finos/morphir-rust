//! Writes a document as plain `.feature` text, with a map from each written line back to the line
//! of the node it came from.
//!
//! `Document.preamble`, `Step.notes` and `Examples.notes` are Markdown kept from a `.feature.md`
//! file. Plain Gherkin has no place to keep free text next to a Feature, a step or an Examples
//! table, so the writer keeps them as `#` comment lines: the preamble above the Feature line and
//! its tags, a step's notes right after the step and its argument, and an Examples table's notes
//! right after the table. The round trip loses these as structure, but keeps them as text.
//!
//! A `Description` (everything before a node's first child) is different: it is written back as
//! plain Markdown, not as comments, so that reading the `.feature` text gives the same
//! description again. The reference Gherkin parsers read a line whose first character is `#` as a
//! comment, even inside a description, and this crate's own `.feature` reader can even lose a
//! trailing comment-like line by folding it into the next heading's tag lines. So a description
//! prose line that would start with `#` once indented is written with a leading backslash
//! instead: `\#`. CommonMark reads `\#` back as a literal `#`, so a written prose line reads back
//! to the same text.
//!
//! A description fence's body is written byte for byte, with no escaping: it is arbitrary data
//! (YAML, Ion and the like), and a fence's own closing line always bounds it, so this crate's own
//! reader never misreads it. A body line that starts with `#` is not escaped and stays as
//! written; the reference Gherkin parsers still read that specific line as a comment, so a
//! description fence body with such a line is not portable to them, even though this crate reads
//! it back correctly.
//!
//! A table cell is escaped for the three sequences Gherkin gives special meaning inside a cell:
//! `\`, `|` and a line break, written as `\\`, `\|` and `\n`. A step's doc string is written with
//! `"""` unless its body contains `"""` anywhere, even in the middle of a line: `gherkin` 0.16
//! treats an occurrence of the delimiter anywhere in a body line as significant, not only one at
//! the line's start. When the body contains `"""`, the doc string is written with a triple
//! backtick fence instead, unless the body also contains a triple backtick anywhere, in which
//! case it stays `"""` and every `"""` in the body is escaped as `\"\"\"`, which this crate's
//! reader already unescapes back to `"""`, wherever it appears in a line.

use crate::model::*;
use crate::span::SourceText;

/// A map from a line of the text `to_feature_text` writes back to the line it came from in the
/// original file. A written line that comes from more than one source line (the closing line of a
/// prose block, for example) or from none (a blank line between blocks) maps to `0`.
#[derive(Debug, Clone, Default)]
pub struct LineMap {
    source_lines: Vec<usize>,
}

impl LineMap {
    /// The original line for a 1-based line of the written text, or `0` for a line past the end
    /// of the written text, or for a written line with no single source line of its own (a blank
    /// separator line between description blocks, for example).
    pub fn source_line(&self, feature_line: usize) -> usize {
        self.source_lines
            .get(feature_line - 1)
            .copied()
            .unwrap_or(0)
    }
}

struct Writer {
    out: String,
    map: LineMap,
}

impl Writer {
    fn line(&mut self, indent: usize, text: &str, source_line: usize) {
        self.out.push_str(&"  ".repeat(indent));
        self.out.push_str(text);
        self.out.push('\n');
        self.map.source_lines.push(source_line);
    }

    fn tags(&mut self, indent: usize, tags: &[Tag], line: usize) {
        if !tags.is_empty() {
            let text = tags
                .iter()
                .map(|t| format!("@{}", t.name))
                .collect::<Vec<_>>()
                .join(" ");
            self.line(indent, &text, line);
        }
    }

    /// A description: prose and fences, written back as plain Markdown. A prose line that would
    /// start with `#` once indented is escaped, so plain Gherkin never reads it as a comment. A
    /// fence's body is written byte for byte; see the module documentation for why that is safe
    /// for this crate's own reader, and what it means for other Gherkin tooling.
    fn description(&mut self, indent: usize, description: &Description) {
        for block in &description.blocks {
            match block {
                DescriptionBlock::Prose(p) => {
                    for (i, text) in p.markdown.trim_end().lines().enumerate() {
                        self.line(
                            indent,
                            &escape_description_prose_line(text.trim_start()),
                            p.position.line + i,
                        );
                    }
                }
                DescriptionBlock::Fence(f) => {
                    self.line(indent, &format!("```{}", f.info.raw), f.position.line);
                    for (i, text) in f.body.lines().enumerate() {
                        self.line(indent, text, f.position.line + 1 + i);
                    }
                    self.line(indent, "```", f.position.line + 1 + f.body.lines().count());
                }
            }
            self.out.push('\n');
            self.map.source_lines.push(0);
        }
    }

    /// Markdown kept only as free text: `Document.preamble`, `Step.notes` and `Examples.notes`.
    /// Every line is written as a `#` comment, so it needs no escaping.
    fn comments(&mut self, indent: usize, description: &Description) {
        for block in &description.blocks {
            match block {
                DescriptionBlock::Prose(p) => {
                    for (i, text) in p.markdown.trim_end().lines().enumerate() {
                        self.comment_line(indent, text.trim_start(), p.position.line + i);
                    }
                }
                DescriptionBlock::Fence(f) => {
                    self.comment_line(indent, &format!("```{}", f.info.raw), f.position.line);
                    for (i, text) in f.body.lines().enumerate() {
                        self.comment_line(indent, text, f.position.line + 1 + i);
                    }
                    self.comment_line(indent, "```", f.position.line + 1 + f.body.lines().count());
                }
            }
            self.out.push('\n');
            self.map.source_lines.push(0);
        }
    }

    fn comment_line(&mut self, indent: usize, text: &str, source_line: usize) {
        if text.is_empty() {
            self.line(indent, "#", source_line);
        } else {
            self.line(indent, &format!("# {text}"), source_line);
        }
    }

    fn table(&mut self, indent: usize, table: &Table) {
        let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        let widths: Vec<usize> = (0..columns)
            .map(|c| {
                table
                    .rows
                    .iter()
                    .filter_map(|r| r.get(c))
                    .map(|cell| escape_table_cell(cell).chars().count())
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        for (i, row) in table.rows.iter().enumerate() {
            let cells: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(c, cell)| format!("{:<width$}", escape_table_cell(cell), width = widths[c]))
                .collect();
            self.line(
                indent,
                &format!("| {} |", cells.join(" | ")),
                table.position.line + i,
            );
        }
    }

    fn steps(&mut self, indent: usize, steps: &[Step]) {
        for step in steps {
            self.line(
                indent,
                &format!("{}{}", step.keyword, step.text),
                step.position.line,
            );
            match &step.argument {
                Some(StepArgument::Table(t)) => self.table(indent + 1, t),
                Some(StepArgument::DocString(d)) => self.doc_string(indent + 1, d),
                None => {}
            }
            self.comments(indent + 1, &step.notes);
        }
    }

    /// A step's doc string. `choose_doc_string_delimiter` picks a delimiter the body's own lines
    /// cannot be mistaken for; see the module documentation for the three cases.
    fn doc_string(&mut self, indent: usize, d: &DocString) {
        let (marker, escape) = choose_doc_string_delimiter(&d.body);
        let opening = format!("{marker}{}", d.content_type.clone().unwrap_or_default());
        self.line(indent, &opening, d.position.line);
        for (i, text) in d.body.lines().enumerate() {
            let text = if escape {
                escape_doc_string_marker(text, marker)
            } else {
                text.to_owned()
            };
            self.line(indent, &text, d.position.line + 1 + i);
        }
        self.line(indent, marker, d.position.line + 1 + d.body.lines().count());
    }

    fn scenario(&mut self, indent: usize, s: &Scenario) {
        self.tags(indent, &s.tags, s.position.line);
        self.line(
            indent,
            &format!("{}: {}", s.keyword, s.name),
            s.position.line,
        );
        self.description(indent + 1, &s.description);
        self.steps(indent + 1, &s.steps);
        for e in &s.examples {
            self.tags(indent + 1, &e.tags, e.position.line);
            let name = e.name.clone().unwrap_or_default();
            self.line(
                indent + 1,
                format!("{}: {name}", e.keyword).trim_end(),
                e.position.line,
            );
            self.description(indent + 2, &e.description);
            if let Some(t) = &e.table {
                self.table(indent + 2, t);
            }
            self.comments(indent + 2, &e.notes);
        }
    }

    fn background(&mut self, indent: usize, b: &Background) {
        self.line(
            indent,
            format!("{}: {}", b.keyword, b.name).trim_end(),
            b.position.line,
        );
        self.description(indent + 1, &b.description);
        self.steps(indent + 1, &b.steps);
    }
}

/// Writes `doc` as `.feature` text. `source` is the text `doc` was read from; the writer takes
/// every span it needs from the model, so it does not read `source` directly.
pub fn to_feature_text(doc: &Document, _source: &SourceText) -> (String, LineMap) {
    let mut w = Writer {
        out: String::new(),
        map: LineMap::default(),
    };
    if let Some(f) = &doc.feature {
        w.comments(0, &doc.preamble);
        w.tags(0, &f.tags, f.position.line);
        // An implicit feature (an MDG file without a Feature heading) has an empty keyword and
        // name. It still has to convert to a valid `.feature` file, so the writer falls back to
        // the `Feature` keyword.
        let keyword = if f.keyword.is_empty() {
            "Feature"
        } else {
            &f.keyword
        };
        w.line(
            0,
            format!("{keyword}: {}", f.name).trim_end(),
            f.position.line,
        );
        w.description(1, &f.description);
        if let Some(b) = &f.background {
            w.background(1, b);
        }
        for s in &f.scenarios {
            w.scenario(1, s);
        }
        for r in &f.rules {
            w.tags(1, &r.tags, r.position.line);
            w.line(1, &format!("{}: {}", r.keyword, r.name), r.position.line);
            w.description(2, &r.description);
            if let Some(b) = &r.background {
                w.background(2, b);
            }
            for s in &r.scenarios {
                w.scenario(2, s);
            }
        }
    }
    (w.out, w.map)
}

/// Escapes a description prose line that would start with `#` once written and indented, so
/// plain Gherkin never reads it as a comment. A backslash in front of the `#` is a CommonMark
/// escape: reading the line back as Markdown gives a literal `#`, the same as before it was
/// written.
fn escape_description_prose_line(line: &str) -> String {
    if line.starts_with('#') {
        format!("\\{line}")
    } else {
        line.to_owned()
    }
}

/// Escapes a table cell for the three backslash sequences a Gherkin table cell gives special
/// meaning: `\|` for a literal `|`, `\\` for a literal `\`, and `\n` for a line break. Walking the
/// cell one character at a time and mapping each one to its own output, rather than running
/// several `String::replace` passes over the whole cell, keeps a backslash this function writes
/// from being escaped again by a later pass.
fn escape_table_cell(cell: &str) -> String {
    let mut escaped = String::with_capacity(cell.len());
    for c in cell.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '|' => escaped.push_str("\\|"),
            '\n' => escaped.push_str("\\n"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// The delimiter to write a doc string's body with, and whether that body needs its own
/// delimiter sequence escaped. See the module documentation for the three cases this chooses
/// between. `gherkin` 0.16 treats an occurrence of the delimiter anywhere in a body line as
/// significant, not only one at the start of the line, so this checks the body for the marker
/// appearing anywhere, not just at a line's start.
fn choose_doc_string_delimiter(body: &str) -> (&'static str, bool) {
    if !body.contains("\"\"\"") {
        ("\"\"\"", false)
    } else if !body.contains("```") {
        ("```", false)
    } else {
        ("\"\"\"", true)
    }
}

/// Escapes every occurrence of `marker` in a doc string body line by putting a backslash in front
/// of each of its characters, the same escaping this crate's `.feature` reader already
/// unescapes.
fn escape_doc_string_marker(line: &str, marker: &str) -> String {
    let escaped: String = marker.chars().flat_map(|c| ['\\', c]).collect();
    line.replace(marker, &escaped)
}
