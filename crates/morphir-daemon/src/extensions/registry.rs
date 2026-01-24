//! Extension registry for managing loaded extensions
//!
//! This module provides discovery and lifecycle management for extensions.

use crate::error::{DaemonError, Result};
use crate::extensions::container::{ExtensionContainer, ExtensionInfo, ExtensionType};
use crate::extensions::host_functions::MorphirHostFunctions;
use crate::extensions::loader::ExtensionLoader;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Configuration for an extension in the registry
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExtensionConfig {
    /// Extension identifier
    pub id: String,
    /// Source path, URL, or embedded
    #[serde(default)]
    pub source: ExtensionSource,
    /// Whether the extension is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Extension-specific configuration
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
}

fn default_true() -> bool {
    true
}

/// Source of an extension
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ExtensionSource {
    /// Local file path
    Path { path: PathBuf },
    /// URL to download
    Url { url: String },
    /// GitHub release
    GitHub {
        repo: String,
        #[serde(default)]
        tag: Option<String>,
        asset: String,
    },
}

impl Default for ExtensionSource {
    fn default() -> Self {
        ExtensionSource::Path {
            path: PathBuf::new(),
        }
    }
}

/// Extension registry managing loaded extensions
pub struct ExtensionRegistry {
    /// Extension loader
    loader: ExtensionLoader,
    /// Loaded extensions by ID
    extensions: RwLock<HashMap<String, Arc<ExtensionContainer>>>,
    /// Extension configurations
    configs: RwLock<HashMap<String, ExtensionConfig>>,
    /// Workspace root for host functions
    workspace_root: PathBuf,
    /// Output directory for host functions
    output_dir: PathBuf,
}

impl ExtensionRegistry {
    /// Create a new extension registry
    pub fn new(workspace_root: PathBuf, output_dir: PathBuf) -> Result<Self> {
        let cache_dir = dirs::cache_dir()
            .ok_or_else(|| DaemonError::Extension("Could not determine cache directory".into()))?
            .join("morphir")
            .join("extensions");

        Ok(Self {
            loader: ExtensionLoader::new(cache_dir)?,
            extensions: RwLock::new(HashMap::new()),
            configs: RwLock::new(HashMap::new()),
            workspace_root,
            output_dir,
        })
    }

    /// Create registry with custom loader
    pub fn with_loader(
        loader: ExtensionLoader,
        workspace_root: PathBuf,
        output_dir: PathBuf,
    ) -> Self {
        Self {
            loader,
            extensions: RwLock::new(HashMap::new()),
            configs: RwLock::new(HashMap::new()),
            workspace_root,
            output_dir,
        }
    }

    /// Register an extension configuration
    pub async fn register(&self, config: ExtensionConfig) -> Result<()> {
        let mut configs = self.configs.write().await;
        info!("Registering extension: {}", config.id);
        configs.insert(config.id.clone(), config);
        Ok(())
    }

    /// Load an extension by ID
    pub async fn load(&self, id: &str) -> Result<Arc<ExtensionContainer>> {
        // Check if already loaded
        {
            let extensions = self.extensions.read().await;
            if let Some(ext) = extensions.get(id) {
                return Ok(ext.clone());
            }
        }

        // Get config
        let config = {
            let configs = self.configs.read().await;
            configs.get(id).cloned().ok_or_else(|| {
                DaemonError::Extension(format!("Extension not registered: {}", id))
            })?
        };

        if !config.enabled {
            return Err(DaemonError::Extension(format!(
                "Extension is disabled: {}",
                id
            )));
        }

        // Load the WASM file
        let wasm_path = match &config.source {
            ExtensionSource::Path { path } => self.loader.load_from_path(path).await?,
            ExtensionSource::Url { url } => self.loader.load_from_url(id, url).await?,
            ExtensionSource::GitHub { repo, tag, asset } => {
                self.loader
                    .load_from_github(id, repo, tag.as_deref(), asset)
                    .await?
            }
        };

        // Create host functions
        let host_funcs = MorphirHostFunctions::for_workspace(
            self.workspace_root.clone(),
            self.output_dir.clone(),
        );

        // Create container
        let container = ExtensionContainer::new(id, &wasm_path, host_funcs)?;
        let container = Arc::new(container);

        // Store in registry
        {
            let mut extensions = self.extensions.write().await;
            extensions.insert(id.to_string(), container.clone());
        }

        info!(
            "Extension loaded: {} v{}",
            container.info().name,
            container.info().version
        );

        Ok(container)
    }

    /// Load extension from a path (convenience method)
    pub async fn load_from_path(&self, id: &str, path: &Path) -> Result<Arc<ExtensionContainer>> {
        self.register(ExtensionConfig {
            id: id.to_string(),
            source: ExtensionSource::Path {
                path: path.to_path_buf(),
            },
            enabled: true,
            config: HashMap::new(),
        })
        .await?;

        self.load(id).await
    }

    /// Unload an extension
    pub async fn unload(&self, id: &str) -> Result<()> {
        let mut extensions = self.extensions.write().await;
        if extensions.remove(id).is_some() {
            info!("Extension unloaded: {}", id);
            Ok(())
        } else {
            Err(DaemonError::Extension(format!(
                "Extension not loaded: {}",
                id
            )))
        }
    }

    /// Get a loaded extension
    pub async fn get(&self, id: &str) -> Option<Arc<ExtensionContainer>> {
        let extensions = self.extensions.read().await;
        extensions.get(id).cloned()
    }

    /// List all loaded extensions
    pub async fn list(&self) -> Vec<ExtensionInfo> {
        let extensions = self.extensions.read().await;
        extensions.values().map(|e| e.info().clone()).collect()
    }

    /// List extensions by type
    pub async fn list_by_type(&self, ext_type: ExtensionType) -> Vec<Arc<ExtensionContainer>> {
        let extensions = self.extensions.read().await;
        extensions
            .values()
            .filter(|e| e.supports(ext_type))
            .cloned()
            .collect()
    }

    /// Find an extension supporting a specific type
    pub async fn find_by_type(&self, ext_type: ExtensionType) -> Option<Arc<ExtensionContainer>> {
        let extensions = self.extensions.read().await;
        extensions.values().find(|e| e.supports(ext_type)).cloned()
    }

    /// Get the extension loader
    pub fn loader(&self) -> &ExtensionLoader {
        &self.loader
    }

    /// Discover extensions from configuration
    pub async fn discover_from_config(
        &self,
        extensions_config: &HashMap<String, ExtensionConfig>,
    ) -> Result<()> {
        for (id, config) in extensions_config {
            let mut config = config.clone();
            config.id = id.clone();
            self.register(config).await?;
        }
        Ok(())
    }

    /// Load all registered extensions
    pub async fn load_all(&self) -> Vec<Result<Arc<ExtensionContainer>>> {
        let ids: Vec<String> = {
            let configs = self.configs.read().await;
            configs.keys().cloned().collect()
        };

        let mut results = Vec::new();
        for id in ids {
            results.push(self.load(&id).await);
        }
        results
    }

    /// Register a builtin extension
    pub async fn register_builtin(&self, id: &str, path: PathBuf) -> Result<()> {
        self.register(ExtensionConfig {
            id: id.to_string(),
            source: ExtensionSource::Path { path },
            enabled: true,
            config: HashMap::new(),
        })
        .await
    }

    /// Find a frontend extension by language name
    pub async fn find_extension_by_language(
        &self,
        language: &str,
    ) -> Option<Arc<ExtensionContainer>> {
        // First check builtin extensions (by ID matching language)
        if let Ok(ext) = self.load(language).await {
            if ext.supports(ExtensionType::Frontend) {
                return Some(ext);
            }
        }

        // Then check registered extensions
        let extensions = self.extensions.read().await;
        for ext in extensions.values() {
            if ext.supports(ExtensionType::Frontend) {
                // For now, match by ID. In the future, we could query the extension
                // for its supported languages
                if ext.id() == language {
                    return Some(ext.clone());
                }
            }
        }
        None
    }

    /// Find a backend extension by target language name
    pub async fn find_extension_by_target(&self, target: &str) -> Option<Arc<ExtensionContainer>> {
        // First check builtin extensions (by ID matching target)
        if let Ok(ext) = self.load(target).await {
            if ext.supports(ExtensionType::Backend) {
                return Some(ext);
            }
        }

        // Then check registered extensions
        let extensions = self.extensions.read().await;
        for ext in extensions.values() {
            if ext.supports(ExtensionType::Backend) {
                // For now, match by ID. In the future, we could query the extension
                // for its supported targets
                if ext.id() == target {
                    return Some(ext.clone());
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_registry_creation() {
        let temp = tempdir().unwrap();
        let registry =
            ExtensionRegistry::new(temp.path().to_path_buf(), temp.path().join("output")).unwrap();

        let list = registry.list().await;
        assert!(list.is_empty());
    }
}
