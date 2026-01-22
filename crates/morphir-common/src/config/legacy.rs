use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use super::model::{MorphirConfig, ProjectSection, DependencySpec};
use super::model::{default_output_dir}; 

/// Legacy `morphir.json` configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyProjectConfig {
    pub name: String,
    pub source_directory: String,
    pub exposed_modules: Vec<String>,
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
    #[serde(default)]
    pub local_dependencies: Vec<String>,
}

impl From<LegacyProjectConfig> for MorphirConfig {
    fn from(legacy: LegacyProjectConfig) -> Self {
        let project = ProjectSection {
            name: legacy.name,
            source_directory: legacy.source_directory,
            exposed_modules: legacy.exposed_modules,
            version: "0.1.0".to_string(), // Default for legacy
            authors: vec![],
            description: None,
            license: None,
            repository: None,
            output_directory: default_output_dir(),
        };

        // Convert simple string dependencies to DependencySpec
        let mut dependencies = HashMap::new();
        for (name, version) in legacy.dependencies {
            dependencies.insert(name, DependencySpec::Version(version));
        }

        MorphirConfig {
            project: Some(project),
            dependencies,
            ..Default::default()
        }
    }
}
