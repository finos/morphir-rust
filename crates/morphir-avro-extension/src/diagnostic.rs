use std::fmt;

use morphir_extension_sdk::{Diagnostic, DiagnosticSeverity, SourceLocation, SourceRange};
use thiserror::Error;

/// A stable diagnostic emitted by the Avro backend extension.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct AvroDiagnostic {
    code: &'static str,
    message: String,
    source_name: Option<String>,
}

/// A projection diagnostic with its final protocol severity preserved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedDiagnostic {
    diagnostic: AvroDiagnostic,
    severity: DiagnosticSeverity,
}

impl ProjectedDiagnostic {
    pub(crate) fn new(diagnostic: AvroDiagnostic, severity: DiagnosticSeverity) -> Self {
        Self {
            diagnostic,
            severity,
        }
    }

    /// Return the stable backend diagnostic code.
    pub fn code(&self) -> &'static str {
        self.diagnostic.code()
    }

    /// Return the human-readable diagnostic message.
    pub fn message(&self) -> &str {
        self.diagnostic.message()
    }

    /// Return the canonical Morphir source associated with this diagnostic.
    pub fn source(&self) -> Option<&str> {
        self.diagnostic.source()
    }

    /// Return the severity chosen by the projection policy.
    pub fn severity(&self) -> DiagnosticSeverity {
        self.severity
    }

    /// Convert this checked projection diagnostic to the extension protocol.
    pub fn into_diagnostic(self) -> Diagnostic {
        self.diagnostic.into_diagnostic(self.severity)
    }
}

impl AvroDiagnostic {
    #[allow(
        dead_code,
        reason = "the backend reserves this stable constructor for the rendering stages"
    )]
    pub(crate) fn unsupported_morphir_type(type_name: impl fmt::Display) -> Self {
        Self::new("AVRO001", format!("unsupported Morphir type: {type_name}"))
    }

    #[allow(
        dead_code,
        reason = "the backend reserves this stable constructor for the rendering stages"
    )]
    pub(crate) fn unbound_type_parameter(parameter: impl fmt::Display) -> Self {
        Self::new("AVRO002", format!("unbound type parameter: {parameter}"))
    }

    #[allow(
        dead_code,
        reason = "the backend reserves this stable constructor for the rendering stages"
    )]
    pub(crate) fn name_collision(name: impl fmt::Display) -> Self {
        Self::new("AVRO003", format!("Avro name collision: {name}"))
    }

    pub(crate) fn invalid_option(message: impl Into<String>) -> Self {
        Self::new(
            "AVRO004",
            format!("invalid backend option: {}", message.into()),
        )
    }

    #[allow(
        dead_code,
        reason = "the backend reserves this stable constructor for the rendering stages"
    )]
    pub(crate) fn unsafe_recursion(type_name: impl fmt::Display) -> Self {
        Self::new(
            "AVRO005",
            format!("unsafe or unrepresentable recursion: {type_name}"),
        )
    }

    #[allow(
        dead_code,
        reason = "the backend reserves this stable constructor for the rendering stages"
    )]
    pub(crate) fn missing_linked_dependency(dependency: impl fmt::Display) -> Self {
        Self::new(
            "AVRO006",
            format!("missing linked dependency: {dependency}"),
        )
    }

    fn new(code: &'static str, message: String) -> Self {
        Self {
            code,
            message,
            source_name: None,
        }
    }

    /// The stable Avro backend diagnostic code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// The human-readable diagnostic message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Return the canonical Morphir source associated with this diagnostic.
    pub fn source(&self) -> Option<&str> {
        self.source_name.as_deref()
    }

    pub(crate) fn with_source(mut self, source: impl Into<String>) -> Self {
        if self.source_name.is_none() {
            self.source_name = Some(source.into());
        }
        self
    }
}

impl AvroDiagnostic {
    /// Convert this diagnostic to the extension protocol at the chosen severity.
    ///
    /// Canonical Morphir FQNames use the deterministic
    /// `morphir-fqname:<canonical-fqname>` URI convention with a zero range.
    pub fn into_diagnostic(self, severity: DiagnosticSeverity) -> Diagnostic {
        let location = self.source_name.map(|source_name| SourceLocation {
            uri: format!("morphir-fqname:{source_name}"),
            range: SourceRange::default(),
        });
        Diagnostic {
            severity,
            code: Some(self.code.to_owned()),
            message: self.message,
            location,
            related: Vec::new(),
        }
    }
}
