use std::fs;
use std::io::Result;
use std::path::{Path, PathBuf};

/// Virtual File System trait
///
/// Abstraction over file system operations to support:
/// - OS file system
/// - In-memory file system (for testing/WASM)
/// - Zip archives (for distribution)
/// - Remote file systems (S3, etc.)
pub trait Vfs {
    /// Read a file to a string
    fn read_to_string(&self, path: &Path) -> Result<String>;

    /// Write a string to a file
    fn write_from_string(&self, path: &Path, content: &str) -> Result<()>;

    /// Check if a path exists
    fn exists(&self, path: &Path) -> bool;

    /// Check if a path is a directory
    fn is_dir(&self, path: &Path) -> bool;

    /// List directory contents
    fn list_dir(&self, path: &Path) -> Result<Vec<PathBuf>>;

    /// Create a directory and its parents
    fn create_dir_all(&self, path: &Path) -> Result<()>;

    /// Resolve a glob pattern to a list of paths
    fn glob(&self, pattern: &str) -> Result<Vec<PathBuf>>;
}

/// OS File System implementation
pub struct OsVfs;

impl Vfs for OsVfs {
    fn read_to_string(&self, path: &Path) -> Result<String> {
        fs::read_to_string(path)
    }

    fn write_from_string(&self, path: &Path, content: &str) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn list_dir(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            entries.push(entry.path());
        }
        Ok(entries)
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        fs::create_dir_all(path)
    }

    fn glob(&self, pattern: &str) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for entry in
            glob::glob(pattern).map_err(std::io::Error::other)?
        {
            paths.push(entry.map_err(std::io::Error::other)?);
        }
        Ok(paths)
    }
}

/// In-Memory File System implementation (for testing)
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default, Debug)]
pub struct MemoryVfs {
    files: Arc<Mutex<HashMap<PathBuf, String>>>,
}

impl MemoryVfs {
    pub fn new() -> Self {
        Self::default()
    }

    fn normalize_path(path: &Path) -> PathBuf {
        let mut normalized = PathBuf::new();
        for component in path.components() {
            if let std::path::Component::CurDir = component {
                continue;
            }
            normalized.push(component);
        }
        if normalized.as_os_str().is_empty() {
            return PathBuf::from(".");
        }
        normalized
    }
}

impl Vfs for MemoryVfs {
    fn read_to_string(&self, path: &Path) -> Result<String> {
        let path = MemoryVfs::normalize_path(path);
        let files = self.files.lock().unwrap();
        files.get(&path).cloned().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("File not found: {:?}", path),
            )
        })
    }

    fn write_from_string(&self, path: &Path, content: &str) -> Result<()> {
        let path = MemoryVfs::normalize_path(path);
        let mut files = self.files.lock().unwrap();
        files.insert(path, content.to_string());
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        let path = MemoryVfs::normalize_path(path);
        let files = self.files.lock().unwrap();
        files.contains_key(&path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        let path = MemoryVfs::normalize_path(path);
        let files = self.files.lock().unwrap();
        // Check if any file starts with this path (and is longer, indicating a child)
        // Or if path is "." or root.
        if path == Path::new(".") || path == Path::new("/") {
            return !files.is_empty();
        }

        for k in files.keys() {
            if k.starts_with(&path) && k != &path {
                return true;
            }
        }
        false
    }

    fn list_dir(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let path = MemoryVfs::normalize_path(path);
        let files = self.files.lock().unwrap();
        let mut entries = Vec::new();
        for k in files.keys() {
            if k.starts_with(&path) && k != &path {
                entries.push(k.clone());
            }
        }
        Ok(entries)
    }

    fn create_dir_all(&self, _path: &Path) -> Result<()> {
        Ok(())
    }

    fn glob(&self, pattern: &str) -> Result<Vec<PathBuf>> {
        let files = self.files.lock().unwrap();
        let glob_pattern = glob::Pattern::new(pattern)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        let mut matches = Vec::new();
        for path in files.keys() {
            if glob_pattern.matches_path(path) {
                matches.push(path.clone());
            }
        }
        Ok(matches)
    }
}
