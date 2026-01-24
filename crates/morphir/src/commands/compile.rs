//! Compile command for compiling source code to Morphir IR

use morphir_design::{discover_config, load_config_context, resolve_compile_output, ensure_morphir_structure};
use morphir_daemon::extensions::registry::ExtensionRegistry;
use morphir_daemon::extensions::container::ExtensionType;
use crate::error::{CliError, convert_extension_diagnostics, handle_error};
use crate::output::{OutputFormat, CompileOutput, Diagnostic, write_output};
use starbase::AppResult;
use std::path::{Path, PathBuf};
use anyhow::Context;

/// Run the compile command
pub async fn run_compile(
    language: Option<String>,
    input: Option<String>,
    output: Option<String>,
    package_name: Option<String>,
    config_path: Option<String>,
    project: Option<String>,
    json: bool,
    json_lines: bool,
) -> AppResult {
    use crate::output::{OutputFormat, CompileOutput, write_output};
    // Discover config if not provided
    let start_dir = std::env::current_dir()
        .context("Failed to get current directory")?;
    
    let config_file = if let Some(cfg) = config_path {
        PathBuf::from(cfg)
    } else {
        discover_config(&start_dir)
            .ok_or_else(|| anyhow::anyhow!("No morphir.toml or morphir.json found"))?
    };
    
    // Load config context
    let ctx = load_config_context(&config_file)
        .context("Failed to load configuration")?;
    
    // Ensure .morphir/ structure exists
    ensure_morphir_structure(&ctx.morphir_dir)
        .context("Failed to create .morphir/ directory structure")?;
    
    // Determine language (from CLI or config)
    let lang = language
        .or_else(|| ctx.config.frontend.as_ref().and_then(|f| f.language.clone()))
        .ok_or_else(|| anyhow::anyhow!("Language not specified and not found in config"))?;
    
    // Determine project name
    let proj_name = package_name
        .or_else(|| ctx.current_project.as_ref().map(|p| p.name.clone()))
        .or_else(|| ctx.config.project.as_ref().map(|p| p.name.clone()))
        .unwrap_or_else(|| "default".to_string());
    
    // Determine input path
    let input_path = if let Some(inp) = input {
        PathBuf::from(inp)
    } else {
        ctx.config.project.as_ref()
            .map(|p| PathBuf::from(&p.source_directory))
            .or_else(|| ctx.config.frontend.as_ref().and_then(|f| {
                f.settings.get("source_directory")
                    .and_then(|v| v.as_str())
                    .map(PathBuf::from)
            }))
            .unwrap_or_else(|| PathBuf::from("src"))
    };
    
    // Determine output path
    let output_path = if let Some(out) = output {
        PathBuf::from(out)
    } else {
        resolve_compile_output(&proj_name, &lang, &ctx.morphir_dir)
    };
    
    // Create extension registry
    let registry = ExtensionRegistry::new(
        ctx.project_root.unwrap_or_else(|| ctx.config_path.parent().unwrap().to_path_buf()),
        output_path.clone(),
    )
    .context("Failed to create extension registry")?;
    
    // Register builtin extensions
    let builtins = morphir_design::discover_builtin_extensions();
    for builtin in builtins {
        if let Some(path) = builtin.path {
            registry.register_builtin(&builtin.id, path).await
                .context(format!("Failed to register builtin extension: {}", builtin.id))?;
        }
    }
    
    // Find and load extension by language
    let extension = registry.find_extension_by_language(&lang).await
        .ok_or_else(|| anyhow::anyhow!("No extension found for language: {}", lang))?;
    
    // Collect source files
    let source_files = collect_source_files(&input_path, &lang)
        .context("Failed to collect source files")?;
    
    // Call extension's compile method
    let compile_params = serde_json::json!({
        "input": input_path.to_string_lossy(),
        "output": output_path.to_string_lossy(),
        "package_name": proj_name,
        "files": source_files,
    });
    
    let result: serde_json::Value = extension.call("morphir.frontend.compile", compile_params).await
        .context("Extension compile call failed")?;
    
    let format = OutputFormat::from_flags(json, json_lines);
    
    // Extract diagnostics and modules from result
    let diagnostics: Vec<Diagnostic> = result
        .get("diagnostics")
        .and_then(|d| serde_json::from_value(d.clone()).ok())
        .unwrap_or_default();
    
    let modules: Vec<String> = result
        .get("modules")
        .and_then(|m| serde_json::from_value(m.clone()).ok())
        .unwrap_or_default();
    
    let success = result
        .get("success")
        .and_then(|s| s.as_bool())
        .unwrap_or(true);
    
    if !success {
        let error_msg = result
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("Compilation failed");
        
        if format != OutputFormat::Human {
            let output = CompileOutput {
                success: false,
                ir: None,
                diagnostics: diagnostics.clone(),
                modules: vec![],
                output_path: output_path.to_string_lossy().to_string(),
            };
            write_output(format, &output)?;
        } else {
            let err = CliError::Compilation {
                message: error_msg.to_string(),
            };
            err.report();
        }
        return Err(anyhow::anyhow!("Compilation failed").into());
    }
    
    if format != OutputFormat::Human {
        let output = CompileOutput {
            success: true,
            ir: result.get("ir").cloned(),
            diagnostics,
            modules,
            output_path: output_path.to_string_lossy().to_string(),
        };
        write_output(format, &output)?;
    } else {
        println!("Compilation successful!");
        println!("Output: {:?}", output_path);
        if !diagnostics.is_empty() {
            println!("\nDiagnostics:");
            for diag in &diagnostics {
                println!("  {}: {}", diag.level, diag.message);
            }
        }
    }
    
    Ok(None)
}

/// Collect source files from input directory
fn collect_source_files(input_path: &Path, language: &str) -> anyhow::Result<Vec<String>> {
    let mut files = Vec::new();
    
    if !input_path.exists() {
        return Ok(files);
    }
    
    if input_path.is_file() {
        files.push(input_path.to_string_lossy().to_string());
        return Ok(files);
    }
    
    // Determine file extension based on language
    let ext = match language {
        "gleam" => "gleam",
        "elm" => "elm",
        "python" => "py",
        _ => return Err(anyhow::anyhow!("Unknown language: {}", language)),
    };
    
    // Walk directory and collect files
    for entry in walkdir::WalkDir::new(input_path) {
        let entry = entry?;
        if entry.file_type().is_file() {
            if let Some(file_ext) = entry.path().extension() {
                if file_ext == ext {
                    files.push(entry.path().to_string_lossy().to_string());
                }
            }
        }
    }
    
    Ok(files)
}
