//! Migrate Command
//!
//! Command to migrate Morphir IR between versions and formats.

use indexmap::IndexMap;
use morphir_common::loader::{load_distribution, LoadedDistribution};
use morphir_common::remote::{RemoteSource, RemoteSourceResolver, ResolveOptions};
use morphir_common::vfs::OsVfs;
use morphir_ir::converter;
use morphir_ir::ir::{classic, v4};
use morphir_ir::naming::PackageName;
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

/// Run the migrate command.
///
/// # Arguments
/// * `input` - Input file path or remote source
/// * `output` - Output file path
/// * `target_version` - Target format version ("v4" or "classic")
/// * `force_refresh` - Force refresh cached remote sources
/// * `no_cache` - Skip cache entirely for remote sources
/// * `json` - Output result as JSON
pub fn run_migrate(
    input: String,
    output: PathBuf,
    target_version: Option<String>,
    force_refresh: bool,
    no_cache: bool,
    json: bool,
) -> AppResult {
    let output_str = output.display().to_string();
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
        println!("Migrating IR from {:?} to {:?}", local_path, output);
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
    let target_v4 = target_version.as_deref() == Some("v4") || target_version.is_none();
    let target_format = if target_v4 { "v4" } else { "classic" };

    match dist {
        LoadedDistribution::Classic(dist) => {
            let source_format = "classic";
            if target_v4 {
                if !json {
                    println!("Converting Classic -> V4");
                }
                let classic::DistributionBody::Library(_, package_path, classic_deps, pkg) =
                    dist.distribution;

                // Warn if dependencies will be lost (Classic deps format differs from V4)
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

                let v4_pkg = converter::classic_to_v4(pkg);

                // Wrap in V4 IRFile
                let v4_ir = v4::IRFile {
                    format_version: v4::FormatVersion::default(),
                    distribution: v4::Distribution::Library(v4::LibraryContent {
                        package_name: PackageName::from(package_path),
                        dependencies: IndexMap::new(), // TODO: Convert classic deps when format is understood
                        def: v4_pkg,
                    }),
                };

                // Save v4_ir
                let content = serde_json::to_string_pretty(&v4_ir).expect("Failed to serialize");
                std::fs::write(&output, content).expect("Failed to write output");

                if json {
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
                    println!("Input is Classic, Target is Classic. Copying...");
                }
                let content = serde_json::to_string_pretty(&dist).expect("Failed to serialize");
                std::fs::write(&output, content).expect("Failed to write output");

                if json {
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
                    println!("Converting V4 -> Classic");
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
                std::fs::write(&output, content).expect("Failed to write output");

                if json {
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
                    println!("Input is V4, Target is V4. Copying...");
                }
                let content = serde_json::to_string_pretty(&ir_file).expect("Failed to serialize");
                std::fs::write(&output, content).expect("Failed to write output");

                if json {
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
        println!("Migration complete.");
    }
    Ok(None)
}
