//! Legacy `morphir.json` configuration normalization.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

/// Legacy `morphir.json` configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyProjectConfig {
    /// Project/package name.
    pub name: String,
    /// Directory containing project source files.
    pub source_directory: String,
    /// Modules exposed by the project.
    pub exposed_modules: Vec<String>,
    /// Version constraints for project dependencies.
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
    /// Legacy local dependency paths.
    #[serde(default)]
    pub local_dependencies: Vec<String>,
}

/// Normalize a legacy configuration into the current JSON configuration shape.
pub fn normalize_legacy_config(legacy: LegacyProjectConfig) -> Value {
    json!({
        "morphir": null,
        "project": {
            "name": legacy.name,
            "version": "0.1.0",
            "description": null,
            "authors": [],
            "license": null,
            "repository": null,
            "source_directory": legacy.source_directory,
            "exposed_modules": legacy.exposed_modules,
            "output_directory": ".morphir/out",
        },
        "workspace": null,
        "frontend": null,
        "ir": null,
        "codegen": null,
        "sources": null,
        "dependencies": legacy.dependencies,
        "dev-dependencies": {},
        "extensions": {},
        "tasks": {},
    })
}
