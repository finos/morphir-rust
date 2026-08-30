mod v3;
mod v4;

use serde_json::Value;
use thiserror::Error;

use crate::model::ProjectionPackage;

/// Failure to decode the supplied Morphir IR generation.
#[derive(Debug, Error)]
pub enum NormalizeError {
    /// The IR root has no `formatVersion` member.
    #[error("Morphir IR root is missing formatVersion")]
    MissingFormatVersion,
    /// The version has an unsupported JSON scalar type.
    #[error("formatVersion has invalid scalar type: {value}")]
    InvalidFormatVersionType {
        /// Supplied JSON value.
        value: String,
    },
    /// The version string is not a canonical semantic version.
    #[error("formatVersion has invalid syntax: {value}")]
    InvalidFormatVersionSyntax {
        /// Supplied version spelling.
        value: String,
    },
    /// A version component exceeds the supported integer range.
    #[error("formatVersion component is outside the unsigned 32-bit range: {value}")]
    FormatVersionOutOfRange {
        /// Supplied version spelling.
        value: String,
    },
    /// The requested major IR generation is unsupported.
    #[error("unsupported Morphir IR format major: {major}")]
    UnsupportedFormatVersionMajor {
        /// Requested major generation.
        major: u32,
    },
    /// The requested revision is newer than the supported baseline.
    #[error("unsupported Morphir IR format revision: {major}.{minor}.{patch}")]
    UnsupportedFormatVersionRevision {
        /// Requested major component.
        major: u32,
        /// Requested minor component.
        minor: u32,
        /// Requested patch component.
        patch: u32,
    },
    /// A v4 entry point does not target one canonical public value.
    #[error("entry point {identifier:?} has {reason} target {target:?}")]
    InvalidEntryPointTarget {
        /// Entry-point identifier.
        identifier: String,
        /// Supplied target FQName.
        target: String,
        /// Stable validation reason.
        reason: &'static str,
    },
    /// More than one entry-point identifier targets the same value.
    #[error("entry point target {target:?} is declared more than once by {identifiers:?}")]
    DuplicateEntryPointTarget {
        /// Duplicate target FQName.
        target: String,
        /// Entry-point identifiers sharing the target.
        identifiers: Vec<String>,
    },
    /// The generation-specific Morphir IR decoder rejected the document.
    #[error("invalid Morphir IR: {0}")]
    Decode(#[from] serde_json::Error),
}

impl NormalizeError {
    /// Stable category for callers that need structured failure handling.
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingFormatVersion => "missing_format_version",
            Self::InvalidFormatVersionType { .. } => "invalid_format_version_type",
            Self::InvalidFormatVersionSyntax { .. } => "invalid_format_version_syntax",
            Self::FormatVersionOutOfRange { .. } => "format_version_out_of_range",
            Self::UnsupportedFormatVersionMajor { .. } => "unsupported_format_version_major",
            Self::UnsupportedFormatVersionRevision { .. } => "unsupported_format_version_revision",
            Self::InvalidEntryPointTarget { .. } => "invalid_entry_point_target",
            Self::DuplicateEntryPointTarget { .. } => "duplicate_entry_point_target",
            Self::Decode(_) => "invalid_ir",
        }
    }
}

/// Normalize a supported Morphir IR distribution to its public, body-free model.
///
/// `formatVersion` is recognized and checked for exact-release compatibility
/// before a generation-specific decoder runs.
///
/// # Examples
///
/// ```
/// use morphir_avro_extension::{DistributionKind, normalize};
///
/// let ir = serde_json::json!({
///     "formatVersion": 3,
///     "distribution": ["Library", [["example"]], [], { "modules": [] }]
/// });
/// let package = normalize(&ir)?;
/// assert_eq!(package.kind, DistributionKind::Library);
/// # Ok::<(), morphir_avro_extension::NormalizeError>(())
/// ```
pub fn normalize(ir: &Value) -> Result<ProjectionPackage, NormalizeError> {
    match recognize_version(ir)? {
        SupportedVersion::V3 => serde_json::from_value(with_integer_version(ir, 3))
            .map(v3::normalize)
            .map_err(Into::into),
        SupportedVersion::V4 => {
            let ir = serde_json::from_value(with_integer_version(ir, 4))?;
            v4::normalize(ir)
        }
    }
}

fn with_integer_version(ir: &Value, major: u32) -> Value {
    let mut normalized = ir.clone();
    normalized["formatVersion"] = Value::from(major);
    normalized
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportedVersion {
    V3,
    V4,
}

fn recognize_version(ir: &Value) -> Result<SupportedVersion, NormalizeError> {
    let version = ir
        .as_object()
        .and_then(|root| root.get("formatVersion"))
        .ok_or(NormalizeError::MissingFormatVersion)?;
    let release = match version {
        Value::Number(number) => {
            let Some(major) = number.as_u64() else {
                return Err(NormalizeError::InvalidFormatVersionType {
                    value: version.to_string(),
                });
            };
            let major =
                u32::try_from(major).map_err(|_| NormalizeError::FormatVersionOutOfRange {
                    value: version.to_string(),
                })?;
            if major == 0 {
                return Err(NormalizeError::InvalidFormatVersionSyntax {
                    value: version.to_string(),
                });
            }
            (major, 0, 0)
        }
        Value::String(source) => parse_release(source)?,
        _ => {
            return Err(NormalizeError::InvalidFormatVersionType {
                value: version.to_string(),
            });
        }
    };
    match release {
        (3, 0, 0) => Ok(SupportedVersion::V3),
        (4, 0, 0) => Ok(SupportedVersion::V4),
        (3 | 4, minor, patch) => Err(NormalizeError::UnsupportedFormatVersionRevision {
            major: release.0,
            minor,
            patch,
        }),
        (major, _, _) => Err(NormalizeError::UnsupportedFormatVersionMajor { major }),
    }
}

fn parse_release(source: &str) -> Result<(u32, u32, u32), NormalizeError> {
    let components = source.split('.').collect::<Vec<_>>();
    if components.len() != 3
        || components.iter().any(|component| {
            component.is_empty()
                || !component.bytes().all(|byte| byte.is_ascii_digit())
                || (component.len() > 1 && component.starts_with('0'))
        })
    {
        return Err(NormalizeError::InvalidFormatVersionSyntax {
            value: source.to_owned(),
        });
    }
    let values = components
        .into_iter()
        .map(|component| parse_component(source, component))
        .collect::<Result<Vec<_>, _>>()?;
    if values[0] < 3 {
        return Err(NormalizeError::InvalidFormatVersionSyntax {
            value: source.to_owned(),
        });
    }
    Ok((values[0], values[1], values[2]))
}

fn parse_component(source: &str, component: &str) -> Result<u32, NormalizeError> {
    component.bytes().try_fold(0_u32, |value, byte| {
        value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u32::from(byte - b'0')))
            .ok_or_else(|| NormalizeError::FormatVersionOutOfRange {
                value: source.to_owned(),
            })
    })
}

pub(crate) fn canonical_fq_name(package: &str, module: &[String], local: &str) -> String {
    format!("{package}:{}#{local}", module.join("/"))
}

pub(crate) fn normalize_signature(
    mut inputs: Vec<crate::model::NamedType>,
    mut output: Option<crate::model::TypeExpr>,
) -> (
    Vec<crate::model::NamedType>,
    Option<crate::model::TypeExpr>,
    crate::model::ValueKind,
) {
    let mut next_argument = 1;
    while let Some(crate::model::TypeExpr::Function {
        input,
        output: next_output,
    }) = output
    {
        while inputs
            .iter()
            .any(|input| input.name == format!("arg{next_argument}"))
        {
            next_argument += 1;
        }
        inputs.push(crate::model::NamedType {
            name: format!("arg{next_argument}"),
            tpe: *input,
        });
        next_argument += 1;
        output = Some(*next_output);
    }
    let value_kind = if inputs.is_empty() {
        crate::model::ValueKind::Constant
    } else {
        crate::model::ValueKind::Function
    };
    (inputs, output, value_kind)
}
