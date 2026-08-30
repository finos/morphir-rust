//! Configuration module
//!
//! Handles loading and parsing of Morphir configuration files, plus the
//! serialization-independent pieces of the effective-configuration algorithm:
//! [`merge`] implements the deep-merge rules, [`env`] maps `MORPHIR_*`
//! environment variables onto the configuration model, and [`redact`] hides
//! credentials before a configuration value is displayed.

/// Environment-variable configuration rules.
pub mod env {
    pub use morphir_config::env::*;
}
/// Legacy `morphir.json` normalization.
pub mod legacy {
    pub use morphir_config::legacy::*;
}
/// Serialization-independent configuration merge rules.
pub mod merge {
    pub use morphir_config::merge::*;
}
pub mod model;
pub mod redact;
/// External secret-reference recognition.
pub mod secret {
    pub use morphir_config::secret::*;
}

use anyhow::Context;
use serde_json::Value;
use std::path::Path;

pub use self::model::*;
pub use self::redact::redact_secrets;
pub use morphir_config::{
    ExposeSecret, ProvenanceMap, SecretReference, SecretReferenceError, SecretString, ValuePath,
    deep_merge, deep_merge_with_provenance, is_secret_reference, merge_all,
};

impl MorphirConfig {
    /// Load configuration from a file path
    pub fn load(path: &Path) -> crate::Result<Self> {
        Ok(serde_json::from_value(load_config_value(path)?)
            .with_context(|| format!("Failed to decode Morphir config: {}", path.display()))?)
    }
}

impl From<legacy::LegacyProjectConfig> for MorphirConfig {
    fn from(legacy: legacy::LegacyProjectConfig) -> Self {
        let project = ProjectSection {
            name: legacy.name,
            source_directory: legacy.source_directory,
            exposed_modules: legacy.exposed_modules,
            version: "0.1.0".to_string(),
            authors: vec![],
            description: None,
            license: None,
            repository: None,
            output_directory: model::default_output_dir(),
        };
        let dependencies = legacy
            .dependencies
            .into_iter()
            .map(|(name, version)| (name, DependencySpec::Version(version)))
            .collect();

        Self {
            project: Some(project),
            dependencies,
            ..Default::default()
        }
    }
}

/// Parse a Morphir configuration file into its serialization-independent value.
///
/// ```no_run
/// use morphir_common::config::load_config_value;
/// use std::path::Path;
///
/// # fn main() -> morphir_common::Result<()> {
/// let value = load_config_value(Path::new("morphir.yaml"))?;
/// assert_eq!(value["project"]["name"], "acme/orders");
/// # Ok(())
/// # }
/// ```
pub fn load_config_value(path: &Path) -> crate::Result<Value> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read Morphir config: {}", path.display()))?;
    Ok(morphir_config::parse_config(
        &path.to_string_lossy(),
        &content,
    )?)
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
