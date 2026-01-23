use morphir_ir::ir::v4::IRFile;
use schemars::schema_for;
use starbase::AppResult;
use std::path::PathBuf;

pub fn run_schema(output: Option<PathBuf>) -> AppResult {
    let schema = schema_for!(IRFile);
    let schema_json = match serde_json::to_string_pretty(&schema) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("Failed to serialize schema: {}", e);
            return Ok(Some(1));
        }
    };

    if let Some(path) = output {
        if let Err(e) = std::fs::write(&path, schema_json) {
            eprintln!("Failed to write schema to {:?}: {}", path, e);
            return Ok(Some(1));
        }
    } else {
        println!("{}", schema_json);
    }

    Ok(None)
}
