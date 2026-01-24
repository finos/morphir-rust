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
        use morphir_extension_sdk::types::{Diagnostic, DiagnosticSeverity, SourceLocation};

        // Convert span to line/column
        let (start_line, start_col) = span_to_line_column(source, self.span.start);
        let (end_line, end_col) = span_to_line_column(source, self.span.end);

        let location = SourceLocation {
            file: file_path.to_string(),
            start_line,
            start_col,
            end_line,
            end_col,
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

/// Convert byte offset to line/column
pub(crate) fn span_to_line_column(source: &str, offset: usize) -> (u32, u32) {
    let mut line = 1;
    let mut col = 1;

    for (i, ch) in source.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }

    (line, col)
}

/// Convert chumsky Rich error to ParseError
pub(crate) fn to_parse_error(err: &Rich<'_, Token, SimpleSpan>, source: &str) -> ParseError {
    let span = err.span();
    let span_range = span.start..span.end;

    // Extract expected tokens
    let expected: Vec<String> = err.expected().map(|e| format!("{:?}", e)).collect();

    let found = err.found().map(|t| format!("{:?}", t));

    // Extract source snippet for context
    let snippet = if span.start < source.len() && span.end <= source.len() {
        Some(source[span_range.clone()].to_string())
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
        span: span_range,
        expected,
        found,
        hint,
        source_snippet: snippet,
    }
}
