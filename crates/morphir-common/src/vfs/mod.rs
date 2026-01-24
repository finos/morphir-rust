pub mod notebook;

pub use notebook::NotebookVfs;

use std::io::Result;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Virtual File System trait
///
/// Abstraction over file system operations to support:
/// - OS file system
/// - In-memory file system (for testing/WASM)
/// - Zip archives (for distribution)
/// - Remote file systems (S3, etc.)
/// - Jupyter notebooks (for testing document trees)
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

    /// Remove a file or directory
    fn remove(&self, path: &Path) -> Result<()>;

    /// Copy a file from one location to another
    fn copy(&self, from: &Path, to: &Path) -> Result<()>;

    /// Get file metadata
    fn metadata(&self, path: &Path) -> Result<FileMetadata>;
}

/// File metadata information
#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub size: u64,
    pub is_file: bool,
    pub is_dir: bool,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
}

// Re-export implementations
pub use memory::MemoryVfs;
pub use os::OsVfs;

mod memory;
mod os;
