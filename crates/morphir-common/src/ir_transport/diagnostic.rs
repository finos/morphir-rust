//! Stable diagnostics produced by IR transport stages.

use std::fmt;

use morphir_core::ir::{Diagnostic as CoreDiagnostic, DiagnosticCode, DiagnosticStage};
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
    /// Attach the decoder-provided physical source location.
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

// =============================================================================
// The kit's diagnostics, as a transport diagnostic spells them
// =============================================================================

/// The transport stage one of the kit's stages answers at.
///
/// The kit knows three; the transport knows seven. `Semantic` has no transport stage of its own —
/// a semantic fault is still something read out of a document — so it answers where a
/// normalization fault does.
pub(crate) fn core_stage(stage: DiagnosticStage) -> Stage {
    match stage {
        DiagnosticStage::Syntax => Stage::Syntax,
        DiagnosticStage::Normalization | DiagnosticStage::Semantic => Stage::Normalization,
    }
}

/// The kit's own spelling of a diagnostic code, which is its serde name.
pub(crate) fn core_code_name(code: DiagnosticCode) -> String {
    match serde_json::to_value(code) {
        Ok(serde_json::Value::String(name)) => name,
        // `DiagnosticCode` is a unit-only enum with `rename_all = "snake_case"`, so this is
        // unreachable; answering with the debug spelling keeps the caller total either way.
        _ => format!("{code:?}"),
    }
}

/// One of the kit's messages with its cursor kept in it.
///
/// A [`TransportDiagnostic`]'s cursor is a semantic [`IrCursor`] and the kit's is a JSON pointer
/// into a document — or, for a document tree, a logical path and a pointer — which has no semantic
/// spelling before the document is understood. The pointer therefore travels in the message and
/// the cursor stays at the root, as every physical-syntax diagnostic in this crate does.
pub(crate) fn core_message(diagnostic: &CoreDiagnostic) -> String {
    if diagnostic.cursor.is_empty() || diagnostic.cursor == "/" {
        diagnostic.message.clone()
    } else {
        format!("{} (at {})", diagnostic.message, diagnostic.cursor)
    }
}

/// The physical source location one of the kit's diagnostics carries, when it carries one.
pub(crate) fn core_source_span(diagnostic: &CoreDiagnostic) -> Option<SourceSpan> {
    match (diagnostic.line, diagnostic.column) {
        (Some(line), Some(column)) => Some(SourceSpan {
            offset: 0,
            length: 0,
            line: line as usize,
            column: column as usize,
        }),
        _ => None,
    }
}

impl fmt::Display for TransportDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for TransportDiagnostic {}
