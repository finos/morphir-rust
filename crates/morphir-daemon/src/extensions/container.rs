//! Extension container using Extism
//!
//! This module provides the runtime container for loaded extensions.

use crate::error::{DaemonError, Result};
use crate::extensions::host_functions::MorphirHostFunctions;
use crate::extensions::protocol::{ExtensionRequest, ExtensionResponse};
use extism::{Manifest, Plugin, Wasm};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Information about a loaded extension
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExtensionInfo {
    /// Extension identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Version
    pub version: String,
    /// Description
    #[serde(default)]
    pub description: Option<String>,
    /// Extension types/capabilities
    #[serde(default)]
    pub types: Vec<ExtensionType>,
    /// Author
    #[serde(default)]
    pub author: Option<String>,
    /// License
    #[serde(default)]
    pub license: Option<String>,
}

/// Type of extension capability
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtensionType {
    Frontend,
    Backend,
    Transform,
    Validator,
}

/// Container for a loaded extension plugin
pub struct ExtensionContainer {
    /// Extension identifier
    id: String,
    /// The Extism plugin instance
    plugin: Arc<RwLock<Plugin>>,
    /// Extension metadata
    info: ExtensionInfo,
    /// Request ID counter
    request_id: std::sync::atomic::AtomicU64,
}

impl ExtensionContainer {
    /// Create a new extension container from a WASM file
    pub fn new(id: &str, wasm_path: &Path, host_funcs: MorphirHostFunctions) -> Result<Self> {
        info!("Loading extension '{}' from {:?}", id, wasm_path);

        // Read the WASM file
        let wasm_bytes = std::fs::read(wasm_path)?;

        Self::from_bytes(id, &wasm_bytes, host_funcs)
    }

    /// Create a new extension container from WASM bytes
    pub fn from_bytes(
        id: &str,
        wasm_bytes: &[u8],
        host_funcs: MorphirHostFunctions,
    ) -> Result<Self> {
        // Create manifest with memory limits
        let manifest = Manifest::new([Wasm::data(wasm_bytes)]).with_memory_max(256 * 1024 * 1024); // 256 MB max

        // Create plugin with host functions
        let mut plugin = Plugin::new(&manifest, host_funcs.into_functions(), true)
            .map_err(|e| DaemonError::Extension(format!("Failed to create plugin: {}", e)))?;

        // Query extension info
        let info: ExtensionInfo = {
            let output = plugin
                .call::<&[u8], Vec<u8>>("morphir_extension_info", &[])
                .map_err(|e| {
                    DaemonError::Extension(format!("Failed to get extension info: {}", e))
                })?;
            serde_json::from_slice(&output)?
        };

        debug!("Loaded extension: {} v{}", info.name, info.version);

        Ok(Self {
            id: id.to_string(),
            plugin: Arc::new(RwLock::new(plugin)),
            info,
            request_id: std::sync::atomic::AtomicU64::new(1),
        })
    }

    /// Get extension info
    pub fn info(&self) -> &ExtensionInfo {
        &self.info
    }

    /// Get extension ID
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Check if extension supports a capability
    pub fn supports(&self, ext_type: ExtensionType) -> bool {
        self.info.types.contains(&ext_type)
    }

    /// Call an extension method with JSON-RPC
    pub async fn call<I: Serialize, O: DeserializeOwned>(
        &self,
        method: &str,
        params: I,
    ) -> Result<O> {
        let id = self
            .request_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let request = ExtensionRequest::new(method, params, id)?;
        let request_bytes = serde_json::to_vec(&request)?;

        debug!("Calling extension method: {} (id={})", method, id);

        // Call the plugin
        let mut plugin = self.plugin.write().await;
        let output = plugin
            .call::<&[u8], Vec<u8>>("handle", &request_bytes)
            .map_err(|e| DaemonError::Extension(format!("Plugin call failed: {}", e)))?;

        let response: ExtensionResponse = serde_json::from_slice(&output)?;
        response.into_result()
    }

    /// Call a raw function on the plugin (no JSON-RPC wrapping)
    pub async fn call_raw(&self, func_name: &str, input: &[u8]) -> Result<Vec<u8>> {
        let mut plugin = self.plugin.write().await;
        plugin
            .call::<&[u8], Vec<u8>>(func_name, input)
            .map_err(|e| DaemonError::Extension(format!("Plugin call failed: {}", e)))
    }
}

/// Builder for ExtensionContainer with configuration options
pub struct ExtensionContainerBuilder {
    id: String,
    wasm_path: Option<std::path::PathBuf>,
    wasm_bytes: Option<Vec<u8>>,
    host_funcs: Option<MorphirHostFunctions>,
    config: HashMap<String, String>,
}

impl ExtensionContainerBuilder {
    /// Create a new builder
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            wasm_path: None,
            wasm_bytes: None,
            host_funcs: None,
            config: HashMap::new(),
        }
    }

    /// Set WASM file path
    pub fn with_path(mut self, path: impl AsRef<Path>) -> Self {
        self.wasm_path = Some(path.as_ref().to_path_buf());
        self
    }

    /// Set WASM bytes
    pub fn with_bytes(mut self, bytes: Vec<u8>) -> Self {
        self.wasm_bytes = Some(bytes);
        self
    }

    /// Set host functions
    pub fn with_host_functions(mut self, funcs: MorphirHostFunctions) -> Self {
        self.host_funcs = Some(funcs);
        self
    }

    /// Add configuration value
    pub fn with_config(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.config.insert(key.into(), value.into());
        self
    }

    /// Build the extension container
    pub fn build(self) -> Result<ExtensionContainer> {
        let host_funcs = self.host_funcs.unwrap_or_default();

        if let Some(path) = self.wasm_path {
            ExtensionContainer::new(&self.id, &path, host_funcs)
        } else if let Some(bytes) = self.wasm_bytes {
            ExtensionContainer::from_bytes(&self.id, &bytes, host_funcs)
        } else {
            Err(DaemonError::Extension(
                "No WASM path or bytes provided".into(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extension_type_serde() {
        let json = serde_json::to_string(&ExtensionType::Frontend).unwrap();
        assert_eq!(json, "\"frontend\"");

        let parsed: ExtensionType = serde_json::from_str("\"backend\"").unwrap();
        assert_eq!(parsed, ExtensionType::Backend);
    }
}
