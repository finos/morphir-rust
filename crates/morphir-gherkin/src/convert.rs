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
//! comment, even inside a description or a description's fenced block, and this crate's own
//! `.feature` reader can even lose a trailing comment-like line by folding it into the next
//! heading's tag lines. So a description line that would start with `#` once indented is written
//! with a leading backslash instead: `\#`. CommonMark reads `\#` back as a literal `#`, so a
//! written prose line reads back to the same text.

use crate::model::*;
use crate::span::SourceText;

#[derive(Debug, Clone, Default)]
pub struct LineMap {
    source_lines: Vec<usize>,
}

impl LineMap {
    /// The original line for a 1-based line of the written text.
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

    /// A description: prose and fences, written back as plain Markdown. A line that would start
    /// with `#` once indented is escaped, so plain Gherkin never reads it as a comment.
    fn description(&mut self, indent: usize, description: &Description) {
        for block in &description.blocks {
            match block {
                DescriptionBlock::Prose(p) => {
                    for (i, text) in p.markdown.trim_end().lines().enumerate() {
                        self.line(
                            indent,
                            &escape_comment_marker(text.trim_start()),
                            p.position.line + i,
                        );
                    }
                }
                DescriptionBlock::Fence(f) => {
                    let info = fence_info_text(&f.info);
                    self.line(indent, &format!("```{info}"), f.position.line);
                    for (i, text) in f.body.lines().enumerate() {
                        self.line(
                            indent,
                            &escape_comment_marker(text),
                            f.position.line + 1 + i,
                        );
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
                    let info = fence_info_text(&f.info);
                    self.comment_line(indent, &format!("```{info}"), f.position.line);
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
                    .map(|cell| cell.chars().count())
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        for (i, row) in table.rows.iter().enumerate() {
            let cells: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(c, cell)| format!("{cell:<width$}", width = widths[c]).replace('|', "\\|"))
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
                Some(StepArgument::DocString(d)) => {
                    let opening = format!("\"\"\"{}", d.content_type.clone().unwrap_or_default());
                    self.line(indent + 1, &opening, d.position.line);
                    for (i, text) in d.body.lines().enumerate() {
                        self.line(indent + 1, text, d.position.line + 1 + i);
                    }
                    self.line(
                        indent + 1,
                        "\"\"\"",
                        d.position.line + 1 + d.body.lines().count(),
                    );
                }
                None => {}
            }
            self.comments(indent + 1, &step.notes);
        }
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

/// The fence info of a description fence, re-assembled from its language, its bare words and its
/// `key=value` options.
fn fence_info_text(info: &FenceInfo) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !info.language.is_empty() {
        parts.push(info.language.clone());
    }
    parts.extend(info.words.iter().cloned());
    parts.extend(
        info.options
            .iter()
            .map(|(key, value)| format!("{key}={value}")),
    );
    parts.join(" ")
}

/// Escapes a line that would start with `#` once written and indented, so plain Gherkin never
/// reads it as a comment. A backslash in front of the `#` is a CommonMark escape: reading the
/// line back as Markdown gives a literal `#`, the same as before it was written. Inside a fenced
/// block, CommonMark does not undo backslash escapes, so the backslash stays part of that line's
/// text; this only matters for a fence body line that itself starts with `#`.
fn escape_comment_marker(line: &str) -> String {
    if line.starts_with('#') {
        format!("\\{line}")
    } else {
        line.to_owned()
    }
}
