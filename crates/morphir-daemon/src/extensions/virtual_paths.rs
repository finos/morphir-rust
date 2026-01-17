//! Virtual path mappings for sandboxed filesystem access
//!
//! Extensions access files through virtual paths that map to real paths.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Virtual path configuration for extensions
#[derive(Debug, Clone)]
pub struct VirtualPathConfig {
    /// Mapping from virtual path prefix to real path
    mappings: HashMap<String, PathBuf>,
}

impl Default for VirtualPathConfig {
    fn default() -> Self {
        Self {
            mappings: HashMap::new(),
        }
    }
}

impl VirtualPathConfig {
    /// Create a new virtual path configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Create configuration for a Morphir workspace
    pub fn for_workspace(workspace_root: &Path, output_dir: &Path) -> Self {
        let mut config = Self::new();

        // /workspace -> actual workspace root
        config.add_mapping("/workspace", workspace_root);

        // /output -> build output directory
        config.add_mapping("/output", output_dir);

        // /cache -> extension cache directory
        if let Some(cache_dir) = dirs::cache_dir() {
            config.add_mapping("/cache", cache_dir.join("morphir"));
        }

        config
    }

    /// Add a virtual path mapping
    pub fn add_mapping(&mut self, virtual_prefix: &str, real_path: impl AsRef<Path>) {
        let prefix = virtual_prefix.trim_end_matches('/').to_string();
        self.mappings.insert(prefix, real_path.as_ref().to_path_buf());
    }

    /// Remove a virtual path mapping
    pub fn remove_mapping(&mut self, virtual_prefix: &str) {
        let prefix = virtual_prefix.trim_end_matches('/');
        self.mappings.remove(prefix);
    }

    /// Resolve a virtual path to a real path
    ///
    /// Returns None if the path doesn't match any mapping.
    pub fn resolve(&self, virtual_path: &str) -> Option<PathBuf> {
        for (prefix, real_base) in &self.mappings {
            if virtual_path == prefix {
                return Some(real_base.clone());
            }

            if virtual_path.starts_with(prefix) {
                let suffix = &virtual_path[prefix.len()..];
                if suffix.starts_with('/') {
                    let suffix = suffix.trim_start_matches('/');
                    return Some(real_base.join(suffix));
                }
            }
        }

        None
    }

    /// Convert a real path to a virtual path
    ///
    /// Returns the virtual path if the real path is under a mapped prefix.
    pub fn virtualize(&self, real_path: &Path) -> Option<String> {
        for (prefix, real_base) in &self.mappings {
            if let Ok(suffix) = real_path.strip_prefix(real_base) {
                if suffix.as_os_str().is_empty() {
                    return Some(prefix.clone());
                }
                return Some(format!("{}/{}", prefix, suffix.display()));
            }
        }

        None
    }

    /// Check if a virtual path is valid (matches a mapping)
    pub fn is_valid(&self, virtual_path: &str) -> bool {
        self.resolve(virtual_path).is_some()
    }

    /// Get all virtual path prefixes
    pub fn prefixes(&self) -> Vec<&str> {
        self.mappings.keys().map(|s| s.as_str()).collect()
    }

    /// Get the mapping for a prefix
    pub fn get_mapping(&self, prefix: &str) -> Option<&Path> {
        let prefix = prefix.trim_end_matches('/');
        self.mappings.get(prefix).map(|p| p.as_path())
    }
}

/// Sandbox for controlling file access
#[derive(Debug, Clone)]
pub struct FileSandbox {
    /// Virtual path configuration
    config: VirtualPathConfig,
    /// Whether to allow reads outside mappings
    allow_external_reads: bool,
    /// Whether to allow writes outside mappings
    allow_external_writes: bool,
}

impl FileSandbox {
    /// Create a new sandbox with the given configuration
    pub fn new(config: VirtualPathConfig) -> Self {
        Self {
            config,
            allow_external_reads: false,
            allow_external_writes: false,
        }
    }

    /// Create a permissive sandbox that allows external access
    pub fn permissive(config: VirtualPathConfig) -> Self {
        Self {
            config,
            allow_external_reads: true,
            allow_external_writes: true,
        }
    }

    /// Check if reading from a path is allowed
    pub fn can_read(&self, path: &str) -> bool {
        self.config.is_valid(path) || self.allow_external_reads
    }

    /// Check if writing to a path is allowed
    pub fn can_write(&self, path: &str) -> bool {
        self.config.is_valid(path) || self.allow_external_writes
    }

    /// Resolve and validate a path for reading
    pub fn resolve_read(&self, virtual_path: &str) -> Result<PathBuf, SandboxError> {
        if !self.can_read(virtual_path) {
            return Err(SandboxError::AccessDenied(virtual_path.to_string()));
        }

        self.config
            .resolve(virtual_path)
            .ok_or_else(|| SandboxError::InvalidPath(virtual_path.to_string()))
    }

    /// Resolve and validate a path for writing
    pub fn resolve_write(&self, virtual_path: &str) -> Result<PathBuf, SandboxError> {
        if !self.can_write(virtual_path) {
            return Err(SandboxError::AccessDenied(virtual_path.to_string()));
        }

        self.config
            .resolve(virtual_path)
            .ok_or_else(|| SandboxError::InvalidPath(virtual_path.to_string()))
    }

    /// Get the virtual path configuration
    pub fn config(&self) -> &VirtualPathConfig {
        &self.config
    }
}

/// Errors from sandbox operations
#[derive(Debug, Clone, thiserror::Error)]
pub enum SandboxError {
    /// Access to path is denied
    #[error("Access denied: {0}")]
    AccessDenied(String),

    /// Path is not valid
    #[error("Invalid path: {0}")]
    InvalidPath(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_virtual_path_resolution() {
        let temp = tempdir().unwrap();
        let mut config = VirtualPathConfig::new();
        config.add_mapping("/workspace", temp.path());

        assert_eq!(
            config.resolve("/workspace"),
            Some(temp.path().to_path_buf())
        );

        assert_eq!(
            config.resolve("/workspace/src/main.rs"),
            Some(temp.path().join("src/main.rs"))
        );

        assert_eq!(config.resolve("/unknown"), None);
    }

    #[test]
    fn test_virtualize() {
        let temp = tempdir().unwrap();
        let mut config = VirtualPathConfig::new();
        config.add_mapping("/workspace", temp.path());

        assert_eq!(
            config.virtualize(temp.path()),
            Some("/workspace".to_string())
        );

        assert_eq!(
            config.virtualize(&temp.path().join("src/lib.rs")),
            Some("/workspace/src/lib.rs".to_string())
        );
    }

    #[test]
    fn test_sandbox() {
        let temp = tempdir().unwrap();
        let mut config = VirtualPathConfig::new();
        config.add_mapping("/workspace", temp.path());

        let sandbox = FileSandbox::new(config);

        assert!(sandbox.can_read("/workspace/file.txt"));
        assert!(!sandbox.can_read("/etc/passwd"));

        assert!(sandbox.resolve_read("/workspace/file.txt").is_ok());
        assert!(sandbox.resolve_read("/etc/passwd").is_err());
    }
}
