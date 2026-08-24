//! Driver for configuration scenarios: loading files, merging values, and
//! mapping environment variables.

use anyhow::Result;
use morphir_common::config::env::env_config_value;
use morphir_common::config::{MorphirConfig, deep_merge};
use serde_json::Value;
use std::path::PathBuf;
use tempfile::TempDir;

const ENV_PREFIX: &str = "MORPHIR";

/// Encapsulates the configuration system under test and the state a scenario
/// builds up while exercising it.
#[derive(Debug, Default)]
pub struct ConfigDriver {
    temp_dir: Option<TempDir>,
    config_path: Option<PathBuf>,
    loaded_config: Option<MorphirConfig>,
    last_result: Option<Result<()>>,
    base_value: Option<Value>,
    overlay_value: Option<Value>,
    merged_value: Option<Value>,
    env_vars: Vec<(String, String)>,
}

impl ConfigDriver {
    // --- Configuration files -------------------------------------------------

    /// Write a configuration file into a fresh temporary directory.
    pub fn given_config_file(&mut self, filename: &str, content: &str) {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let path = dir.path().join(filename);
        std::fs::write(&path, content).expect("Failed to write config file");
        self.config_path = Some(path);
        self.temp_dir = Some(dir);
    }

    /// Load the configuration file written by [`Self::given_config_file`].
    pub fn when_loading_config(&mut self) {
        let path = self
            .config_path
            .as_ref()
            .expect("A configuration file must be given first");
        match MorphirConfig::load(path) {
            Ok(config) => {
                self.loaded_config = Some(config);
                self.last_result = Some(Ok(()));
            }
            Err(error) => self.last_result = Some(Err(error)),
        }
    }

    /// The successfully loaded configuration.
    pub fn loaded_config(&self) -> &MorphirConfig {
        self.loaded_config.as_ref().unwrap_or_else(|| {
            panic!(
                "Configuration was not loaded: {:?}",
                self.last_result
                    .as_ref()
                    .and_then(|result| result.as_ref().err())
            )
        })
    }

    // --- Merge rules ---------------------------------------------------------

    /// Provide the lower-precedence value for a merge.
    pub fn given_base_value(&mut self, value: Value) {
        self.base_value = Some(value);
    }

    /// Provide the higher-precedence value for a merge.
    pub fn given_overlay_value(&mut self, value: Value) {
        self.overlay_value = Some(value);
    }

    /// Merge the overlay onto the base value.
    pub fn when_merging(&mut self) {
        let base = self.base_value.as_ref().expect("Base value required");
        let overlay = self.overlay_value.as_ref().expect("Overlay value required");
        self.merged_value = Some(deep_merge(base, overlay));
    }

    /// The base value as it is after any merge (merging must not mutate it).
    pub fn base_value(&self) -> Option<&Value> {
        self.base_value.as_ref()
    }

    // --- Environment variables ----------------------------------------------

    /// Record an environment variable for the scenario.
    pub fn given_env_var(&mut self, name: &str, value: &str) {
        self.env_vars.push((name.to_string(), value.to_string()));
    }

    /// Map the recorded environment variables onto the configuration model.
    pub fn when_loading_environment(&mut self) {
        self.merged_value = Some(env_config_value(
            ENV_PREFIX,
            self.env_vars
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        ));
    }

    // --- Assertions ----------------------------------------------------------

    /// Look up a dotted path (`ir.format_version`) in the merged value.
    pub fn merged_value_at(&self, path: &str) -> Option<&Value> {
        let pointer = format!("/{}", path.replace('.', "/"));
        self.merged_value
            .as_ref()
            .expect("Nothing has been merged or loaded yet")
            .pointer(&pointer)
    }

    /// The whole merged value, for diagnostics in assertion messages.
    pub fn merged_value(&self) -> Option<&Value> {
        self.merged_value.as_ref()
    }
}
