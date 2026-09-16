//! Diagnostics shared by every IR codec, matching the Morphir Compatibility Kit's
//! diagnostic codes, stages and JSON-pointer cursors.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The kit's diagnostic codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    InvalidJson,
    DuplicateMember,
    NestingTooDeep,
    InvalidType,
    MissingMember,
    UnknownMember,
    UnknownNode,
    AmbiguousShorthand,
    InvalidName,
    InvalidPath,
    InvalidFqname,
    InvalidLiteral,
    InvalidAccess,
    InvalidDistributionShape,
    LegacySpelling,
    MissingFormatVersion,
    DuplicateFormatVersion,
    InvalidFormatVersionType,
    InvalidFormatVersionSyntax,
    FormatVersionOutOfRange,
    UnsupportedFormatVersionMajor,
    UnsupportedFormatVersionRevision,
    InvalidYaml,
    UnsupportedYamlFeature,
}

/// The stage in which a diagnostic was raised.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticStage {
    Syntax,
    Normalization,
    Semantic,
}

/// A single diagnostic, located by a JSON pointer cursor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub stage: DiagnosticStage,
    pub cursor: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

/// A non-fatal warning, e.g. `legacy_spelling`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    pub code: DiagnosticCode,
    pub cursor: String,
}

impl Diagnostic {
    pub fn new(
        code: DiagnosticCode,
        stage: DiagnosticStage,
        cursor: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            stage,
            cursor: cursor.into(),
            message: message.into(),
            line: None,
            column: None,
        }
    }

    pub fn normalization(
        code: DiagnosticCode,
        cursor: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(code, DiagnosticStage::Normalization, cursor, message)
    }

    pub fn syntax(
        code: DiagnosticCode,
        cursor: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(code, DiagnosticStage::Syntax, cursor, message)
    }

    /// Recovers a [`Diagnostic`] previously smuggled through a serde error via
    /// [`DiagnosticError`]. serde_json appends its own `at line N column M` text (and other
    /// codecs may prefix their own context), so the marker is searched for anywhere in the
    /// error's `Display` output rather than requiring it as a prefix.
    pub fn from_serde_error<E: fmt::Display>(err: &E) -> Option<Diagnostic> {
        let text = err.to_string();
        let start = text.find(DIAGNOSTIC_MARKER)? + DIAGNOSTIC_MARKER.len();
        let rest = &text[start..];
        // serde_json appends its own " at line N column M" text after the JSON payload, so
        // parse just the leading JSON value and ignore whatever trails it rather than requiring
        // the whole remainder to be valid JSON.
        let mut de = serde_json::Deserializer::from_str(rest);
        Diagnostic::deserialize(&mut de).ok()
    }
}

const DIAGNOSTIC_MARKER: &str = "@@morphir-diagnostic@@";

/// Wraps a [`Diagnostic`] so it can be carried through a serde error via
/// `serde::de::Error::custom`, and recovered later with [`Diagnostic::from_serde_error`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticError(pub Diagnostic);

impl fmt::Display for DiagnosticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let json = serde_json::to_string(&self.0).map_err(|_| fmt::Error)?;
        write!(f, "{DIAGNOSTIC_MARKER}{json}")
    }
}

impl std::error::Error for DiagnosticError {}
