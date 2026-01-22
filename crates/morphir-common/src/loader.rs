use crate::vfs::Vfs;
use anyhow::{Context, Result};
use morphir_ir::ir::{classic, v4};
use std::path::Path;

#[derive(Debug)]
pub enum LoadedDistribution {
    V4(v4::Distribution),
    Classic(classic::Distribution),
}

pub fn load_distribution(vfs: &impl Vfs, path: &Path) -> Result<LoadedDistribution> {
    if vfs.is_dir(path) {
        return load_v4_from_dir(vfs, path);
    }

    let content = vfs.read_to_string(path)?;

    if let Ok(dist) = serde_json::from_str::<v4::Distribution>(&content) {
        if dist.format_version == 4 {
            return Ok(LoadedDistribution::V4(dist));
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
    let mut modules = Vec::new();

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

            let module_entry = v4::ModuleDefinitionEntry(
                v4::Path::new(&module_name),
                v4::AccessControlledModuleDefinition {
                    access: v4::Access::Public,
                    value: v4::ModuleDefinition {
                        types: vec![],
                        values: vec![],
                        doc: None,
                    },
                },
            );
            modules.push(module_entry);
        }
    }

    let dist = v4::Distribution {
        format_version: 4,
        distribution: v4::DistributionBody::Library(v4::LibraryDistribution(
            v4::LibraryTag::Library,
            v4::Path::new(&package_name),
            vec![],
            v4::PackageDefinition { modules },
        )),
    };

    Ok(LoadedDistribution::V4(dist))
}
