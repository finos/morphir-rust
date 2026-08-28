//! Parse error handling for Gleam parser
//!
//! This module contains error types and utilities for handling parsing errors.

use chumsky::prelude::*;
use chumsky::span::SimpleSpan;

use crate::frontend::lexer::{Span, Token};

/// Parse error type with span information
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
    pub expected: Vec<String>,
    pub found: Option<String>,
    pub hint: Option<String>,
    pub source_snippet: Option<String>,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {:?}", self.message, self.span)
    }
}

impl std::error::Error for ParseError {}

/// Convert ParseError to extension SDK Diagnostic
impl ParseError {
    pub fn to_diagnostic(
        &self,
        file_path: &str,
        source: &str,
    ) -> morphir_extension_sdk::types::Diagnostic {
        use morphir_extension_sdk::types::{
            Diagnostic, DiagnosticSeverity, SourceLocation, SourceRange,
        };

        // Convert byte spans to zero-based LSP positions.
        let start = source_position(source, self.span.start);
        let end = source_position(source, self.span.end);

        let location = SourceLocation {
            uri: file_path.to_string(),
            range: SourceRange { start, end },
        };

        // Build error message with hint
        let mut message = self.message.clone();
        if let Some(hint) = &self.hint {
            message.push('\n');
            message.push_str(hint);
        }
        if let Some(snippet) = &self.source_snippet {
            message.push('\n');
            message.push_str(&format!("Found: {}", snippet));
        }

        Diagnostic {
            severity: DiagnosticSeverity::Error,
            code: Some("PARSE_ERROR".to_string()),
            message,
            location: Some(location),
            related: vec![],
        }
    }
}

/// Convert a byte offset to a zero-based LSP position.
pub(crate) fn source_position(
    source: &str,
    offset: usize,
) -> morphir_extension_sdk::types::SourcePosition {
    use morphir_extension_sdk::types::SourcePosition;

    let offset = clamped_char_boundary(source, offset);
    let source_prefix = &source[..offset];
    let line = u32::try_from(source_prefix.bytes().filter(|byte| *byte == b'\n').count())
        .unwrap_or(u32::MAX);
    let line_prefix = source_prefix
        .rsplit_once('\n')
        .map_or(source_prefix, |(_, line)| line);
    let line_prefix = line_prefix.strip_suffix('\r').unwrap_or(line_prefix);

    SourcePosition::from_line_prefix(line, line_prefix)
}

fn clamped_char_boundary(source: &str, offset: usize) -> usize {
    let mut boundary = offset.min(source.len());
    while !source.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

/// Convert chumsky Rich error to ParseError
pub(crate) fn to_parse_error(err: &Rich<'_, Token, SimpleSpan>, source: &str) -> ParseError {
    let span = err.span();
    let span_start = clamped_char_boundary(source, span.start);
    let span_end = clamped_char_boundary(source, span.end);

    // Extract expected tokens
    let expected: Vec<String> = err.expected().map(|e| format!("{:?}", e)).collect();

    let found = err.found().map(|t| format!("{:?}", t));

    // Extract source snippet for context
    let snippet = if span_start < source.len() && span_start <= span_end {
        Some(source[span_start..span_end].to_string())
    } else {
        None
    };

    // Generate hint based on expected tokens
    let hint = if !expected.is_empty() {
        Some(format!("Expected one of: {}", expected.join(", ")))
    } else {
        None
    };

    ParseError {
        message: format!("Parse error: {:?}", err.reason()),
        span: span_start..span_end,
        expected,
        found,
        hint,
        source_snippet: snippet,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_error(span: Span) -> ParseError {
        ParseError {
            message: "invalid syntax".into(),
            span,
            expected: vec![],
            found: None,
            hint: None,
            source_snippet: None,
        }
    }

    #[test]
    fn diagnostic_uses_zero_based_lines_and_utf16_characters() {
        let source = "first\n😀x";

        let diagnostic = parse_error(10..11).to_diagnostic("mem://example.gleam", source);
        let range = diagnostic.location.expect("diagnostic location").range;

        assert_eq!(range.start.line, 1);
        assert_eq!(range.start.character, 2);
        assert_eq!(range.end.line, 1);
        assert_eq!(range.end.character, 3);
    }

    #[test]
    fn diagnostic_clamps_offsets_inside_astral_characters_to_utf8_boundaries() {
        let source = "😀x";

        let diagnostic = parse_error(1..4).to_diagnostic("mem://astral.gleam", source);
        let location = diagnostic.location.expect("diagnostic location");

        assert_eq!(location.uri, "mem://astral.gleam");
        assert_eq!(location.range.start.line, 0);
        assert_eq!(location.range.start.character, 0);
        assert_eq!(location.range.end.line, 0);
        assert_eq!(location.range.end.character, 2);
    }

    #[test]
    fn diagnostic_positions_cross_newlines_at_zero_character() {
        let source = "😀\nx";

        let diagnostic = parse_error(4..5).to_diagnostic("mem://newline.gleam", source);
        let range = diagnostic.location.expect("diagnostic location").range;

        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 2);
        assert_eq!(range.end.line, 1);
        assert_eq!(range.end.character, 0);
    }

    #[test]
    fn diagnostic_positions_exclude_crlf_line_terminators() {
        let source = "ab\r\ncd";

        let before_cr = source_position(source, 2);
        let inside_crlf = source_position(source, 3);
        let after_crlf = source_position(source, 4);

        assert_eq!((before_cr.line, before_cr.character), (0, 2));
        assert_eq!((inside_crlf.line, inside_crlf.character), (0, 2));
        assert_eq!((after_crlf.line, after_crlf.character), (1, 0));
    }
}
