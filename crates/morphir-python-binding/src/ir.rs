//! Version boundaries around the shared Python language mapping.

mod classic;

use crate::{Outcome, error};
use morphir_core::ir::v4::{self, IRFile};

#[derive(Clone, Copy)]
pub(crate) enum Version {
    V3,
    V4,
}

impl Version {
    pub(crate) fn parse(text: &str) -> Outcome<Self> {
        match text {
            "3" | "3.0.0" => Ok(Self::V3),
            "4" | "4.0.0" => Ok(Self::V4),
            _ => Err(error("PY001", "Expected IR version 3, 3.0.0, 4 or 4.0.0")),
        }
    }

    pub(crate) fn major(self) -> &'static str {
        match self {
            Self::V3 => "3",
            Self::V4 => "4",
        }
    }

    pub(crate) fn encode(self, model: IRFile) -> Outcome<serde_json::Value> {
        match self {
            Self::V3 => serde_json::to_value(classic::encode(&model)?),
            Self::V4 => {
                v4::with_type_encoding(v4::TypeEncoding::Compact, || serde_json::to_value(model))
            }
        }
        .map_err(|e| error("PY005", e.to_string()))
    }
}

pub(crate) fn decode(value: serde_json::Value) -> Outcome<IRFile> {
    use morphir_core::format_version::{NormalizedFormatVersion, ScalarValue, SupportTable};
    let header = value
        .get("formatVersion")
        .ok_or_else(|| error("PY005", "Missing formatVersion"))?;
    let scalar = ScalarValue::from_json(header).map_err(|e| error("PY005", e.to_string()))?;
    let normalized = NormalizedFormatVersion::from_scalar(&scalar, &SupportTable::reference())
        .map_err(|e| error("PY005", e.to_string()))?;
    if !normalized.is_supported() {
        return Err(error("PY005", "Unsupported Morphir IR version"));
    }
    if normalized.release.major() == 3 {
        classic::decode(value)
    } else {
        // Let the shared v4 codec enforce its complete version/header contract.
        serde_json::from_value(value).map_err(|e| error("PY005", e.to_string()))
    }
}
