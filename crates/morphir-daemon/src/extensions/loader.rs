//! Extension loader using Extism
//!
//! This module handles loading WASM plugins from various sources.

use crate::error::{DaemonError, Result};
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Extension loader that manages plugin loading from various sources
pub struct ExtensionLoader {
    /// Cache directory for downloaded plugins
    cache_dir: PathBuf,
    /// Temporary directory for plugin operations
    temp_dir: PathBuf,
}

impl ExtensionLoader {
    /// Create a new extension loader
    ///
    /// # Arguments
    /// * `cache_dir` - Directory for caching downloaded plugins
    pub fn new(cache_dir: PathBuf) -> Result<Self> {
        let temp_dir = cache_dir.join("temp");
        std::fs::create_dir_all(&cache_dir)?;
        std::fs::create_dir_all(&temp_dir)?;

        info!("Extension loader initialized with cache at {:?}", cache_dir);

        Ok(Self { cache_dir, temp_dir })
    }

    /// Create a loader with default cache directory
    pub fn with_default_cache() -> Result<Self> {
        let cache_dir = dirs::cache_dir()
            .ok_or_else(|| DaemonError::Extension("Could not determine cache directory".into()))?
            .join("morphir")
            .join("extensions");

        Self::new(cache_dir)
    }

    /// Load extension from a local file path
    pub async fn load_from_path(&self, path: &Path) -> Result<PathBuf> {
        if !path.exists() {
            return Err(DaemonError::Extension(format!(
                "Extension file not found: {:?}",
                path
            )));
        }

        // Verify it's a WASM file
        if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
            return Err(DaemonError::Extension(format!(
                "Expected .wasm file, got: {:?}",
                path
            )));
        }

        debug!("Loading extension from path: {:?}", path);
        Ok(path.to_path_buf())
    }

    /// Load extension from a URL
    pub async fn load_from_url(&self, id: &str, url: &str) -> Result<PathBuf> {
        // Calculate cache path
        let cache_path = self.cache_dir.join(format!("{}.wasm", id));

        // Check if already cached
        if cache_path.exists() {
            debug!("Using cached extension: {:?}", cache_path);
            return Ok(cache_path);
        }

        info!("Downloading extension from: {}", url);

        // Download the file
        let response = reqwest::get(url).await.map_err(|e| {
            DaemonError::Extension(format!("Failed to download extension: {}", e))
        })?;

        if !response.status().is_success() {
            return Err(DaemonError::Extension(format!(
                "Failed to download extension: HTTP {}",
                response.status()
            )));
        }

        let bytes = response.bytes().await.map_err(|e| {
            DaemonError::Extension(format!("Failed to read extension bytes: {}", e))
        })?;

        // Write to temp file first, then move to cache
        let temp_path = self.temp_dir.join(format!("{}.wasm.tmp", id));
        tokio::fs::write(&temp_path, &bytes).await?;
        tokio::fs::rename(&temp_path, &cache_path).await?;

        info!("Extension cached at: {:?}", cache_path);
        Ok(cache_path)
    }

    /// Load extension from a GitHub release
    pub async fn load_from_github(
        &self,
        id: &str,
        repo: &str,
        tag: Option<&str>,
        asset_name: &str,
    ) -> Result<PathBuf> {
        let tag = tag.unwrap_or("latest");
        let url = if tag == "latest" {
            format!(
                "https://github.com/{}/releases/latest/download/{}",
                repo, asset_name
            )
        } else {
            format!(
                "https://github.com/{}/releases/download/{}/{}",
                repo, tag, asset_name
            )
        };

        self.load_from_url(id, &url).await
    }

    /// Get the cache directory path
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Clear the extension cache
    pub async fn clear_cache(&self) -> Result<()> {
        if self.cache_dir.exists() {
            tokio::fs::remove_dir_all(&self.cache_dir).await?;
            tokio::fs::create_dir_all(&self.cache_dir).await?;
        }
        Ok(())
    }

    /// List cached extensions
    pub fn list_cached(&self) -> Result<Vec<PathBuf>> {
        let mut extensions = Vec::new();

        if self.cache_dir.exists() {
            for entry in std::fs::read_dir(&self.cache_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("wasm") {
                    extensions.push(path);
                }
            }
        }

        Ok(extensions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_loader_creation() {
        let temp = tempdir().unwrap();
        let loader = ExtensionLoader::new(temp.path().to_path_buf()).unwrap();
        assert!(loader.cache_dir().exists());
    }
}
