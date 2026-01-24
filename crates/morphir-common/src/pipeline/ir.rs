//! IR Transformation Pipeline
//!
//! Extensions to the pipeline framework for Morphir IR transformations.

use crate::pipeline::{Pipeline, Step};
use crate::Result;
use anyhow;
use morphir_ir::converter;
use serde_json::Value;
use std::path::PathBuf;

/// IR format version
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrVersion {
    V3,
    V4,
}

/// Detect IR format version from JSON value
pub fn detect_ir_version(ir: &Value) -> Option<IrVersion> {
    // Check for V4 format indicators
    if ir.get("formatVersion").is_some() {
        return Some(IrVersion::V4);
    }

    // Check for V4 wrapper object format
    if let Some(obj) = ir.as_object() {
        if obj.contains_key("Library")
            || obj.contains_key("Specs")
            || obj.contains_key("Application")
        {
            return Some(IrVersion::V4);
        }
    }

    // Check for V3 tagged array format
    if let Some(arr) = ir.as_array() {
        if !arr.is_empty() {
            if let Some(first) = arr.first() {
                if let Some(tag) = first.as_str() {
                    // V3 uses tagged arrays like ["Library", ...]
                    if tag == "Library" || tag == "Specs" || tag == "Application" {
                        return Some(IrVersion::V3);
                    }
                }
            }
        }
    }

    None
}

/// V3 to V4 converter step
pub struct V3ToV4Converter;

impl Step for V3ToV4Converter {
    type Base = ();
    type Input = Value;
    type Output = Value;

    fn run(&self, input: Self::Input) -> Result<Self::Output> {
        // Deserialize to Classic Package
        let classic_pkg: morphir_ir::ir::classic::Package = serde_json::from_value(input)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize Classic IR: {}", e))?;

        // Convert to V4
        let v4_pkg = converter::classic_to_v4(classic_pkg);

        // Serialize back to JSON
        serde_json::to_value(v4_pkg)
            .map_err(|e| anyhow::anyhow!("Failed to serialize V4 IR: {}", e))
    }
}

/// V4 to V3 converter step
pub struct V4ToV3Converter;

impl Step for V4ToV3Converter {
    type Base = ();
    type Input = Value;
    type Output = Value;

    fn run(&self, input: Self::Input) -> Result<Self::Output> {
        // Deserialize to V4 PackageDefinition
        let v4_pkg: morphir_ir::ir::v4::PackageDefinition = serde_json::from_value(input)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize V4 IR: {}", e))?;

        // Convert to Classic
        let classic_pkg = converter::v4_to_classic(v4_pkg);

        // Serialize back to JSON
        serde_json::to_value(classic_pkg)
            .map_err(|e| anyhow::anyhow!("Failed to serialize Classic IR: {}", e))
    }
}

/// Format detection step
pub struct FormatDetector;

impl Step for FormatDetector {
    type Base = ();
    type Input = Value;
    type Output = (Value, IrVersion);

    fn run(&self, input: Self::Input) -> Result<Self::Output> {
        let version = detect_ir_version(&input)
            .ok_or_else(|| anyhow::anyhow!("Could not detect IR format version"))?;
        Ok((input, version))
    }
}

/// Pipeline configuration
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub transforms: Vec<String>,
    pub decorator_dir: Option<PathBuf>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            transforms: vec![],
            decorator_dir: None,
        }
    }
}

/// Build a pipeline from configuration
pub fn build_pipeline(
    config: &PipelineConfig,
) -> Pipeline<impl Step<Input = Value, Output = Value>> {
    // Start with format detection
    let _pipeline = Pipeline::new(FormatDetector);

    // Add V3 to V4 conversion if needed
    if config.transforms.contains(&"v3-to-v4".to_string()) {
        // Chain format detection -> V3 to V4
        // For now, simplified - would need proper chaining
    }

    // For now, return a simple identity pipeline
    Pipeline::new(IdentityStep)
}

/// Identity step (pass-through)
struct IdentityStep;

impl Step for IdentityStep {
    type Base = ();
    type Input = Value;
    type Output = Value;

    fn run(&self, input: Self::Input) -> Result<Self::Output> {
        Ok(input)
    }
}
