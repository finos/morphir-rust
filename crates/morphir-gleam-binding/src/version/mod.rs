//! Supported IR releases at the Gleam extension boundary.
mod classic;

use morphir_core::ir::{classic as c, v4 as v};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IrVersion {
    V3,
    V4,
}

impl IrVersion {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "3" | "3.0.0" => Ok(Self::V3),
            "4" | "4.0.0" => Ok(Self::V4),
            _ => Err(format!(
                "Unsupported Morphir IR version '{value}'; Gleam supports 3 and 4"
            )),
        }
    }
    pub(crate) fn release(self) -> &'static str {
        match self {
            Self::V3 => "3.0.0",
            Self::V4 => "4.0.0",
        }
    }
    pub(crate) fn encode(self, ir: &v::IRFile) -> Result<Value, String> {
        match self {
            Self::V3 => serde_json::to_value(classic::encode(ir)?),
            Self::V4 => serde_json::to_value(ir),
        }
        .map_err(|e| e.to_string())
    }
}

pub(crate) fn read_ir(value: &Value) -> Result<v::IRFile, String> {
    let written = value
        .get("formatVersion")
        .ok_or("Missing Morphir IR formatVersion")?;
    let format: v::FormatVersion = serde_json::from_value(written.clone())
        .map_err(|e| format!("Unsupported Morphir IR version '{written}': {e}"))?;
    let normalized = format
        .normalize()
        .map_err(|e| format!("Unsupported Morphir IR version '{written}': {e}"))?;
    let version = IrVersion::parse(&normalized.release.to_exact_string())?;
    match version {
        IrVersion::V3 => {
            let classic: c::Distribution =
                serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
            morphir_core::migration::migrate_distribution(&classic, Default::default())
                .map(|result| result.value)
                .map_err(|e| e.message)
        }
        IrVersion::V4 => serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
    }
}
