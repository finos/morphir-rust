//! Source caching layer for remote sources.

use crate::remote::config::CacheConfig;
use crate::remote::error::{RemoteSourceError, Result};
use crate::remote::source::RemoteSource;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Cache entry metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// The source this entry is for.
    pub source: RemoteSource,

    /// When the entry was cached.
    pub cached_at: u64,

    /// SHA256 hash of the content.
    pub content_hash: String,

    /// HTTP ETag if available.
    pub etag: Option<String>,

    /// Size in bytes.
    pub size: u64,

    /// Path to the cached content relative to cache root.
    pub path: String,
}

/// Cache index storing metadata for all cached entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheIndex {
    /// Map from cache key to entry metadata.
    pub entries: HashMap<String, CacheEntry>,
}

/// Source cache for storing downloaded remote sources.
pub struct SourceCache {
    /// Root directory for the cache.
    root: PathBuf,

    /// Cache configuration.
    config: CacheConfig,

    /// In-memory index (loaded on demand).
    index: Option<CacheIndex>,
}

impl SourceCache {
    /// Create a new source cache with the given configuration.
    pub fn new(config: CacheConfig) -> Result<Self> {
        let root = config.directory.clone().unwrap_or_else(|| {
            dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from(".cache"))
                .join("morphir")
                .join("sources")
        });

        fs::create_dir_all(&root).map_err(|_| RemoteSourceError::CacheDirectoryError {
            path: root.clone(),
        })?;

        Ok(Self {
            root,
            config,
            index: None,
        })
    }

    /// Create a cache with default configuration.
    pub fn with_defaults() -> Result<Self> {
        Self::new(CacheConfig::default())
    }

    /// Get the cache root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get the path where a source would be cached.
    pub fn cache_path(&self, source: &RemoteSource) -> PathBuf {
        let key = source.cache_key();
        let subdir = match source {
            RemoteSource::Local { .. } => "local",
            RemoteSource::Http { .. } => "http",
            RemoteSource::Git { .. } => "git",
            RemoteSource::GitHub { .. } => "github",
            RemoteSource::Gist { .. } => "gist",
        };

        self.root.join(subdir).join(&key[..2]).join(&key)
    }

    /// Check if a cached entry exists and is valid.
    pub fn is_valid(&self, source: &RemoteSource) -> bool {
        let path = self.cache_path(source);
        if !path.exists() {
            return false;
        }

        // Check TTL if configured
        if self.config.ttl_secs > 0 {
            if let Ok(metadata) = fs::metadata(&path) {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(elapsed) = SystemTime::now().duration_since(modified) {
                        if elapsed > Duration::from_secs(self.config.ttl_secs) {
                            return false;
                        }
                    }
                }
            }
        }

        true
    }

    /// Get a cached source if it exists and is valid.
    pub fn get(&self, source: &RemoteSource) -> Option<PathBuf> {
        if self.is_valid(source) {
            let path = self.cache_path(source);
            if path.exists() {
                return Some(path);
            }
        }
        None
    }

    /// Store content in the cache.
    pub fn put(&mut self, source: &RemoteSource, content_path: &Path) -> Result<PathBuf> {
        let cache_path = self.cache_path(source);

        // Create parent directories
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Copy content to cache
        if content_path.is_dir() {
            copy_dir_all(content_path, &cache_path)?;
        } else {
            fs::copy(content_path, &cache_path)?;
        }

        // Update index
        self.update_index(source, &cache_path)?;

        Ok(cache_path)
    }

    /// Store raw bytes in the cache.
    pub fn put_bytes(&mut self, source: &RemoteSource, content: &[u8]) -> Result<PathBuf> {
        let cache_path = self.cache_path(source);

        // Create parent directories
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write content
        fs::write(&cache_path, content)?;

        // Update index
        self.update_index(source, &cache_path)?;

        Ok(cache_path)
    }

    /// Remove a cached entry.
    pub fn remove(&mut self, source: &RemoteSource) -> Result<()> {
        let cache_path = self.cache_path(source);

        if cache_path.exists() {
            if cache_path.is_dir() {
                fs::remove_dir_all(&cache_path)?;
            } else {
                fs::remove_file(&cache_path)?;
            }
        }

        // Update index
        self.remove_from_index(source)?;

        Ok(())
    }

    /// Clear all cached entries.
    pub fn clear(&mut self) -> Result<()> {
        if self.root.exists() {
            fs::remove_dir_all(&self.root)?;
            fs::create_dir_all(&self.root)?;
        }
        self.index = Some(CacheIndex::default());
        self.save_index()?;
        Ok(())
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        let mut stats = CacheStats::default();

        if let Ok(index) = self.load_index() {
            stats.entry_count = index.entries.len();
            for entry in index.entries.values() {
                stats.total_size += entry.size;
            }
        }

        stats
    }

    /// Load the cache index from disk.
    fn load_index(&self) -> Result<CacheIndex> {
        let index_path = self.root.join("index.json");

        if index_path.exists() {
            let content = fs::read_to_string(&index_path)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            Ok(CacheIndex::default())
        }
    }

    /// Save the cache index to disk.
    fn save_index(&self) -> Result<()> {
        if let Some(ref index) = self.index {
            let index_path = self.root.join("index.json");
            let content = serde_json::to_string_pretty(index)?;
            fs::write(index_path, content)?;
        }
        Ok(())
    }

    /// Update the index with a new entry.
    fn update_index(&mut self, source: &RemoteSource, cache_path: &Path) -> Result<()> {
        // Load index if not already loaded
        if self.index.is_none() {
            self.index = Some(self.load_index().unwrap_or_default());
        }
        let index = self.index.as_mut().unwrap();

        let size = if cache_path.is_dir() {
            dir_size(cache_path).unwrap_or(0)
        } else {
            fs::metadata(cache_path).map(|m| m.len()).unwrap_or(0)
        };

        let entry = CacheEntry {
            source: source.clone(),
            cached_at: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            content_hash: String::new(), // TODO: compute hash
            etag: None,
            size,
            path: cache_path
                .strip_prefix(&self.root)
                .unwrap_or(cache_path)
                .to_string_lossy()
                .to_string(),
        };

        index.entries.insert(source.cache_key(), entry);
        self.save_index()
    }

    /// Remove an entry from the index.
    fn remove_from_index(&mut self, source: &RemoteSource) -> Result<()> {
        if self.index.is_none() {
            self.index = Some(self.load_index()?);
        }

        if let Some(ref mut index) = self.index {
            index.entries.remove(&source.cache_key());
            self.save_index()?;
        }

        Ok(())
    }
}

/// Cache statistics.
#[derive(Debug, Default)]
pub struct CacheStats {
    /// Number of cached entries.
    pub entry_count: usize,

    /// Total size of cached content in bytes.
    pub total_size: u64,
}

/// Recursively copy a directory.
fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if ty.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

/// Calculate the total size of a directory.
fn dir_size(path: &Path) -> Result<u64> {
    let mut size = 0;

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;

        if metadata.is_dir() {
            size += dir_size(&entry.path())?;
        } else {
            size += metadata.len();
        }
    }

    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_cache() -> (SourceCache, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config = CacheConfig {
            directory: Some(temp_dir.path().to_path_buf()),
            ..Default::default()
        };
        let cache = SourceCache::new(config).unwrap();
        (cache, temp_dir)
    }

    #[test]
    fn test_cache_path() {
        let (cache, _temp) = test_cache();
        let source = RemoteSource::parse("https://example.com/file.json").unwrap();
        let path = cache.cache_path(&source);

        assert!(path.starts_with(cache.root()));
        assert!(path.to_string_lossy().contains("http"));
    }

    #[test]
    fn test_cache_put_get() {
        let (mut cache, temp) = test_cache();
        let source = RemoteSource::parse("https://example.com/file.json").unwrap();

        // Create a test file
        let test_file = temp.path().join("test.json");
        fs::write(&test_file, r#"{"test": true}"#).unwrap();

        // Cache it
        let cached_path = cache.put(&source, &test_file).unwrap();
        assert!(cached_path.exists());

        // Retrieve it
        let retrieved = cache.get(&source);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap(), cached_path);
    }

    #[test]
    fn test_cache_bytes() {
        let (mut cache, _temp) = test_cache();
        let source = RemoteSource::parse("https://example.com/data.json").unwrap();

        let content = b"Hello, World!";
        let cached_path = cache.put_bytes(&source, content).unwrap();

        assert!(cached_path.exists());
        assert_eq!(fs::read(&cached_path).unwrap(), content);
    }

    #[test]
    fn test_cache_remove() {
        let (mut cache, temp) = test_cache();
        let source = RemoteSource::parse("https://example.com/file.json").unwrap();

        // Create and cache a test file
        let test_file = temp.path().join("test.json");
        fs::write(&test_file, r#"{"test": true}"#).unwrap();
        let cached_path = cache.put(&source, &test_file).unwrap();

        // Remove it
        cache.remove(&source).unwrap();
        assert!(!cached_path.exists());
        assert!(cache.get(&source).is_none());
    }
}
