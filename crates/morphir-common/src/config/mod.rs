//! Configuration module
//!
//! Handles loading and parsing of Morphir configuration files.

pub mod legacy;
pub mod model;

use self::legacy::LegacyProjectConfig;
use anyhow::{Context, anyhow};
use serde_json::Value;
use std::path::Path;

pub use self::model::*;

impl MorphirConfig {
    /// Load configuration from a file path
    pub fn load(path: &Path) -> crate::Result<Self> {
        serde_json::from_value(load_config_value(path)?)
            .with_context(|| format!("Failed to decode Morphir config: {}", path.display()))
    }
}

/// Parse a Morphir configuration file into its serialization-independent value.
pub fn load_config_value(path: &Path) -> crate::Result<Value> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read Morphir config: {}", path.display()))?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);

    match extension.as_deref() {
        Some("toml") => {
            let value: toml::Value = toml::from_str(&content)
                .with_context(|| format!("Failed to parse TOML config: {}", path.display()))?;
            serde_json::to_value(value).context("Failed to normalize TOML config")
        }
        Some("yaml" | "yml") => {
            validate_yaml_syntax(&content)?;
            let value: serde_yaml::Value = serde_yaml::from_str(&content)
                .with_context(|| format!("Failed to parse YAML config: {}", path.display()))?;
            validate_yaml_value(&value, true)?;
            serde_json::to_value(value).context("Failed to normalize YAML config")
        }
        Some("json") => {
            let legacy: LegacyProjectConfig =
                serde_json::from_str(&content).with_context(|| {
                    format!("Failed to parse legacy JSON config: {}", path.display())
                })?;
            serde_json::to_value(MorphirConfig::from(legacy))
                .context("Failed to normalize legacy JSON config")
        }
        _ => Err(anyhow!(
            "Unsupported Morphir config format for {} (expected .toml, .yaml, .yml, or .json)",
            path.display()
        )),
    }
}

fn validate_yaml_syntax(content: &str) -> crate::Result<()> {
    use yaml_rust::parser::{Event, Parser};

    let mut parser = Parser::new(content.chars());
    loop {
        let (event, _) = parser.next().context("Failed to scan YAML config")?;
        match event {
            Event::Alias(_) => return Err(anyhow!("YAML config must not contain aliases")),
            Event::Scalar(_, _, anchor, tag) => {
                if anchor != 0 {
                    return Err(anyhow!("YAML config must not contain anchors"));
                }
                if tag.is_some() {
                    return Err(anyhow!("YAML config must not contain custom tags"));
                }
            }
            Event::SequenceStart(anchor) | Event::MappingStart(anchor) if anchor != 0 => {
                return Err(anyhow!("YAML config must not contain anchors"));
            }
            Event::StreamEnd => return Ok(()),
            _ => {}
        }
    }
}

fn validate_yaml_value(value: &serde_yaml::Value, at_root: bool) -> crate::Result<()> {
    match value {
        serde_yaml::Value::Null => Err(anyhow!("YAML config must not contain null values")),
        serde_yaml::Value::Tagged(_) => Err(anyhow!("YAML config must not contain custom tags")),
        serde_yaml::Value::Mapping(mapping) => {
            for (key, value) in mapping {
                let key = key
                    .as_str()
                    .ok_or_else(|| anyhow!("YAML config mapping keys must be strings"))?;
                if key == "<<" {
                    return Err(anyhow!("YAML config must not use merge keys"));
                }
                validate_yaml_value(value, false)?;
            }
            Ok(())
        }
        serde_yaml::Value::Sequence(values) => {
            if at_root {
                return Err(anyhow!("YAML config root must be a mapping"));
            }
            values
                .iter()
                .try_for_each(|value| validate_yaml_value(value, false))
        }
        _ if at_root => Err(anyhow!("YAML config root must be a mapping")),
        _ => Ok(()),
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

    #[test]
    fn test_load_yaml() -> anyhow::Result<()> {
        let yaml_content = r#"
project:
  name: My.Project
  version: 1.0.0
  source_directory: src
  exposed_modules:
    - Foo
    - Bar
"#;
        let dir = tempfile::tempdir()?;
        let file_path = dir.path().join("morphir.yaml");
        std::fs::write(&file_path, yaml_content)?;

        let config = MorphirConfig::load(&file_path)?;
        let project = config.project.expect("project config");
        assert_eq!(project.name, "My.Project");
        assert_eq!(project.exposed_modules, vec!["Foo", "Bar"]);

        Ok(())
    }

    #[test]
    fn test_load_explicit_yml() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let file_path = dir.path().join("custom.yml");
        std::fs::write(
            &file_path,
            "project:\n  name: My.Project\n  version: 1.0.0\n",
        )?;

        let config = MorphirConfig::load(&file_path)?;
        assert!(config.is_project());

        Ok(())
    }

    #[test]
    fn test_rejects_unsupported_config_extension() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let file_path = dir.path().join("morphir.txt");
        std::fs::write(&file_path, "[project]\nname = \"My.Project\"")?;

        let error = MorphirConfig::load(&file_path).expect_err("unsupported extension");
        assert!(
            error
                .to_string()
                .contains("Unsupported Morphir config format")
        );

        Ok(())
    }

    #[test]
    fn test_rejects_yaml_null_values() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let file_path = dir.path().join("morphir.yaml");
        std::fs::write(&file_path, "project: null\n")?;

        let error = MorphirConfig::load(&file_path).expect_err("null value");
        assert!(error.to_string().contains("must not contain null"));

        Ok(())
    }

    #[test]
    fn test_rejects_non_mapping_yaml_root() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let file_path = dir.path().join("morphir.yaml");
        std::fs::write(&file_path, "- project\n")?;

        let error = MorphirConfig::load(&file_path).expect_err("sequence root");
        assert!(error.to_string().contains("root must be a mapping"));

        Ok(())
    }

    #[test]
    fn test_rejects_yaml_anchors_and_aliases() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let file_path = dir.path().join("morphir.yaml");
        std::fs::write(
            &file_path,
            "defaults: &defaults\n  language: elm\nfrontend: *defaults\n",
        )?;

        let error = MorphirConfig::load(&file_path).expect_err("anchor");
        assert!(error.to_string().contains("must not contain anchors"));

        Ok(())
    }

    #[test]
    fn test_rejects_yaml_custom_tags() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let file_path = dir.path().join("morphir.yaml");
        std::fs::write(&file_path, "project: !custom value\n")?;

        let error = MorphirConfig::load(&file_path).expect_err("custom tag");
        assert!(error.to_string().contains("must not contain custom tags"));

        Ok(())
    }

    #[test]
    fn test_rejects_duplicate_yaml_keys() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let file_path = dir.path().join("morphir.yaml");
        std::fs::write(&file_path, "project: {}\nproject: {}\n")?;

        MorphirConfig::load(&file_path).expect_err("duplicate key");

        Ok(())
    }

    #[test]
    fn test_rejects_multiple_yaml_documents() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let file_path = dir.path().join("morphir.yaml");
        std::fs::write(&file_path, "project: {}\n---\nproject: {}\n")?;

        MorphirConfig::load(&file_path).expect_err("multiple documents");

        Ok(())
    }
}
