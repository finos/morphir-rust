//! WASM Component runtime for loading and executing extensions

use std::collections::HashMap;
use std::path::Path;

use crate::error::{ExtensionError, Result};
use crate::types::{
    ExtensionCapabilities, ExtensionInfo, ExtensionSource, ExtensionType, LoadedExtension,
    ResourceLimits,
};

/// The extension runtime manages loading and execution of WASM extensions
pub struct ExtensionRuntime {
    /// Loaded extensions
    extensions: HashMap<String, LoadedExtension>,
    /// Default resource limits
    default_limits: ResourceLimits,
    /// WASM engine (shared across all extensions)
    engine: wasmtime::Engine,
}

impl ExtensionRuntime {
    /// Create a new extension runtime
    pub fn new() -> Result<Self> {
        let mut config = wasmtime::Config::new();
        config.wasm_component_model(true);
        config.async_support(true);

        let engine = wasmtime::Engine::new(&config)
            .map_err(|e| ExtensionError::WasmRuntime(e.to_string()))?;

        Ok(Self {
            extensions: HashMap::new(),
            default_limits: ResourceLimits::default(),
            engine,
        })
    }

    /// Create a runtime with custom resource limits
    pub fn with_limits(limits: ResourceLimits) -> Result<Self> {
        let mut runtime = Self::new()?;
        runtime.default_limits = limits;
        Ok(runtime)
    }

    /// Load an extension from a file path
    pub async fn load_from_path(&mut self, path: &Path) -> Result<&LoadedExtension> {
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| ExtensionError::LoadFailed(format!("Failed to read {}: {}", path.display(), e)))?;

        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        self.load_from_bytes(&id, &bytes, ExtensionSource::Path(path.to_path_buf()))
            .await
    }

    /// Load an extension from bytes
    pub async fn load_from_bytes(
        &mut self,
        id: &str,
        bytes: &[u8],
        source: ExtensionSource,
    ) -> Result<&LoadedExtension> {
        // Validate WASM component
        let _component = wasmtime::component::Component::new(&self.engine, bytes)
            .map_err(|e| ExtensionError::LoadFailed(format!("Invalid WASM component: {}", e)))?;

        // TODO: Instantiate the component and query its capabilities
        // For now, create placeholder info
        let info = ExtensionInfo {
            id: id.to_string(),
            name: id.to_string(),
            version: "0.0.0".to_string(),
            description: None,
            types: vec![],  // Will be populated from component
            author: None,
            homepage: None,
            license: None,
        };

        let capabilities = ExtensionCapabilities::default();

        let loaded = LoadedExtension {
            info,
            source,
            capabilities,
            active: true,
        };

        self.extensions.insert(id.to_string(), loaded);
        Ok(self.extensions.get(id).unwrap())
    }

    /// Unload an extension
    pub fn unload(&mut self, id: &str) -> Result<()> {
        self.extensions
            .remove(id)
            .ok_or_else(|| ExtensionError::NotFound(id.to_string()))?;
        Ok(())
    }

    /// Get a loaded extension by ID
    pub fn get(&self, id: &str) -> Option<&LoadedExtension> {
        self.extensions.get(id)
    }

    /// List all loaded extensions
    pub fn list(&self) -> impl Iterator<Item = &LoadedExtension> {
        self.extensions.values()
    }

    /// List extensions that support a specific capability
    pub fn list_by_type(&self, ext_type: ExtensionType) -> impl Iterator<Item = &LoadedExtension> {
        self.extensions
            .values()
            .filter(move |ext| ext.info.types.contains(&ext_type))
    }

    /// Check if an extension supports a capability
    pub fn supports_type(&self, id: &str, ext_type: ExtensionType) -> bool {
        self.extensions
            .get(id)
            .map(|ext| ext.info.types.contains(&ext_type))
            .unwrap_or(false)
    }

    /// Get the WASM engine
    pub fn engine(&self) -> &wasmtime::Engine {
        &self.engine
    }
}

impl Default for ExtensionRuntime {
    fn default() -> Self {
        Self::new().expect("Failed to create default extension runtime")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_runtime_creation() {
        let runtime = ExtensionRuntime::new();
        assert!(runtime.is_ok());
    }
}
