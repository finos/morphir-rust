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
use starbase::AppResult;
use std::path::PathBuf;

/// Run the migrate command.
///
/// # Arguments
/// * `input` - Input file path or remote source
/// * `output` - Output file path
/// * `target_version` - Target format version ("v4" or "classic")
/// * `force_refresh` - Force refresh cached remote sources
/// * `no_cache` - Skip cache entirely for remote sources
pub fn run_migrate(
    input: String,
    output: PathBuf,
    target_version: Option<String>,
    force_refresh: bool,
    no_cache: bool,
) -> AppResult {
    // Parse input source
    let source = match RemoteSource::parse(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Invalid input source: {}", e);
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
                eprintln!("Failed to initialize source resolver: {}", e);
                return Ok(Some(1));
            }
        };

        // Check if source is allowed
        if !resolver.is_allowed(&source) {
            eprintln!("Source URL not allowed by configuration: {}", input);
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
                eprintln!("Failed to fetch source: {}", e);
                return Ok(Some(1));
            }
        }
    };

    println!("Migrating IR from {:?} to {:?}", local_path, output);

    let vfs = OsVfs;

    // Load input
    let dist = match load_distribution(&vfs, &local_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to load input: {}", e);
            return Ok(Some(1));
        }
    };

    // Convert
    let target_v4 = target_version.as_deref() == Some("v4") || target_version.is_none();

    match dist {
        LoadedDistribution::Classic(dist) => {
            if target_v4 {
                println!("Converting Classic -> V4");
                let classic::DistributionBody::Library(_, package_path, _, pkg) = dist.distribution;
                let v4_pkg = converter::classic_to_v4(pkg);

                // Wrap in V4 IRFile
                let v4_ir = v4::IRFile {
                    format_version: v4::FormatVersion::default(),
                    distribution: v4::Distribution::Library(v4::LibraryContent {
                        package_name: PackageName::from(package_path),
                        dependencies: IndexMap::new(),
                        def: v4_pkg,
                    }),
                };

                // Save v4_ir
                let content = serde_json::to_string_pretty(&v4_ir).expect("Failed to serialize");
                std::fs::write(&output, content).expect("Failed to write output");
            } else {
                println!("Input is Classic, Target is Classic. Copying...");
                let content = serde_json::to_string_pretty(&dist).expect("Failed to serialize");
                std::fs::write(&output, content).expect("Failed to write output");
            }
        }
        LoadedDistribution::V4(ir_file) => {
            if !target_v4 {
                println!("Converting V4 -> Classic");
                let v4::Distribution::Library(lib_content) = ir_file.distribution else {
                    eprintln!("Only Library distributions can be converted to Classic format");
                    return Ok(Some(1));
                };

                let classic_pkg = converter::v4_to_classic(lib_content.def);

                // Wrap in Classic Distribution
                let classic_dist = classic::Distribution {
                    format_version: 1,
                    distribution: classic::DistributionBody::Library(
                        classic::LibraryTag::Library,
                        lib_content.package_name.into_path(),
                        vec![],
                        classic_pkg,
                    ),
                };

                let content =
                    serde_json::to_string_pretty(&classic_dist).expect("Failed to serialize");
                std::fs::write(&output, content).expect("Failed to write output");
            } else {
                println!("Input is V4, Target is V4. Copying...");
                let content = serde_json::to_string_pretty(&ir_file).expect("Failed to serialize");
                std::fs::write(&output, content).expect("Failed to write output");
            }
        }
    }

    println!("Migration complete.");
    Ok(None)
}
