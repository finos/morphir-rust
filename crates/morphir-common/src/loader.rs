use crate::remote::{RemoteSource, RemoteSourceResolver, ResolveOptions};
use crate::vfs::{OsVfs, Vfs};
use anyhow::{Context, Result};
use indexmap::IndexMap;
use morphir_ir::ir::{classic, v4};
use morphir_ir::naming::PackageName;
use std::path::Path;

#[derive(Debug)]
pub enum LoadedDistribution {
    V4(v4::IRFile),
    Classic(classic::Distribution),
}

/// Load distribution from a source string (local path or remote source).
///
/// This is a convenience function that:
/// 1. Parses the source string into a RemoteSource
/// 2. Resolves it to a local path (downloading if necessary)
/// 3. Loads the distribution from the local path
///
/// # Arguments
/// * `source` - A source string (local path, URL, or shorthand like `github:owner/repo`)
///
/// # Examples
/// ```ignore
/// // Local file
/// let dist = load_distribution_from_source("./morphir-ir.json")?;
///
/// // Remote URL
/// let dist = load_distribution_from_source("https://example.com/morphir-ir.json")?;
///
/// // GitHub shorthand
/// let dist = load_distribution_from_source("github:finos/morphir-examples/examples/basic")?;
/// ```
pub fn load_distribution_from_source(source: &str) -> Result<LoadedDistribution> {
    load_distribution_from_source_with_options(source, &ResolveOptions::new())
}

/// Load distribution from a source string with custom resolve options.
pub fn load_distribution_from_source_with_options(
    source: &str,
    options: &ResolveOptions,
) -> Result<LoadedDistribution> {
    let remote_source =
        RemoteSource::parse(source).map_err(|e| anyhow::anyhow!("Invalid source: {}", e))?;

    let local_path = if remote_source.is_local() {
        std::path::PathBuf::from(source)
    } else {
        let mut resolver = RemoteSourceResolver::with_defaults()
            .map_err(|e| anyhow::anyhow!("Failed to create source resolver: {}", e))?;

        resolver
            .resolve(&remote_source, options)
            .map_err(|e| anyhow::anyhow!("Failed to resolve source: {}", e))?
    };

    let vfs = OsVfs;
    load_distribution(&vfs, &local_path)
}

pub fn load_distribution(vfs: &impl Vfs, path: &Path) -> Result<LoadedDistribution> {
    if vfs.is_dir(path) {
        return load_v4_from_dir(vfs, path);
    }

    let content = vfs.read_to_string(path)?;

    if let Ok(ir_file) = serde_json::from_str::<v4::IRFile>(&content) {
        // Check if it's a V4 format based on format_version
        let is_v4 = match &ir_file.format_version {
            v4::FormatVersion::Integer(n) => *n >= 4,
            v4::FormatVersion::String(s) => s.starts_with("4"),
        };

        if is_v4 {
            return Ok(LoadedDistribution::V4(ir_file));
        }
    }

    let classic_dist: classic::Distribution = serde_json::from_str(&content)
        .context("Failed to parse distribution as either V4 or Classic IR")?;

    Ok(LoadedDistribution::Classic(classic_dist))
}

fn load_v4_from_dir(vfs: &impl Vfs, path: &Path) -> Result<LoadedDistribution> {
    // Read morphir.json from the directory root to get package name
    let morphir_json_path = path.join("morphir.json");
    let package_name = if vfs.exists(&morphir_json_path) {
        let content = vfs.read_to_string(&morphir_json_path)?;
        let config: serde_json::Value =
            serde_json::from_str(&content).context("Failed to parse morphir.json")?;
        config
            .get("name")
            .and_then(|n| n.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown-package".to_string())
    } else {
        "unknown-package".to_string()
    };

    // Scan for module JSON files in src/ directory
    let src_path = path.join("src");
    let mut modules: IndexMap<String, v4::AccessControlledModuleDefinition> = IndexMap::new();

    if vfs.is_dir(&src_path) {
        // Use glob to find all JSON files under src/
        let pattern = "src/**/*.json";
        let json_files = vfs.glob(pattern).unwrap_or_default();

        for file_path in json_files {
            // Skip Package.json files (metadata only)
            if file_path
                .file_name()
                .map(|n| n == "Package.json")
                .unwrap_or(false)
            {
                continue;
            }

            // Derive module name from path: src/Test/Module.json -> Test.Module
            let relative = file_path.strip_prefix("src/").unwrap_or(&file_path);
            let module_name = relative
                .with_extension("")
                .to_string_lossy()
                .replace('/', ".");

            let module_def = v4::AccessControlledModuleDefinition {
                access: v4::Access::Public,
                value: v4::ModuleDefinition {
                    types: IndexMap::new(),
                    values: IndexMap::new(),
                    doc: None,
                },
            };
            modules.insert(module_name, module_def);
        }
    }

    let ir_file = v4::IRFile {
        format_version: v4::FormatVersion::default(),
        distribution: v4::Distribution::Library(v4::LibraryContent {
            package_name: PackageName::from_str(&package_name),
            dependencies: IndexMap::new(),
            def: v4::PackageDefinition { modules },
        }),
    };

    Ok(LoadedDistribution::V4(ir_file))
}
