//! Stable diagnostic categories for format-version recognition.

/// Diagnostic produced while recognizing or checking `formatVersion`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatVersionDiagnostic {
    code: &'static str,
    message: String,
}

impl FormatVersionDiagnostic {
    /// Create a diagnostic with a stable code and human-readable message.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Return the stable diagnostic code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Return the human-readable message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Diagnostic for a root mapping that omits `formatVersion`.
    pub fn missing_format_version() -> Self {
        Self::new(
            "missing_format_version",
            "the root mapping is missing formatVersion",
        )
    }

    /// Diagnostic for more than one root-level `formatVersion` member.
    pub fn duplicate_format_version() -> Self {
        Self::new(
            "duplicate_format_version",
            "the root mapping contains more than one formatVersion member",
        )
    }

    /// Diagnostic when `formatVersion` is not a string or unsigned integer.
    pub fn invalid_format_version_type() -> Self {
        Self::new(
            "invalid_format_version_type",
            "formatVersion must be a string or unsigned integer",
        )
    }

    /// Diagnostic when a release string does not match the accepted grammar.
    pub fn invalid_format_version_syntax() -> Self {
        Self::new(
            "invalid_format_version_syntax",
            "formatVersion does not match the accepted release grammar",
        )
    }

    /// Diagnostic when a numeric component exceeds the unsigned 32-bit range.
    pub fn format_version_out_of_range() -> Self {
        Self::new(
            "format_version_out_of_range",
            "a formatVersion component exceeds the unsigned 32-bit range",
        )
    }

    pub(crate) fn unsupported_format_version_major(release: &str) -> Self {
        Self::new(
            "unsupported_format_version_major",
            format!("no supported release exists for major family {release}"),
        )
    }

    pub(crate) fn unsupported_format_version_revision(release: &str) -> Self {
        Self::new(
            "unsupported_format_version_revision",
            format!("release {release} is recognized but not supported"),
        )
    }
}

impl std::fmt::Display for FormatVersionDiagnostic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for FormatVersionDiagnostic {}
