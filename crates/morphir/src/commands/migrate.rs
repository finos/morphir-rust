//! Migrate Command
//!
//! Command to migrate Morphir IR between versions and formats.

use starbase::AppResult;
use morphir_common::vfs::OsVfs;
use morphir_common::loader::{load_distribution, LoadedDistribution};
use morphir_ir::converter;
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
        LoadedDistribution::Classic(pkg) => {
            if target_v4 {
                println!("Converting Classic -> V4");
                let v4_dist = converter::classic_to_v4(pkg);
                // Save v4_dist
                let content = serde_json::to_string_pretty(&v4_dist).expect("Failed to serialize");
                // TODO: Write to output using VFS or std::fs
                std::fs::write(&output, content).expect("Failed to write output");
            } else {
                println!("Input is Classic, Target is Classic. Copying...");
                let content = serde_json::to_string_pretty(&pkg).expect("Failed to serialize");
                std::fs::write(&output, content).expect("Failed to write output");
            }
        }
        LoadedDistribution::V4(dist) => {
             if !target_v4 {
                println!("Converting V4 -> Classic");
                let classic_pkg = converter::v4_to_classic(dist);
                let content = serde_json::to_string_pretty(&classic_pkg).expect("Failed to serialize");
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
