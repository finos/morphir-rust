//! Migrate Command
//!
//! Command to migrate Morphir IR between versions and formats.

use morphir_common::loader::{load_distribution, LoadedDistribution};
use morphir_common::vfs::OsVfs;
use morphir_ir::converter;
use morphir_ir::ir::{classic, v4};
use morphir_ir::naming::PackageName;
use starbase::AppResult;
use std::path::PathBuf;

pub fn run_migrate(input: PathBuf, output: PathBuf, target_version: Option<String>) -> AppResult {
    let vfs = OsVfs;
    println!("Migrating IR from {:?} to {:?}", input, output);

    // Load input
    let dist = match load_distribution(&vfs, &input) {
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

                // Wrap in V4 Distribution
                let v4_dist = v4::Distribution {
                    format_version: 4,
                    distribution: v4::DistributionBody::Library(v4::LibraryDistribution(
                        v4::LibraryTag::Library,
                        PackageName::from(package_path),
                        vec![],
                        v4_pkg,
                    )),
                };

                // Save v4_dist
                let content = serde_json::to_string_pretty(&v4_dist).expect("Failed to serialize");
                std::fs::write(&output, content).expect("Failed to write output");
            } else {
                println!("Input is Classic, Target is Classic. Copying...");
                let content = serde_json::to_string_pretty(&dist).expect("Failed to serialize");
                std::fs::write(&output, content).expect("Failed to write output");
            }
        }
        LoadedDistribution::V4(dist) => {
            if !target_v4 {
                println!("Converting V4 -> Classic");
                let v4::DistributionBody::Library(lib_dist) = dist.distribution;
                let v4::LibraryDistribution(_, package_name, _, pkg_def) = lib_dist;

                let classic_pkg = converter::v4_to_classic(pkg_def);

                // Wrap in Classic Distribution
                let classic_dist = classic::Distribution {
                    format_version: 1,
                    distribution: classic::DistributionBody::Library(
                        classic::LibraryTag::Library,
                        package_name.into_path(),
                        vec![],
                        classic_pkg,
                    ),
                };

                let content =
                    serde_json::to_string_pretty(&classic_dist).expect("Failed to serialize");
                std::fs::write(&output, content).expect("Failed to write output");
            } else {
                println!("Input is V4, Target is V4. Copying...");
                let content = serde_json::to_string_pretty(&dist).expect("Failed to serialize");
                std::fs::write(&output, content).expect("Failed to write output");
            }
        }
    }

    println!("Migration complete.");
    Ok(None)
}
