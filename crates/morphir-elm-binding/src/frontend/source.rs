//! Byte-span to LSP-style (zero-based line, UTF-16 character) source positions.

use crate::span::Span;
use morphir_extension_sdk::{
    Diagnostic, DiagnosticSeverity, SourceLocation, SourcePosition, SourceRange,
};

/// The zero-based line and UTF-16 character offset a byte falls on.
pub fn position(source: &str, byte: usize) -> SourcePosition {
    let byte = byte.min(source.len());
    let before = &source[..byte];
    let line = before.matches('\n').count() as u32;
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let character = source[line_start..byte].encode_utf16().count() as u32;
    SourcePosition { line, character }
}

/// The source range a byte span covers.
pub fn range(source: &str, span: Span) -> SourceRange {
    SourceRange {
        start: position(source, span.start),
        end: position(source, span.end),
    }
}

/// A diagnostic located at a span in one document.
pub fn diagnostic(
    uri: &str,
    source: &str,
    span: Span,
    severity: DiagnosticSeverity,
    code: &str,
    message: impl Into<String>,
) -> Diagnostic {
    Diagnostic {
        severity,
        code: Some(code.into()),
        message: message.into(),
        location: Some(SourceLocation {
            uri: uri.into(),
            range: range(source, span),
        }),
        related: Vec::new(),
    }
}
