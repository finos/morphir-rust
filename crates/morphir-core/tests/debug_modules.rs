use morphir_core::ir::classic::module::ModuleEntry;
use serde_json::Value;
use std::fs;

const REFERENCE_MODEL_IR: &str = "/home/damian/code/repos/github/finos/morphir-elm/tests-integration/reference-model/morphir-ir.json";

#[test]
fn test_debug_reference_model_modules() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(REFERENCE_MODEL_IR);
    if !path.exists() {
        println!("Skipping: file not found");
        return;
    }

    let content = fs::read_to_string(&path).expect("Failed to read morphir-ir.json");
    let json: Value = serde_json::from_str(&content).expect("Failed to parse JSON to Value");

    // distribution -> [Library, path, deps, package]
    // package -> {modules: [...]}

    let dist_array = json
        .get("distribution")
        .expect("No distribution field")
        .as_array()
        .expect("distribution is not an array");

    let package = &dist_array[3];
    let modules = package
        .get("modules")
        .expect("No modules field in package")
        .as_array()
        .expect("modules is not an array");

    eprintln!("Found {} modules", modules.len());

    for (i, mod_json) in modules.iter().enumerate() {
        eprintln!("-- Check Module {} --", i);
        if let Some(arr) = mod_json.as_array() {
            eprintln!("   Array Length: {}", arr.len());
        }
        // Try to deserialize this specific module to ModuleEntry
        let result: Result<ModuleEntry<Value, Value>, serde_json::Error> =
            serde_json::from_value(mod_json.clone());

        // Serialize to string for debugging context
        let json_str =
            serde_json::to_string(&mod_json).unwrap_or_else(|_| "FAILED_TO_SERIALIZE".to_string());
        eprintln!("   JSON Length: {}", json_str.len());

        match result {
            Ok(entry) => eprintln!("   OK: {:?}", entry.path),
            Err(e) => {
                eprintln!("   Result: Err({:?})", e);
                if e.is_data() {
                    let col = e.column(); // Note: for from_value, this is often 0
                    eprintln!("   Error at column: {}", col);
                    if col > 0 && col <= json_str.len() {
                        let start = col.saturating_sub(50);
                        let end = if col + 50 < json_str.len() {
                            col + 50
                        } else {
                            json_str.len()
                        };
                        eprintln!("   Context: ...{}...", &json_str[start..end]);
                    } else if col == 0 && json_str.len() > 100 {
                        // Trailing characters often imply end of stream, but from_value is weird.
                        // Let's print the end of the JSON string
                        let start = if json_str.len() > 100 {
                            json_str.len() - 100
                        } else {
                            0
                        };
                        eprintln!("   End Context: ...{}", &json_str[start..]);
                    }
                }
                // Don't panic, verify if other modules fail
            }
        }
    }
}
