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
    /// [`DiagnosticError`].
    ///
    /// The marker is only honoured where [`DiagnosticError`] writes it: at the very start of the
    /// message. A derived `Deserialize` echoes the document back into its own messages
    /// (``unknown variant `…` ``), so a document member spelling the marker out would otherwise
    /// forge a diagnostic of its own choosing. What follows the marker has to be one JSON
    /// `Diagnostic` and then nothing but the location suffix a codec appends, for the same
    /// reason.
    pub fn from_serde_error<E: fmt::Display>(err: &E) -> Option<Diagnostic> {
        let text = err.to_string();
        let rest = text.strip_prefix(DIAGNOSTIC_MARKER)?;
        // serde_json appends its own " at line N column M" text after the JSON payload, so parse
        // just the leading JSON value rather than requiring the whole remainder to be valid JSON.
        let mut carried = serde_json::Deserializer::from_str(rest).into_iter::<Diagnostic>();
        let diagnostic = carried.next()?.ok()?;
        is_location_suffix(&rest[carried.byte_offset()..]).then_some(diagnostic)
    }
}

/// Whether `text` is all a codec may add after the carried JSON: nothing, or its own location.
///
/// serde_json writes ` at line N column M` and serde-saphyr ` at line N, column M`; anything
/// else means the JSON was not the whole of what the marker introduced.
fn is_location_suffix(text: &str) -> bool {
    let text = text.trim_start();
    if text.is_empty() {
        return true;
    }
    let Some(rest) = text.strip_prefix("at line ") else {
        return false;
    };
    let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    let rest = rest.trim_start_matches(',').trim_start();
    let Some(rest) = rest.strip_prefix("column ") else {
        return false;
    };
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
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
