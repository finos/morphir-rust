//! Scalar recognition and normalization for `formatVersion`.

use super::diagnostic::FormatVersionDiagnostic;
use super::triplet::ReleaseTriplet;

/// Accepted scalar forms of `formatVersion`.
#[derive(Debug, Clone, PartialEq)]
pub enum ScalarValue {
    /// Unsigned integer family alias.
    Integer(u64),
    /// Exact release string.
    String(String),
}

impl ScalarValue {
    /// Parse one JSON scalar value.
    pub fn from_json(value: &serde_json::Value) -> Result<Self, FormatVersionDiagnostic> {
        match value {
            serde_json::Value::Number(number) => {
                if let Some(integer) = number.as_u64() {
                    Ok(Self::Integer(integer))
                } else if number.is_i64() || number.is_f64() {
                    Err(FormatVersionDiagnostic::invalid_format_version_type())
                } else {
                    Err(FormatVersionDiagnostic::format_version_out_of_range())
                }
            }
            serde_json::Value::String(string) => Ok(Self::String(string.clone())),
            _ => Err(FormatVersionDiagnostic::invalid_format_version_type()),
        }
    }

    /// Parse one YAML scalar string and optional typed integer hint.
    pub fn from_yaml_string(raw: &str) -> Result<Self, FormatVersionDiagnostic> {
        let trimmed = raw.trim();
        if trimmed.contains('.') {
            Ok(Self::String(trimmed.to_owned()))
        } else {
            parse_component(trimmed).map(|component| Self::Integer(component as u64))
        }
    }
}

/// Recognize and normalize one scalar value to an exact release triplet.
pub fn normalize_scalar(value: &ScalarValue) -> Result<ReleaseTriplet, FormatVersionDiagnostic> {
    match value {
        ScalarValue::Integer(integer) => normalize_integer(*integer),
        ScalarValue::String(string) => normalize_release_string(string),
    }
}

fn normalize_integer(value: u64) -> Result<ReleaseTriplet, FormatVersionDiagnostic> {
    if value == 0 {
        return Err(FormatVersionDiagnostic::invalid_format_version_syntax());
    }
    if value > u32::MAX as u64 {
        return Err(FormatVersionDiagnostic::format_version_out_of_range());
    }
    let major = value as u32;
    Ok(ReleaseTriplet::new(major, 0, 0))
}

fn normalize_release_string(value: &str) -> Result<ReleaseTriplet, FormatVersionDiagnostic> {
    if value != value.trim() {
        return Err(FormatVersionDiagnostic::invalid_format_version_syntax());
    }
    let mut parts = value.split('.');
    let major = parts
        .next()
        .ok_or_else(FormatVersionDiagnostic::invalid_format_version_syntax)?;
    let minor = parts
        .next()
        .ok_or_else(FormatVersionDiagnostic::invalid_format_version_syntax)?;
    let patch = parts
        .next()
        .ok_or_else(FormatVersionDiagnostic::invalid_format_version_syntax)?;
    if parts.next().is_some() {
        return Err(FormatVersionDiagnostic::invalid_format_version_syntax());
    }

    let major = parse_component(major)?;
    if major < 3 {
        return Err(FormatVersionDiagnostic::invalid_format_version_syntax());
    }
    let minor = parse_component(minor)?;
    let patch = parse_component(patch)?;
    Ok(ReleaseTriplet::new(major, minor, patch))
}

fn parse_component(value: &str) -> Result<u32, FormatVersionDiagnostic> {
    if value.is_empty() {
        return Err(FormatVersionDiagnostic::invalid_format_version_syntax());
    }
    if value.len() > 1 && value.starts_with('0') {
        return Err(FormatVersionDiagnostic::invalid_format_version_syntax());
    }
    if value.starts_with('+') || value.starts_with('-') {
        return Err(FormatVersionDiagnostic::invalid_format_version_syntax());
    }
    let parsed = value
        .parse::<u64>()
        .map_err(|_| FormatVersionDiagnostic::invalid_format_version_syntax())?;
    if parsed > u32::MAX as u64 {
        return Err(FormatVersionDiagnostic::format_version_out_of_range());
    }
    Ok(parsed as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_prerelease_suffixes() {
        let scalar = ScalarValue::from_json(&json!("3.0.0-alpha")).unwrap();
        assert_eq!(
            normalize_scalar(&scalar).unwrap_err().code(),
            "invalid_format_version_syntax"
        );
    }

    #[test]
    fn rejects_u32_overflow_in_release_strings() {
        let scalar = ScalarValue::from_json(&json!("4294967296.0.0")).unwrap();
        assert_eq!(
            normalize_scalar(&scalar).unwrap_err().code(),
            "format_version_out_of_range"
        );
    }
}
