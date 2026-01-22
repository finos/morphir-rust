//! WASM code generation from Morphir IR

use base64::{engine::general_purpose::STANDARD, Engine};
use morphir_extension_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wasm_encoder::{
    CodeSection, ExportKind, ExportSection, Function, FunctionSection, Instruction, Module,
    TypeSection, ValType,
};

/// Morphir distribution IR (simplified)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Distribution {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub modules: Vec<ModuleIR>,
}

/// Module IR representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleIR {
    pub name: String,
    #[serde(default)]
    pub values: Vec<ValueDef>,
}

/// Value definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueDef {
    pub name: String,
    #[serde(default)]
    pub body: serde_json::Value,
}

/// Generate WASM binary from Morphir IR
pub fn generate_wasm(
    ir: &serde_json::Value,
    _options: &HashMap<String, serde_json::Value>,
) -> Result<Vec<Artifact>> {
    let mut artifacts = Vec::new();

    // Try to parse as distribution or module list
    let (name, modules): (String, Vec<ModuleIR>) =
        if let Ok(dist) = serde_json::from_value::<Distribution>(ir.clone()) {
            (dist.name, dist.modules)
        } else if let Ok(modules) = serde_json::from_value::<Vec<ModuleIR>>(ir.clone()) {
            ("morphir".to_string(), modules)
        } else {
            // Try single module
            let module = serde_json::from_value::<ModuleIR>(ir.clone())?;
            (module.name.clone(), vec![module])
        };

    // Generate a single WASM module containing all Morphir modules
    let wasm_bytes = compile_to_wasm(&modules)?;

    // Encode as base64 for JSON transport
    let encoded = STANDARD.encode(&wasm_bytes);

    artifacts.push(Artifact {
        path: format!("{}.wasm", name),
        content: encoded,
        binary: true,
    });

    Ok(artifacts)
}

/// Compile Morphir modules to WASM bytes
fn compile_to_wasm(modules: &[ModuleIR]) -> Result<Vec<u8>> {
    let mut module = Module::new();

    // Type section - define function signatures
    let mut types = TypeSection::new();

    // Function section - declare functions
    let mut functions = FunctionSection::new();

    // Export section - export functions
    let mut exports = ExportSection::new();

    // Code section - function bodies
    let mut codes = CodeSection::new();

    // Collect all value definitions from all modules
    let mut func_index = 0u32;

    for module_ir in modules {
        for value_def in &module_ir.values {
            // For now, generate simple i32-returning functions
            // Type: () -> i32
            types.ty().function([], [ValType::I32]);

            // Function uses type at same index
            functions.function(func_index);

            // Export the function
            let export_name = format!("{}_{}", module_ir.name, value_def.name);
            exports.export(&export_name, ExportKind::Func, func_index);

            // Generate function body
            let mut func = Function::new([]);
            generate_function_body(&mut func, &value_def.body)?;
            codes.function(&func);

            func_index += 1;
        }
    }

    // Only add sections if they have content
    if func_index > 0 {
        module.section(&types);
        module.section(&functions);
        module.section(&exports);
        module.section(&codes);
    }

    Ok(module.finish())
}

/// Generate WASM instructions for a Morphir expression
fn generate_function_body(func: &mut Function, body: &serde_json::Value) -> Result<()> {
    if let Some(obj) = body.as_object() {
        if let Some(kind) = obj.get("kind").and_then(|v| v.as_str()) {
            match kind {
                "literal" => {
                    if let Some(value) = obj.get("value") {
                        if let Some(lit_obj) = value.as_object() {
                            if let Some(lit_type) = lit_obj.get("type").and_then(|v| v.as_str()) {
                                match lit_type {
                                    "int" => {
                                        if let Some(n) =
                                            lit_obj.get("value").and_then(|v| v.as_i64())
                                        {
                                            func.instruction(&Instruction::I32Const(n as i32));
                                            func.instruction(&Instruction::End);
                                            return Ok(());
                                        }
                                    }
                                    "bool" => {
                                        if let Some(b) =
                                            lit_obj.get("value").and_then(|v| v.as_bool())
                                        {
                                            func.instruction(&Instruction::I32Const(if b {
                                                1
                                            } else {
                                                0
                                            }));
                                            func.instruction(&Instruction::End);
                                            return Ok(());
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Default: return 0
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&Instruction::End);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_simple_wasm() {
        let ir = serde_json::json!({
            "name": "example",
            "modules": [{
                "name": "main",
                "values": [{
                    "name": "answer",
                    "body": {
                        "kind": "literal",
                        "value": {
                            "type": "int",
                            "value": 42
                        }
                    }
                }]
            }]
        });

        let options = HashMap::new();
        let result = generate_wasm(&ir, &options).unwrap();

        assert_eq!(result.len(), 1);
        assert!(result[0].path.ends_with(".wasm"));
        assert!(result[0].binary);

        // Decode and verify it's valid WASM
        let bytes = STANDARD.decode(&result[0].content).unwrap();
        assert!(bytes.starts_with(&[0x00, 0x61, 0x73, 0x6d])); // WASM magic number
    }
}
