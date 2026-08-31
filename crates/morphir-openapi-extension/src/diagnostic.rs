use morphir_extension_sdk::{Diagnostic, DiagnosticSeverity};
use thiserror::Error;

/// A stable diagnostic emitted by the OpenAPI and JSON Schema backend.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct SchemaDiagnostic {
    code: &'static str,
    message: String,
    source_name: Option<String>,
}

impl SchemaDiagnostic {
    fn new(code: &'static str, message: impl Into<String>, source_name: Option<String>) -> Self {
        Self {
            code,
            message: message.into(),
            source_name,
        }
    }

    /// The host asked for a target this extension does not advertise.
    pub fn unknown_target(target: &str) -> Self {
        Self::new(
            "JSC001",
            format!(
                "unsupported generation target '{target}'; this extension advertises 'openapi' and 'json-schema'"
            ),
            None,
        )
    }

    /// A backend option was unknown, of the wrong type, or out of range.
    pub fn invalid_option(message: impl Into<String>) -> Self {
        Self::new("JSC002", message, None)
    }

    /// A Morphir form has no safe schema projection.
    pub fn unsupported_form(source_name: &str, message: impl Into<String>) -> Self {
        Self::new("JSC003", message, Some(source_name.to_owned()))
    }

    /// Two projected declarations claimed the same schema name.
    pub fn name_collision(source_name: &str, message: impl Into<String>) -> Self {
        Self::new("JSC004", message, Some(source_name.to_owned()))
    }

    /// Return the stable backend diagnostic code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Return the human-readable diagnostic message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Return the canonical Morphir source associated with this diagnostic.
    pub fn source(&self) -> Option<&str> {
        self.source_name.as_deref()
    }

    /// Convert this diagnostic to the extension protocol representation.
    pub fn into_diagnostic(self, severity: DiagnosticSeverity) -> Diagnostic {
        Diagnostic {
            severity,
            code: Some(self.code.into()),
            message: match &self.source_name {
                Some(source) => format!("{}: {}", source, self.message),
                None => self.message.clone(),
            },
            location: None,
            related: Vec::new(),
        }
    }
}

/// An internal failure that must not escape as a protocol error.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum SchemaGenerationError {
    /// A renderer produced output the backend could not serialize.
    #[error("schema rendering failed: {0}")]
    Rendering(String),
}
