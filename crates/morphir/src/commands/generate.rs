//! Generate command for code generation from Morphir IR

use crate::error::{convert_extension_diagnostics, CliError};
use crate::output::{write_output, Diagnostic, GenerateOutput, OutputFormat};
use anyhow::Context;
use morphir_common::loader::load_ir;
use morphir_daemon::extensions::container::ExtensionType;
use morphir_daemon::extensions::registry::ExtensionRegistry;
use morphir_design::{
    discover_config, ensure_morphir_structure, load_config_context, resolve_generate_output,
};
use starbase::AppResult;
use std::path::{Path, PathBuf};

/// Run the generate command
pub async fn run_generate(
    target: Option<String>,
    input: Option<String>,
    output: Option<String>,
    config_path: Option<String>,
    project: Option<String>,
    json: bool,
    json_lines: bool,
) -> AppResult {
    use crate::output::{write_output, GenerateOutput, OutputFormat};
    // Discover config if not provided
    let start_dir = std::env::current_dir().context("Failed to get current directory")?;

    let config_file = if let Some(cfg) = config_path {
        PathBuf::from(cfg)
    } else {
        discover_config(&start_dir)
            .ok_or_else(|| anyhow::anyhow!("No morphir.toml or morphir.json found"))?
    };

    // Load config context
    let ctx = load_config_context(&config_file).context("Failed to load configuration")?;

    // Ensure .morphir/ structure exists
    ensure_morphir_structure(&ctx.morphir_dir)
        .context("Failed to create .morphir/ directory structure")?;

    // Determine target (from CLI or config)
    let target_lang = target
        .or_else(|| {
            ctx.config
                .codegen
                .as_ref()
                .and_then(|c| c.targets.first().cloned())
        })
        .ok_or_else(|| anyhow::anyhow!("Target not specified and not found in config"))?;

    // Determine project name
    let proj_name = ctx
        .current_project
        .as_ref()
        .map(|p| p.name.clone())
        .or_else(|| ctx.config.project.as_ref().map(|p| p.name.clone()))
        .unwrap_or_else(|| "default".to_string());

    // Determine IR input path
    let input_path = if let Some(inp) = input {
        PathBuf::from(inp)
    } else {
        // Default to compile output for the target language
        morphir_design::resolve_compile_output(&proj_name, &target_lang, &ctx.morphir_dir)
    };

    if !input_path.exists() {
        return Err(anyhow::anyhow!("IR input path does not exist: {:?}", input_path).into());
    }

    // Determine output path
    let output_path = if let Some(out) = output {
        PathBuf::from(out)
    } else {
        resolve_generate_output(&proj_name, &target_lang, &ctx.morphir_dir)
    };

    // Create extension registry
    let registry = ExtensionRegistry::new(
        ctx.project_root
            .unwrap_or_else(|| ctx.config_path.parent().unwrap().to_path_buf()),
        output_path.clone(),
    )
    .context("Failed to create extension registry")?;

    // Register builtin extensions
    let builtins = morphir_design::discover_builtin_extensions();
    for builtin in builtins {
        if let Some(path) = builtin.path {
            registry
                .register_builtin(&builtin.id, path)
                .await
                .context(format!(
                    "Failed to register builtin extension: {}",
                    builtin.id
                ))?;
        }
    }

    // Find and load extension by target
    let extension = registry
        .find_extension_by_target(&target_lang)
        .await
        .ok_or_else(|| anyhow::anyhow!("No extension found for target: {}", target_lang))?;

    // Load IR (detect format)
    let ir_data = load_ir(&input_path).context("Failed to load Morphir IR")?;

    // Call extension's generate method
    let generate_params = serde_json::json!({
        "input": input_path.to_string_lossy(),
        "output": output_path.to_string_lossy(),
        "ir": ir_data,
    });

    let result: serde_json::Value = extension
        .call("morphir.backend.generate", generate_params)
        .await
        .context("Extension generate call failed")?;

    let format = OutputFormat::from_flags(json, json_lines);

    // Extract diagnostics and artifacts from result
    let diagnostics: Vec<Diagnostic> = result
        .get("diagnostics")
        .and_then(|d| serde_json::from_value(d.clone()).ok())
        .unwrap_or_default();

    let artifacts: Vec<String> = result
        .get("artifacts")
        .and_then(|a| serde_json::from_value(a.clone()).ok())
        .unwrap_or_default();

    let success = result
        .get("success")
        .and_then(|s| s.as_bool())
        .unwrap_or(true);

    if !success {
        let error_msg = result
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("Code generation failed");

        if format != OutputFormat::Human {
            let output = GenerateOutput {
                success: false,
                artifacts: vec![],
                diagnostics: diagnostics.clone(),
                output_path: output_path.to_string_lossy().to_string(),
            };
            write_output(format, &output)?;
        } else {
            let err = CliError::Compilation {
                message: error_msg.to_string(),
            };
            err.report();
        }
        return Err(anyhow::anyhow!("Code generation failed").into());
    }

    if format != OutputFormat::Human {
        let output = GenerateOutput {
            success: true,
            artifacts,
            diagnostics,
            output_path: output_path.to_string_lossy().to_string(),
        };
        write_output(format, &output)?;
    } else {
        println!("Code generation successful!");
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
