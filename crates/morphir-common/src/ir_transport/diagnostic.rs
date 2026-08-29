//! Stable diagnostics produced by IR transport stages.

use std::fmt;

use morphir_core::migration::MigrationDiagnostic;
use morphir_core::traversal::IrCursor;

pub use morphir_core::migration::Severity;

/// Processing stage that produced a transport diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    /// Selecting the input profile or layout.
    Detection,
    /// Parsing physical JSON or YAML syntax.
    Syntax,
    /// Converting accepted vocabulary into semantic IR.
    Normalization,
    /// Converting between IR versions.
    Migration,
    /// Applying a semantic rewrite.
    Transformation,
    /// Encoding semantic IR into a physical format.
    Encoding,
    /// Publishing a completed artifact.
    Publication,
}

/// Byte and human-readable location supplied by a physical decoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceSpan {
    /// Zero-based byte offset.
    pub offset: usize,
    /// Length in bytes.
    pub length: usize,
    /// One-based source line.
    pub line: usize,
    /// One-based source column.
    pub column: usize,
}

/// Diagnostic shared by detection, codecs, transforms, and publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransportDiagnostic(Box<TransportDiagnosticData>);

#[derive(Clone, Debug, Eq, PartialEq)]
struct TransportDiagnosticData {
    code: String,
    stage: Stage,
    severity: Severity,
    cursor: IrCursor,
    message: String,
    guidance: Option<String>,
    source_span: Option<SourceSpan>,
}

impl TransportDiagnostic {
    /// Create an error at a semantic cursor.
    pub fn error(
        code: impl Into<String>,
        stage: Stage,
        cursor: IrCursor,
        message: impl Into<String>,
    ) -> Self {
        Self(Box::new(TransportDiagnosticData {
            code: code.into(),
            stage,
            severity: Severity::Error,
            cursor,
            message: message.into(),
            guidance: None,
            source_span: None,
        }))
    }

    /// Create a warning at a semantic cursor.
    pub fn warning(
        code: impl Into<String>,
        stage: Stage,
        cursor: IrCursor,
        message: impl Into<String>,
    ) -> Self {
        Self(Box::new(TransportDiagnosticData {
            code: code.into(),
            stage,
            severity: Severity::Warning,
            cursor,
            message: message.into(),
            guidance: None,
            source_span: None,
        }))
    }

    /// Attach actionable recovery guidance.
    pub fn with_guidance(mut self, guidance: impl Into<String>) -> Self {
        self.0.guidance = Some(guidance.into());
        self
    }
    pub fn with_source_span(mut self, source_span: SourceSpan) -> Self {
        self.0.source_span = Some(source_span);
        self
    }

    /// Return the stable diagnostic code.
    pub fn code(&self) -> &str {
        &self.0.code
    }

    /// Return the stage that produced the diagnostic.
    pub fn stage(&self) -> Stage {
        self.0.stage
    }

    /// Return the diagnostic severity.
    pub fn severity(&self) -> Severity {
        self.0.severity
    }

    /// Return the semantic cursor.
    pub fn cursor(&self) -> &IrCursor {
        &self.0.cursor
    }

    /// Return the human-readable message.
    pub fn message(&self) -> &str {
        &self.0.message
    }

    /// Return recovery guidance when available.
    pub fn guidance(&self) -> Option<&str> {
        self.0.guidance.as_deref()
    }

    /// Return the decoder-provided physical source location when available.
    pub fn source_span(&self) -> Option<SourceSpan> {
        self.0.source_span
    }
}

impl From<MigrationDiagnostic> for TransportDiagnostic {
    fn from(value: MigrationDiagnostic) -> Self {
        Self(Box::new(TransportDiagnosticData {
            code: value.code.to_owned(),
            stage: Stage::Migration,
            severity: value.severity,
            cursor: value.cursor().clone(),
            message: value.message,
            guidance: value.help,
            source_span: None,
        }))
    }
}

impl fmt::Display for TransportDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for TransportDiagnostic {}
