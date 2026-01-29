//! Migrate Command
//!
//! Command to migrate Morphir IR between versions and formats.

use crate::tui::JsonPager;
use indexmap::IndexMap;
use morphir_common::loader::{LoadedDistribution, load_distribution};
use morphir_common::remote::{RemoteSource, RemoteSourceResolver, ResolveOptions};
use morphir_common::vfs::OsVfs;
use morphir_core::ir::{classic, v4};
use serde::Serialize;
use starbase::AppResult;
use std::path::PathBuf;

/// JSON output for migrate command
#[derive(Serialize)]
struct MigrateResult {
    success: bool,
    input: String,
    output: String,
    source_format: String,
    target_format: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl MigrateResult {
    fn success(
        input: &str,
        output: &str,
        source_format: &str,
        target_format: &str,
        warnings: Vec<String>,
    ) -> Self {
        Self {
            success: true,
            input: input.to_string(),
            output: output.to_string(),
            source_format: source_format.to_string(),
            target_format: target_format.to_string(),
            warnings,
            error: None,
        }
    }

    fn error(input: &str, output: &str, error: &str) -> Self {
        Self {
            success: false,
            input: input.to_string(),
            output: output.to_string(),
            source_format: String::new(),
            target_format: String::new(),
            warnings: Vec::new(),
            error: Some(error.to_string()),
        }
    }
}

/// Display JSON content using the ratatui-based pager with syntax highlighting.
fn display_json_in_pager(content: &str, title: &str) -> std::io::Result<()> {
    let pager = JsonPager::new(content.to_string(), title.to_string());
    pager.run()
}

/// Write content to output file or display in pager with syntax highlighting.
fn write_or_display(output: &Option<PathBuf>, content: &str, json_mode: bool, title: &str) {
    match output {
        Some(path) => {
            std::fs::write(path, content).expect("Failed to write output");
        }
        None => {
            if !json_mode {
                // Display in pager with syntax highlighting (like bat)
                if let Err(e) = display_json_in_pager(content, title) {
                    eprintln!("Failed to display output: {}", e);
                    // Fallback to plain output
                    println!("{}", content);
                }
            } else {
                // In JSON mode with no output file, emit the migrated IR to stdout
                println!("{}", content);
            }
        }
    }
}

/// Resolve target version string to a normalized format.
/// Returns (is_v4, format_name) where format_name is either "v4" or "classic".
fn resolve_target_version(version: &str) -> Result<(bool, &'static str), String> {
    match version.to_lowercase().as_str() {
        // Latest always resolves to the newest format
        "latest" => Ok((true, "v4")),
        // V4 format
        "v4" | "4" => Ok((true, "v4")),
        // Classic formats (V1, V2, V3) all map to "classic"
        "classic" | "v3" | "3" | "v2" | "2" | "v1" | "1" => Ok((false, "classic")),
        _ => Err(format!(
            "Invalid target version '{}'. Valid values: latest, v4, 4, classic, v3, 3, v2, 2, v1, 1",
            version
        )),
    }
}

/// Run the migrate command.
///
/// # Arguments
/// * `input` - Input file path or remote source
/// * `output` - Output file path
/// * `target_version` - Target format version ("latest", "v4", or "classic")
/// * `force_refresh` - Force refresh cached remote sources
/// * `no_cache` - Skip cache entirely for remote sources
/// * `json` - Output result as JSON
/// * `expanded` - Use expanded (non-compact) format for V4 output
pub fn run_migrate(
    input: String,
    output: Option<PathBuf>,
    target_version: String,
    force_refresh: bool,
    no_cache: bool,
    json: bool,
    expanded: bool,
) -> AppResult {
    let output_str = output
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<console>".to_string());
    let mut warnings: Vec<String> = Vec::new();

    // Helper to output error
    let output_error = |msg: &str| {
        if json {
            let result = MigrateResult::error(&input, &output_str, msg);
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
        } else {
            eprintln!("{}", msg);
        }
    };

    // Parse input source
    let source = match RemoteSource::parse(&input) {
        Ok(s) => s,
        Err(e) => {
            output_error(&format!("Invalid input source: {}", e));
            return Ok(Some(1));
        }
    };

    // Resolve source to local path
    let local_path = if source.is_local() {
        // Local path - use directly
        PathBuf::from(&input)
    } else {
        // Remote source - resolve using resolver
        let mut resolver = match RemoteSourceResolver::with_defaults() {
            Ok(r) => r,
            Err(e) => {
                output_error(&format!("Failed to initialize source resolver: {}", e));
                return Ok(Some(1));
            }
        };

        // Check if source is allowed
        if !resolver.is_allowed(&source) {
            output_error(&format!(
                "Source URL not allowed by configuration: {}",
                input
            ));
            return Ok(Some(1));
        }

        let options = if no_cache {
            ResolveOptions::no_cache()
        } else if force_refresh {
            ResolveOptions::force_refresh()
        } else {
            ResolveOptions::new()
        };

        match resolver.resolve(&source, &options) {
            Ok(path) => path,
            Err(e) => {
                output_error(&format!("Failed to fetch source: {}", e));
                return Ok(Some(1));
            }
        }
    };

    if !json {
        match &output {
            Some(path) => eprintln!("Migrating IR from {:?} to {:?}", local_path, path),
            None => eprintln!("Migrating IR from {:?} (displaying to console)", local_path),
        }
    }

    let vfs = OsVfs;

    // Load input
    let dist = match load_distribution(&vfs, &local_path) {
        Ok(d) => d,
        Err(e) => {
            output_error(&format!("Failed to load input: {}", e));
            return Ok(Some(1));
        }
    };

    // Convert
    // Resolve target version
    let (target_v4, target_format) = match resolve_target_version(&target_version) {
        Ok(result) => result,
        Err(msg) => {
            output_error(&msg);
            return Ok(Some(1));
        }
    };

    match dist {
        LoadedDistribution::Classic(dist) => {
            let source_format = "classic";
            if target_v4 {
                if !json {
                    eprintln!("Converting Classic -> V4");
                }
                let classic::DistributionBody::Library(_, package_path, classic_deps, pkg) =
                    dist.distribution;

                // Warn if dependencies will be lost (a Classic deps format differs from V4)
                if !classic_deps.is_empty() {
                    let warn = format!(
                        "{} dependencies found in Classic format. \
                         Dependency conversion is not yet supported and will be omitted.",
                        classic_deps.len()
                    );
                    if !json {
                        eprintln!("Warning: {}", warn);
                    }
                    warnings.push(warn);
                }

                let v4_pkg = converter::classic_to_v4_with_options(pkg, !expanded);

                // Wrap in V4 IRFile
                let v4_ir = v4::IRFile {
                    format_version: v4::FormatVersion::default(),
                    distribution: v4::Distribution::Library(v4::LibraryContent {
                        package_name: PackageName::from(package_path),
                        dependencies: IndexMap::new(), // TODO: Convert classic deps when format is understood
                        def: v4_pkg,
                    }),
                };

                // Save or display v4_ir
                let content = serde_json::to_string_pretty(&v4_ir).expect("Failed to serialize");
                let title = format!("morphir-ir.json (V4 format, migrated from {})", input);
                write_or_display(&output, &content, json, &title);

                if json && output.is_some() {
                    // Only print MigrateResult to stdout when output file is specified
                    // When no output file, the migrated IR goes to stdout and metadata to stderr
                    let result = MigrateResult::success(
                        &input,
                        &output_str,
                        source_format,
                        target_format,
                        warnings,
                    );
                    println!("{}", serde_json::to_string_pretty(&result).unwrap());
                }
            } else {
                if !json {
                    eprintln!("Input is Classic, Target is Classic. Copying...");
                }
                let content = serde_json::to_string_pretty(&dist).expect("Failed to serialize");
                let title = format!("morphir-ir.json (Classic format, from {})", input);
                write_or_display(&output, &content, json, &title);

                if json && output.is_some() {
                    let result = MigrateResult::success(
                        &input,
                        &output_str,
                        source_format,
                        target_format,
                        warnings,
                    );
                    println!("{}", serde_json::to_string_pretty(&result).unwrap());
                }
            }
        }
        LoadedDistribution::V4(ir_file) => {
            let source_format = "v4";
            if !target_v4 {
                if !json {
                    eprintln!("Converting V4 -> Classic");
                }
                let v4::Distribution::Library(lib_content) = ir_file.distribution else {
                    output_error("Only Library distributions can be converted to Classic format");
                    return Ok(Some(1));
                };

                // Warn if dependencies will be lost
                if !lib_content.dependencies.is_empty() {
                    let warn = format!(
                        "{} dependencies found in V4 format. \
                         Dependency conversion is not yet supported and will be omitted.",
                        lib_content.dependencies.len()
                    );
                    if !json {
                        eprintln!("Warning: {}", warn);
                    }
                    warnings.push(warn);
                }

                let classic_pkg = converter::v4_to_classic(lib_content.def);

                // Wrap in Classic Distribution
                let classic_dist = classic::Distribution {
                    format_version: 1,
                    distribution: classic::DistributionBody::Library(
                        classic::LibraryTag::Library,
                        lib_content.package_name.into_path(),
                        vec![], // TODO: Convert V4 deps to classic format
                        classic_pkg,
                    ),
                };

                let content =
                    serde_json::to_string_pretty(&classic_dist).expect("Failed to serialize");
                let title = format!("morphir-ir.json (Classic format, migrated from {})", input);
                write_or_display(&output, &content, json, &title);

                if json && output.is_some() {
                    let result = MigrateResult::success(
                        &input,
                        &output_str,
                        source_format,
                        target_format,
                        warnings,
                    );
                    println!("{}", serde_json::to_string_pretty(&result).unwrap());
                }
            } else {
                if !json {
                    eprintln!("Input is V4, Target is V4. Copying...");
                }
                let content = serde_json::to_string_pretty(&ir_file).expect("Failed to serialize");
                let title = format!("morphir-ir.json (V4 format, from {})", input);
                write_or_display(&output, &content, json, &title);

                if json && output.is_some() {
                    let result = MigrateResult::success(
                        &input,
                        &output_str,
                        source_format,
                        target_format,
                        warnings,
                    );
                    println!("{}", serde_json::to_string_pretty(&result).unwrap());
                }
            }
        }
    }

    if !json {
        eprintln!("Migration complete.");
    }
    Ok(None)
}
