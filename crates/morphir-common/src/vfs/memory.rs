use super::{FileMetadata, Vfs};
use std::collections::HashMap;
use std::io::Result;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// In-Memory File System implementation (for testing)
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

        // Check if it's a file
        if files.contains_key(&path) {
            return true;
        }

        // Check if it's a directory (any file has this path as a prefix)
        // This is consistent with OsVfs/std::path::Path::exists() semantics
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

    fn remove(&self, path: &Path) -> Result<()> {
        let path = MemoryVfs::normalize_path(path);
        let mut files = self.files.lock().unwrap();

        // Remove the file itself
        files.remove(&path);

        // Remove all files that are children of this path (if it's a directory)
        let keys_to_remove: Vec<PathBuf> = files
            .keys()
            .filter(|k| k.starts_with(&path) && *k != &path)
            .cloned()
            .collect();

        for key in keys_to_remove {
            files.remove(&key);
        }

        Ok(())
    }

    fn copy(&self, from: &Path, to: &Path) -> Result<()> {
        let from = MemoryVfs::normalize_path(from);
        let to = MemoryVfs::normalize_path(to);

        // Clone the content while holding the lock briefly
        let content = {
            let files = self.files.lock().unwrap();
            files.get(&from).cloned().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("File not found: {:?}", from),
                )
            })?
        };

        // Now insert with a new lock
        let mut files = self.files.lock().unwrap();
        files.insert(to, content);
        Ok(())
    }

    fn metadata(&self, path: &Path) -> Result<FileMetadata> {
        let path = MemoryVfs::normalize_path(path);
        let files = self.files.lock().unwrap();

        if let Some(content) = files.get(&path) {
            Ok(FileMetadata {
                size: content.len() as u64,
                is_file: true,
                is_dir: false,
                modified: None,
                created: None,
            })
        } else {
            // Check if it's a directory
            let is_dir = self.is_dir(&path);
            Ok(FileMetadata {
                size: 0,
                is_file: false,
                is_dir,
                modified: None,
                created: None,
            })
        }
    }
}
