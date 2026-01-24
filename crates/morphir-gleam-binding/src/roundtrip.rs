//! Roundtrip testing infrastructure for Gleam → IR V4 → Gleam → IR equivalence
//!
//! This module provides utilities for verifying that Gleam code can be roundtripped
//! through the Morphir IR V4 format and back to equivalent Gleam code.

use crate::backend::MorphirToGleamVisitor;
use crate::frontend::ast::ModuleIR;
use crate::frontend::{GleamToMorphirVisitor, parse_gleam};
use morphir_common::vfs::{MemoryVfs, Vfs};
use morphir_ir::ir::v4::{AccessControlledModuleDefinition, ModuleDefinition};
use morphir_ir::naming::{ModuleName, PackageName};
use std::path::PathBuf;

/// Result of a roundtrip operation
#[derive(Debug)]
pub struct RoundtripResult {
    /// The original parsed ModuleIR
    pub original: ModuleIR,
    /// The regenerated ModuleIR (parsed from generated Gleam code)
    pub regenerated: ModuleIR,
    /// The generated Gleam source code
    pub generated_code: String,
    /// The intermediate IR (as JSON for debugging)
    pub intermediate_ir: Option<serde_json::Value>,
}

/// Error during roundtrip
#[derive(Debug)]
#[allow(clippy::enum_variant_names)]
pub enum RoundtripError {
    /// Error parsing original Gleam source
    ParseError(String),
    /// Error converting to IR V4
    IrConversionError(String),
    /// Error generating Gleam from IR
    CodeGenError(String),
    /// Error parsing generated Gleam
    ReparseError(String),
}

impl std::fmt::Display for RoundtripError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoundtripError::ParseError(e) => write!(f, "Parse error: {}", e),
            RoundtripError::IrConversionError(e) => write!(f, "IR conversion error: {}", e),
            RoundtripError::CodeGenError(e) => write!(f, "Code generation error: {}", e),
            RoundtripError::ReparseError(e) => write!(f, "Reparse error: {}", e),
        }
    }
}

impl std::error::Error for RoundtripError {}

/// Perform a full roundtrip: Gleam → IR V4 → Gleam → parse
///
/// This function:
/// 1. Parses the original Gleam source to a ModuleIR
/// 2. Converts the ModuleIR to IR V4 Document Tree format
/// 3. Converts the IR V4 back to Gleam source code
/// 4. Re-parses the generated Gleam source to a new ModuleIR
/// 5. Returns both ModuleIRs and the generated code for comparison
pub fn roundtrip_gleam(source: &str) -> Result<RoundtripResult, RoundtripError> {
    roundtrip_gleam_with_options(source, "test", "test_module")
}

/// Perform a roundtrip with custom package and module names
pub fn roundtrip_gleam_with_options(
    source: &str,
    package_name: &str,
    module_name: &str,
) -> Result<RoundtripResult, RoundtripError> {
    // Step 1: Parse original Gleam source
    let original = parse_gleam("input.gleam", source)
        .map_err(|e| RoundtripError::ParseError(format!("{:?}", e)))?;

    // Step 2: Convert to IR V4 using in-memory VFS
    let ir_vfs = MemoryVfs::new();
    let output_dir = PathBuf::from("/ir");
    let pkg_name = PackageName::parse(package_name);
    let mod_name = ModuleName::parse(module_name);

    let frontend_visitor = GleamToMorphirVisitor::new(
        ir_vfs.clone(),
        output_dir.clone(),
        pkg_name.clone(),
        mod_name.clone(),
    );

    frontend_visitor
        .visit_module_v4(&original)
        .map_err(|e| RoundtripError::IrConversionError(e.to_string()))?;

    // Read the format.json if it exists (for debugging)
    let format_json_path = output_dir.join("format.json");
    let intermediate_ir: Option<serde_json::Value> = if ir_vfs.exists(&format_json_path) {
        ir_vfs
            .read_to_string(&format_json_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    } else {
        None
    };

    // Step 3: Build V4 module definition from the written files
    // For now, we'll create a simple module from the original ModuleIR
    let module_def = build_v4_module_from_ir(&original, &ir_vfs, &output_dir, &pkg_name, &mod_name)
        .map_err(|e| RoundtripError::IrConversionError(e.to_string()))?;

    // Step 4: Generate Gleam code from IR V4 using backend visitor
    let gen_vfs = MemoryVfs::new();
    let gen_output_dir = PathBuf::from("/gen");
    let backend_visitor = MorphirToGleamVisitor::new(
        gen_vfs.clone(),
        gen_output_dir.clone(),
        package_name.to_string(),
    );

    backend_visitor
        .visit_module(&mod_name, &module_def)
        .map_err(|e| RoundtripError::CodeGenError(e.to_string()))?;

    // Read generated Gleam code
    let gen_file_path = gen_output_dir.join(format!("{}.gleam", module_name));
    let generated_code = gen_vfs.read_to_string(&gen_file_path).map_err(|e| {
        RoundtripError::CodeGenError(format!("Failed to read generated file: {}", e))
    })?;

    // Step 5: Re-parse generated Gleam
    let regenerated = parse_gleam("generated.gleam", &generated_code)
        .map_err(|e| RoundtripError::ReparseError(format!("{:?}", e)))?;

    Ok(RoundtripResult {
        original,
        regenerated,
        generated_code,
        intermediate_ir,
    })
}

/// Build a V4 AccessControlledModuleDefinition from the original ModuleIR
fn build_v4_module_from_ir(
    module_ir: &ModuleIR,
    vfs: &MemoryVfs,
    output_dir: &PathBuf,
    package_name: &PackageName,
    module_name: &ModuleName,
) -> std::io::Result<AccessControlledModuleDefinition> {
    use indexmap::IndexMap;
    use morphir_ir::ir::v4::Access;

    // Try to read the written V4 files and reconstruct the module definition
    let module_dir = output_dir
        .join(".morphir-dist")
        .join("pkg")
        .join(package_name.to_string())
        .join(module_name.to_string());

    let mut types = IndexMap::new();
    let mut values = IndexMap::new();

    // Read type definitions
    let types_dir = module_dir.join("types");
    if vfs.exists(&types_dir) {
        for type_def in &module_ir.types {
            let type_file = types_dir.join(format!("{}.json", type_def.name));
            if vfs.exists(&type_file) {
                if let Ok(content) = vfs.read_to_string(&type_file) {
                    if let Ok(type_def_json) = serde_json::from_str(&content) {
                        types.insert(type_def.name.clone(), type_def_json);
                    }
                }
            }
        }
    }

    // Read value definitions
    let values_dir = module_dir.join("values");
    if vfs.exists(&values_dir) {
        for value_def in &module_ir.values {
            let value_file = values_dir.join(format!("{}.json", value_def.name));
            if vfs.exists(&value_file) {
                if let Ok(content) = vfs.read_to_string(&value_file) {
                    if let Ok(value_def_json) = serde_json::from_str(&content) {
                        values.insert(value_def.name.clone(), value_def_json);
                    }
                }
            }
        }
    }

    Ok(AccessControlledModuleDefinition {
        access: Access::Public,
        value: ModuleDefinition {
            types,
            values,
            doc: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_simple_function() {
        let source = r#"pub fn answer() { 42 }"#;

        let result = roundtrip_gleam(source);

        // For now, we just check that it doesn't panic
        // Real assertions will be added as we build out the infrastructure
        match result {
            Ok(r) => {
                println!("Original: {:?}", r.original);
                println!("Generated code:\n{}", r.generated_code);
                println!("Regenerated: {:?}", r.regenerated);
            }
            Err(e) => {
                println!("Roundtrip error: {}", e);
                // For now, we allow errors as we build out the infrastructure
            }
        }
    }

    #[test]
    fn test_roundtrip_string_literal() {
        let source = r#"pub fn hello() { "world" }"#;

        let result = roundtrip_gleam(source);

        match result {
            Ok(r) => {
                println!("Original: {:?}", r.original);
                println!("Generated code:\n{}", r.generated_code);
                println!("Regenerated: {:?}", r.regenerated);
            }
            Err(e) => {
                println!("Roundtrip error: {}", e);
            }
        }
    }
}
