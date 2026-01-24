use super::{FileMetadata, Vfs};
use std::fs;
use std::io::Result;
use std::path::Path;

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

    fn list_dir(&self, path: &Path) -> Result<Vec<std::path::PathBuf>> {
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

    fn glob(&self, pattern: &str) -> Result<Vec<std::path::PathBuf>> {
        let mut paths = Vec::new();
        for entry in glob::glob(pattern).map_err(std::io::Error::other)? {
            paths.push(entry.map_err(std::io::Error::other)?);
        }
        Ok(paths)
    }

    fn remove(&self, path: &Path) -> Result<()> {
        if path.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        }
    }

    fn copy(&self, from: &Path, to: &Path) -> Result<()> {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(from, to)?;
        Ok(())
    }

    fn metadata(&self, path: &Path) -> Result<FileMetadata> {
        let meta = fs::metadata(path)?;
        Ok(FileMetadata {
            size: meta.len(),
            is_file: meta.is_file(),
            is_dir: meta.is_dir(),
            modified: meta.modified().ok(),
            created: meta.created().ok(),
        })
    }
}
