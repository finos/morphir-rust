//! Shared Morphir IR `formatVersion` contract from v3 onward.
//!
//! Implements the normative grammar, normalization, support-table compatibility,
//! and canonical spelling rules defined in the parent Morphir specification.

mod canonical;
mod diagnostic;
mod parse;
mod support;
mod triplet;

pub use canonical::CanonicalSpelling;
pub use diagnostic::FormatVersionDiagnostic;
pub use parse::ScalarValue;
pub use support::{Compatibility, SupportTable, default_support_table};
pub use triplet::ReleaseTriplet;

/// Result of recognizing and normalizing one `formatVersion` scalar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedFormatVersion {
    /// Exact three-component release after normalization.
    pub release: ReleaseTriplet,
    /// Canonical wire spelling for writers.
    pub canonical: CanonicalSpelling,
    /// Compatibility against a support table.
    pub compatibility: Compatibility,
}

impl NormalizedFormatVersion {
    /// Recognize, normalize, and check compatibility for one scalar value.
    pub fn from_scalar(
        value: &ScalarValue,
        support: &SupportTable,
    ) -> Result<Self, FormatVersionDiagnostic> {
        let release = parse::normalize_scalar(value)?;
        let canonical = canonical::canonical_spelling(&release);
        let compatibility = support.check(&release);
        Ok(Self {
            release,
            canonical,
            compatibility,
        })
    }

    /// Return `true` when the release is supported by the given table.
    pub fn is_supported(&self) -> bool {
        self.compatibility == Compatibility::Supported
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn normalize_json(value: serde_json::Value) -> Result<NormalizedFormatVersion, String> {
        let scalar =
            ScalarValue::from_json(&value).map_err(|diagnostic| diagnostic.code().to_string())?;
        NormalizedFormatVersion::from_scalar(&scalar, &SupportTable::reference())
            .map_err(|diagnostic| diagnostic.code().to_string())
    }

    #[test]
    fn integer_three_normalizes_to_supported_v3() {
        let normalized = normalize_json(json!(3)).expect("v3 integer");
        assert_eq!(normalized.release, ReleaseTriplet::new(3, 0, 0));
        assert_eq!(normalized.canonical, CanonicalSpelling::Integer(3));
        assert_eq!(normalized.compatibility, Compatibility::Supported);
    }

    #[test]
    fn string_three_one_zero_is_unsupported_revision() {
        let normalized = normalize_json(json!("3.1.0")).expect("recognized revision");
        assert_eq!(normalized.release, ReleaseTriplet::new(3, 1, 0));
        assert_eq!(normalized.compatibility, Compatibility::UnsupportedRevision);
    }

    #[test]
    fn forbidden_v1_release_string_fails_syntax() {
        let error = normalize_json(json!("1.0.0")).expect_err("forbidden string");
        assert_eq!(error, "invalid_format_version_syntax");
    }
}
