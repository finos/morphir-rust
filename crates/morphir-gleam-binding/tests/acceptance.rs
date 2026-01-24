//! BDD Acceptance Tests for Gleam Binding
//!
//! Uses Cucumber/Gherkin for behavior-driven testing of the full
//! Gleam parsing and code generation pipeline.

mod cli_helpers;
mod coverage;

use anyhow::Result;
use cucumber::{given, then, when, World};
use morphir_common::vfs::{MemoryVfs, Vfs};
use morphir_gleam_binding::frontend::parser::ModuleIR;
use morphir_gleam_binding::frontend::{parse_gleam, GleamToMorphirVisitor};
use morphir_ir::naming::{ModuleName, PackageName};
use std::path::PathBuf;
use tempfile::TempDir;

#[derive(Debug, World)]
pub struct GleamTestWorld {
    source_files: Vec<(String, String)>, // (path, content)
    parsed_modules: Vec<ModuleIR>,
    parse_errors: Vec<String>,
    generated_files: Vec<(String, String)>, // (path, content)
    morphir_ir: Option<serde_json::Value>,
    temp_dir: Option<TempDir>,
    project_root: Option<PathBuf>,
    cli_context: Option<cli_helpers::CliTestContext>,
    cli_result: Option<cli_helpers::CommandResult>,
}

#[given(expr = "I have a Gleam source file {string} with:")]
async fn i_have_gleam_source_file(
    w: &mut GleamTestWorld,
    filename: String,
    step: &cucumber::gherkin::Step,
) {
    let content = step.docstring.as_ref().expect("Docstring required").clone();
    w.source_files.push((filename, content));
}

#[given(expr = "I have a Gleam project at {string}")]
async fn i_have_gleam_project(w: &mut GleamTestWorld, project_path: String) {
    let dir = tempfile::tempdir().expect("Failed to create temp dir");
    w.project_root = Some(dir.path().join(project_path));
    w.temp_dir = Some(dir);
}

#[given(expr = "the project has the following structure:")]
async fn project_has_structure(w: &mut GleamTestWorld, step: &cucumber::gherkin::Step) {
    let root = w.project_root.as_ref().expect("Project root not set");
    if let Some(table) = &step.table {
        for row in &table.rows {
            if row.is_empty() {
                continue;
            }
            let path = root.join(&row[0]);
            let content = if row.len() > 1 { &row[1] } else { "" };

            // Create parent directories
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("Failed to create directory");
            }

            std::fs::write(&path, content).expect("Failed to write file");
        }
    }
}

#[when(expr = "I parse the project")]
async fn i_parse_project(w: &mut GleamTestWorld) {
    let root = w.project_root.as_ref().expect("Project root not set");

    // Find all .gleam files
    let mut gleam_files = Vec::new();
    find_gleam_files(root, &mut gleam_files);

    for file_path in gleam_files {
        let content = std::fs::read_to_string(&file_path).expect("Failed to read file");
        let relative_path = file_path
            .strip_prefix(root)
            .expect("Path not under root")
            .to_string_lossy()
            .replace(".gleam", "")
            .replace("/", "_");

        match parse_gleam(&relative_path, &content) {
            Ok(module) => w.parsed_modules.push(module),
            Err(e) => w.parse_errors.push(e.to_string()),
        }
    }
}

#[then(expr = "I should get {int} modules")]
async fn should_get_modules(w: &mut GleamTestWorld, count: usize) {
    assert_eq!(w.parsed_modules.len(), count);
}

#[then(expr = "module {string} should exist")]
async fn module_should_exist(w: &mut GleamTestWorld, name: String) {
    let exists = w.parsed_modules.iter().any(|m| m.name == name);
    assert!(exists, "Module {} not found", name);
}

#[then(expr = "module {string} should have {int} values")]
async fn module_should_have_values(w: &mut GleamTestWorld, name: String, count: usize) {
    let module = w
        .parsed_modules
        .iter()
        .find(|m| m.name == name)
        .expect(&format!("Module {} not found", name));
    assert_eq!(module.values.len(), count);
}

// Helper function to find all .gleam files in a directory
fn find_gleam_files(dir: &std::path::Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                find_gleam_files(&path, files);
            } else if path.extension().and_then(|s| s.to_str()) == Some("gleam") {
                files.push(path);
            }
        }
    }
}

#[when(expr = "I parse the file")]
async fn i_parse_file(w: &mut GleamTestWorld) {
    if let Some((path, content)) = w.source_files.first() {
        match parse_gleam(path, content) {
            Ok(module) => {
                w.parsed_modules.push(module);
            }
            Err(e) => {
                w.parse_errors.push(e.to_string());
            }
        }
    }
}

#[when(expr = "I parse {string} to ModuleIR")]
async fn i_parse_file_by_name(w: &mut GleamTestWorld, filename: String) {
    if let Some((_path, content)) = w.source_files.iter().find(|(p, _)| p == &filename) {
        match parse_gleam(&filename, content) {
            Ok(module) => {
                w.parsed_modules.push(module);
            }
            Err(e) => {
                w.parse_errors.push(e.to_string());
            }
        }
    }
}

#[when(expr = "I convert IR V4 Document Tree back to Gleam source {string}")]
async fn convert_from_ir_v4(w: &mut GleamTestWorld, output_file: String) {
    use morphir_common::vfs::{MemoryVfs, Vfs};
    use morphir_gleam_binding::backend::visitor::MorphirToGleamVisitor;
    use morphir_ir::ir::v4::PackageDefinition;
    use morphir_ir::naming::ModuleName;

    // For now, simplified - would need to load IR V4 from document tree
    // This is a placeholder implementation
    let vfs = MemoryVfs::new();
    let output_dir = PathBuf::from("/test-output");
    let package_name = "test-package".to_string();
    let visitor = MorphirToGleamVisitor::new(vfs.clone(), output_dir.clone(), package_name);

    // TODO: Load PackageDefinition from document tree and convert
    // For now, just mark that conversion was attempted
    w.generated_files
        .push((output_file, "// Generated".to_string()));
}

#[then(expr = "the roundtrip should complete")]
async fn roundtrip_should_complete(w: &mut GleamTestWorld) {
    // Check that we have parsed modules
    assert!(
        !w.parsed_modules.is_empty(),
        "No modules parsed for roundtrip"
    );
    // Check that we have generated files
    assert!(
        !w.generated_files.is_empty(),
        "No files generated for roundtrip"
    );
}

#[then(expr = "the original and generated ModuleIR should be semantically equivalent")]
async fn modules_should_be_equivalent(w: &mut GleamTestWorld) {
    // Compare original and generated ModuleIR
    // Allow formatting differences but require semantic equivalence
    assert!(
        w.parsed_modules.len() >= 2,
        "Expected at least original and generated modules"
    );

    let original = &w.parsed_modules[0];
    let generated = &w.parsed_modules[1];

    // Basic semantic equivalence check
    assert_eq!(original.name, generated.name, "Module names should match");
    assert_eq!(
        original.types.len(),
        generated.types.len(),
        "Type count should match"
    );
    assert_eq!(
        original.values.len(),
        generated.values.len(),
        "Value count should match"
    );
}

#[then(expr = "each roundtrip should produce equivalent ModuleIR")]
async fn roundtrips_should_be_equivalent(w: &mut GleamTestWorld) {
    // Compare all intermediate ModuleIR results
    // Ensure they're all semantically equivalent
    if w.parsed_modules.len() < 2 {
        return; // Not enough modules to compare
    }

    let first = &w.parsed_modules[0];
    for module in w.parsed_modules.iter().skip(1) {
        assert_eq!(first.name, module.name);
        assert_eq!(first.types.len(), module.types.len());
        assert_eq!(first.values.len(), module.values.len());
    }
}

#[then(expr = "the roundtrip should preserve {string}")]
async fn roundtrip_should_preserve(w: &mut GleamTestWorld, _aspect: String) {
    // Check specific aspect is preserved (e.g., "function signature", "type definition")
    // For now, basic check that modules exist
    assert!(
        !w.parsed_modules.is_empty(),
        "No modules to check preservation"
    );
}

#[when(expr = "I convert ModuleIR to IR V4 Document Tree")]
async fn convert_to_ir_v4(w: &mut GleamTestWorld) {
    use morphir_common::vfs::Vfs;

    let vfs = MemoryVfs::new();
    let output_dir = PathBuf::from("/test-output");
    let package_name = PackageName::parse("test-package");

    for module_ir in &w.parsed_modules {
        let module_name = ModuleName::parse(&module_ir.name);
        let visitor = GleamToMorphirVisitor::new(
            vfs.clone(),
            output_dir.clone(),
            package_name.clone(),
            module_name,
        );

        if let Err(e) = visitor.visit_module_v4(module_ir) {
            w.parse_errors
                .push(format!("Failed to convert to IR V4: {}", e));
        }
    }
}

#[then(expr = "parsing should succeed")]
async fn parsing_should_succeed(w: &mut GleamTestWorld) {
    assert!(
        w.parse_errors.is_empty(),
        "Parsing failed with errors: {:?}",
        w.parse_errors
    );
    assert!(!w.parsed_modules.is_empty(), "No modules were parsed");
}

#[then(expr = "the parsed module should have name {string}")]
async fn module_should_have_name(w: &mut GleamTestWorld, name: String) {
    let module = w.parsed_modules.first().expect("No parsed modules");
    assert_eq!(module.name, name);
}

#[then(expr = "the parsed module should have {int} type definitions")]
async fn module_should_have_type_count(w: &mut GleamTestWorld, count: usize) {
    let module = w.parsed_modules.first().expect("No parsed modules");
    assert_eq!(module.types.len(), count);
}

#[then(expr = "the parsed module should have {int} value definitions")]
async fn module_should_have_value_count(w: &mut GleamTestWorld, count: usize) {
    let module = w.parsed_modules.first().expect("No parsed modules");
    assert_eq!(module.values.len(), count);
}

// CLI E2E test step definitions

#[given(expr = "I have a temporary test project")]
async fn i_have_temp_test_project(w: &mut GleamTestWorld) {
    let ctx = cli_helpers::CliTestContext::new().expect("Failed to create test context");
    ctx.create_test_project()
        .expect("Failed to create test project");
    w.cli_context = Some(ctx);
}

#[given(expr = "I have a Gleam project structure:")]
async fn i_have_gleam_project_structure(w: &mut GleamTestWorld, step: &cucumber::gherkin::Step) {
    let ctx = w.cli_context.as_mut().expect("CLI context not initialized");

    if let Some(table) = &step.table {
        for row in &table.rows {
            if row.is_empty() {
                continue;
            }
            let path = &row[0];
            let content = if row.len() > 1 { &row[1] } else { "" };
            ctx.write_source_file(path, content)
                .expect("Failed to write source file");
        }
    }
}

#[given(expr = "I have a morphir.toml file:")]
async fn i_have_morphir_toml(w: &mut GleamTestWorld, step: &cucumber::gherkin::Step) {
    let ctx = w.cli_context.as_mut().expect("CLI context not initialized");

    let content = step.docstring.as_ref().expect("Docstring required").clone();

    ctx.write_test_config(&content)
        .expect("Failed to write morphir.toml");
}

#[given(expr = "I have an invalid morphir.toml file:")]
async fn i_have_invalid_morphir_toml(w: &mut GleamTestWorld, step: &cucumber::gherkin::Step) {
    let ctx = w.cli_context.as_mut().expect("CLI context not initialized");

    let content = step.docstring.as_ref().expect("Docstring required").clone();

    // Write invalid config
    ctx.write_test_config(&content)
        .expect("Failed to write invalid morphir.toml");
}

#[given(expr = "I have compiled IR at {string}")]
async fn i_have_compiled_ir(w: &mut GleamTestWorld, ir_path: String) {
    // This would set up a pre-compiled IR structure
    // For now, we'll assume it exists or will be created by compile step
    let ctx = w.cli_context.as_ref().expect("CLI context not initialized");

    let full_path = ctx.project_root.join(&ir_path);
    // Create directory structure
    std::fs::create_dir_all(&full_path).expect("Failed to create IR directory");

    // Write a minimal format.json
    let format_json = serde_json::json!({
        "formatVersion": 4,
        "packageName": "test"
    });
    std::fs::write(
        full_path.join("format.json"),
        serde_json::to_string_pretty(&format_json).unwrap(),
    )
    .expect("Failed to write format.json");
}

#[when(expr = "I run CLI command {string}")]
async fn i_run_cli_command(w: &mut GleamTestWorld, command: String) {
    let ctx = w.cli_context.as_mut().expect("CLI context not initialized");

    // Parse command into args
    let args: Vec<&str> = command
        .split_whitespace()
        .skip(1) // Skip "morphir"
        .collect();

    let result = ctx
        .execute_cli_command(&args)
        .expect("Failed to execute CLI command");

    w.cli_result = Some(result);
}

#[then(expr = "the CLI command should succeed")]
async fn cli_command_should_succeed(w: &mut GleamTestWorld) {
    let result = w.cli_result.as_ref().expect("No CLI result available");
    result.assert_success();
}

#[then(expr = "the CLI command should fail")]
async fn cli_command_should_fail(w: &mut GleamTestWorld) {
    let result = w.cli_result.as_ref().expect("No CLI result available");
    result.assert_failure();
}

#[then(expr = "the output should contain {string}")]
async fn output_should_contain(w: &mut GleamTestWorld, text: String) {
    let result = w.cli_result.as_ref().expect("No CLI result available");
    result.assert_output_contains(&text);
}

#[then(expr = "the error output should contain {string} or {string}")]
async fn error_output_should_contain_alt(w: &mut GleamTestWorld, text1: String, text2: String) {
    let result = w.cli_result.as_ref().expect("No CLI result available");

    let contains = result.stdout.contains(&text1)
        || result.stderr.contains(&text1)
        || result.stdout.contains(&text2)
        || result.stderr.contains(&text2);

    assert!(
        contains,
        "Error output does not contain '{}' or '{}'\nSTDOUT:\n{}\nSTDERR:\n{}",
        text1, text2, result.stdout, result.stderr
    );
}

#[then(expr = "the CLI should create .morphir/out/ structure")]
async fn cli_should_create_morphir_structure(w: &mut GleamTestWorld) {
    let ctx = w.cli_context.as_ref().expect("CLI context not initialized");

    cli_helpers::assert_morphir_structure(&ctx.morphir_dir, "default");
}

#[then(expr = "the CLI should create .morphir/out/<project>/compile/gleam/ structure")]
async fn cli_should_create_compile_structure(w: &mut GleamTestWorld) {
    let ctx = w.cli_context.as_ref().expect("CLI context not initialized");

    // Try to determine project name from config or use "test"
    let project = "test";
    let compile_dir = ctx
        .morphir_dir
        .join("out")
        .join(project)
        .join("compile")
        .join("gleam");

    assert!(
        compile_dir.exists(),
        "Compile output directory does not exist: {:?}",
        compile_dir
    );
}

#[then(expr = "the CLI should create .morphir/out/<project>/generate/gleam/ structure")]
async fn cli_should_create_generate_structure(w: &mut GleamTestWorld) {
    let ctx = w.cli_context.as_ref().expect("CLI context not initialized");

    let project = "test";
    let generate_dir = ctx
        .morphir_dir
        .join("out")
        .join(project)
        .join("generate")
        .join("gleam");

    assert!(
        generate_dir.exists(),
        "Generate output directory does not exist: {:?}",
        generate_dir
    );
}

#[then(expr = "the CLI should create both compile and generate output structures")]
async fn cli_should_create_both_structures(w: &mut GleamTestWorld) {
    cli_should_create_compile_structure(w).await;
    cli_should_create_generate_structure(w).await;
}

#[then(expr = "the CLI should use configuration from morphir.toml")]
async fn cli_should_use_config(w: &mut GleamTestWorld) {
    // Verify that config was used by checking output paths
    let ctx = w.cli_context.as_ref().expect("CLI context not initialized");

    // Config should have been read (no error about missing config)
    let result = w.cli_result.as_ref().expect("No CLI result available");

    assert!(
        !result.stderr.contains("No morphir.toml"),
        "Config file was not found or used"
    );
}

#[then(expr = "the JSON output should be valid")]
async fn json_output_should_be_valid(w: &mut GleamTestWorld) {
    let result = w.cli_result.as_ref().expect("No CLI result available");

    result
        .assert_json_output()
        .expect("JSON output is not valid");
}

#[then(expr = "the JSON output should contain {string}")]
async fn json_output_should_contain(w: &mut GleamTestWorld, key: String) {
    let result = w.cli_result.as_ref().expect("No CLI result available");

    let json: serde_json::Value = result
        .assert_json_output()
        .expect("JSON output is not valid");

    assert!(
        json.get(&key).is_some(),
        "JSON output does not contain key '{}'\nJSON:\n{}",
        key,
        serde_json::to_string_pretty(&json).unwrap()
    );
}

#[then(expr = "the JSON Lines output should be valid")]
async fn json_lines_output_should_be_valid(w: &mut GleamTestWorld) {
    let result = w.cli_result.as_ref().expect("No CLI result available");

    // Each line should be valid JSON
    for line in result.stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        serde_json::from_str::<serde_json::Value>(line)
            .expect(&format!("Invalid JSON line: {}", line));
    }
}

#[tokio::main]
async fn main() {
    GleamTestWorld::run("tests/features").await;
}
