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
///
/// # Contract
///
/// Implementations must maintain consistent behavior across all methods:
///
/// - **`exists(path)`**: Returns `true` if the path refers to either a file OR a directory.
///   This matches standard filesystem semantics (e.g., `std::path::Path::exists()`).
///
/// - **`is_dir(path)`**: Returns `true` only if the path is a directory. Implies `exists()`.
///
/// - **`read_to_string(path)`**: Only succeeds for files, not directories.
///
/// - **`write_from_string(path, content)`**: Creates parent directories as needed.
///
/// - **`list_dir(path)`**: Returns contents of a directory. Fails if path is not a directory.
///
/// - **`glob(pattern)`**: Returns all matching paths (both files and directories).
pub trait Vfs {
    /// Read a file to a string.
    ///
    /// Returns an error if the path does not exist or is a directory.
    fn read_to_string(&self, path: &Path) -> Result<String>;

    /// Write a string to a file.
    ///
    /// Creates parent directories as needed. Overwrites existing files.
    fn write_from_string(&self, path: &Path, content: &str) -> Result<()>;

    /// Check if a path exists (file OR directory).
    ///
    /// Returns `true` if the path refers to an existing file or directory.
    /// This is consistent with `std::path::Path::exists()` semantics.
    fn exists(&self, path: &Path) -> bool;

    /// Check if a path is a directory.
    ///
    /// Returns `true` only if the path exists and is a directory.
    fn is_dir(&self, path: &Path) -> bool;

    /// List directory contents.
    ///
    /// Returns paths of all entries in the directory.
    /// Returns an error if the path does not exist or is not a directory.
    fn list_dir(&self, path: &Path) -> Result<Vec<PathBuf>>;

    /// Create a directory and all parent directories.
    ///
    /// Does nothing if the directory already exists.
    fn create_dir_all(&self, path: &Path) -> Result<()>;

    /// Resolve a glob pattern to a list of matching paths.
    ///
    /// Returns both files and directories that match the pattern.
    fn glob(&self, pattern: &str) -> Result<Vec<PathBuf>>;

    /// Remove a file or directory.
    ///
    /// If the path is a directory, removes it and all its contents recursively.
    fn remove(&self, path: &Path) -> Result<()>;

    /// Copy a file from one location to another.
    ///
    /// Returns an error if the source does not exist or is a directory.
    fn copy(&self, from: &Path, to: &Path) -> Result<()>;

    /// Get file metadata.
    ///
    /// Returns metadata for the path, including whether it's a file or directory.
    fn metadata(&self, path: &Path) -> Result<FileMetadata>;

    /// Return whether a path is a symbolic link without following it.
    ///
    /// Virtual filesystems without symbolic links may use the default implementation.
    fn is_symlink(&self, _path: &Path) -> Result<bool> {
        Ok(false)
    }

    /// Return a stable identity path for traversal and containment checks.
    ///
    /// Virtual filesystems use lexical normalization by default. Filesystems that can
    /// contain links should override this with their canonical path operation.
    fn canonicalize(&self, path: &Path) -> Result<PathBuf> {
        Ok(normalize_lexically(path))
    }
}

fn normalize_lexically(path: &Path) -> PathBuf {
    use std::path::Component;

    path.components()
        .fold(PathBuf::new(), |mut normalized, component| {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    if !normalized.pop() && !path.has_root() {
                        normalized.push(component.as_os_str());
                    }
                }
                Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                    normalized.push(component.as_os_str());
                }
            }
            normalized
        })
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

mod backend;
pub use ::vfs::{FileSystem, MemoryFS, PhysicalFS, VfsError, VfsPath, VfsResult};
pub use backend::{
    ManifestLastPublisher, PhysicalPublisher, PublicationCapabilities, Publisher, memory_root,
    physical_root,
};

mod memory;
mod os;
