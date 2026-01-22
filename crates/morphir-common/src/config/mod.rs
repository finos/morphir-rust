//! Configuration module
//!
//! Handles loading and parsing of Morphir configuration files (morphir.toml, morphir.json).

pub mod model;
pub mod legacy;

use std::path::PathBuf;
use self::legacy::LegacyProjectConfig;

pub use self::model::*;

impl MorphirConfig {
    /// Load configuration from a file path
    pub fn load(path: &PathBuf) -> crate::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        
        // Detect format based on extension
        if let Some(ext) = path.extension() {
            if ext == "json" {
                let legacy: LegacyProjectConfig = serde_json::from_str(&content)?;
                return Ok(legacy.into());
            }
        }
        
        // Default to TOML
        let config: MorphirConfig = toml::from_str(&content)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_load_toml() -> anyhow::Result<()> {
        let toml_content = r#"
[project]
name = "My.Project"
version = "1.0.0"
source_directory = "src"
exposed_modules = ["Foo", "Bar"]
"#;
        let mut file = NamedTempFile::new()?;
        write!(file, "{}", toml_content)?;
        
        let dir = tempfile::tempdir()?;
        let file_path = dir.path().join("morphir.toml");
        std::fs::write(&file_path, toml_content)?;

        let config = MorphirConfig::load(&file_path)?;
        assert!(config.is_project());
        let project = config.project.unwrap();
        assert_eq!(project.name, "My.Project");
        assert_eq!(project.version, "1.0.0");
        
        Ok(())
    }

    #[test]
    fn test_load_legacy_json() -> anyhow::Result<()> {
        let json_content = r#"{
    "name": "Legacy.Project",
    "sourceDirectory": "source",
    "exposedModules": ["A", "B"],
    "dependencies": {
        "finos/morphir-dapr": "0.1.0"
    }
}"#;
        let dir = tempfile::tempdir()?;
        let file_path = dir.path().join("morphir.json");
        std::fs::write(&file_path, json_content)?;

        let config = MorphirConfig::load(&file_path)?;
        assert!(config.is_project());
        let project = config.project.unwrap();
        assert_eq!(project.name, "Legacy.Project");
        assert_eq!(project.source_directory, "source");
        assert_eq!(project.exposed_modules, vec!["A", "B"]);
        
        // Check dependencies
        assert!(config.dependencies.contains_key("finos/morphir-dapr"));
        
        Ok(())
    }
}
