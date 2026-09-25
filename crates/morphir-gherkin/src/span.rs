//! Source positions. A span is a byte range into the original file; a line and column are 1-based.

/// A byte range into the original file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    /// The byte offset of the span's start, inclusive.
    pub start: usize,
    /// The byte offset of the span's end, exclusive.
    pub end: usize,
}

/// A 1-based line and column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LineCol {
    /// The 1-based line number.
    pub line: usize,
    /// The 1-based column, counting characters rather than bytes.
    pub col: usize,
}

/// The original text of a file, with the start offset of each line.
#[derive(Debug, Clone)]
pub struct SourceText {
    text: String,
    line_starts: Vec<usize>,
}

impl SourceText {
    /// Wraps `text` and indexes the start offset of each of its lines.
    ///
    /// ```
    /// use morphir_gherkin::SourceText;
    ///
    /// let source = SourceText::new("a\nb\n");
    /// assert_eq!(source.line_start(2), 2);
    /// assert_eq!(source.line_col(2).line, 2);
    /// ```
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
        Self {
            text: text.to_owned(),
            line_starts,
        }
    }

    /// The wrapped text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The line and column of a byte offset. The column counts characters, not bytes.
    pub fn line_col(&self, offset: usize) -> LineCol {
        let line = match self.line_starts.binary_search(&offset) {
            Ok(index) => index,
            Err(index) => index - 1,
        };
        let start = self.line_starts[line];
        LineCol {
            line: line + 1,
            col: self.text[start..offset].chars().count() + 1,
        }
    }

    /// The byte offset where a 1-based line starts.
    pub fn line_start(&self, line: usize) -> usize {
        self.line_starts[line - 1]
    }

    /// The number of lines.
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// The text of `span`.
    pub fn slice(&self, span: Span) -> &str {
        &self.text[span.start..span.end]
    }
}
