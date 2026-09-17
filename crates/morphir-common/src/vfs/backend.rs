use std::path::Path;

use ::vfs::{MemoryFS, VfsPath};

use super::contained::ContainedPhysicalFS;

/// Guarantees available when publishing a completed migration result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublicationCapabilities {
    pub atomic_file_replace: bool,
    pub atomic_dir_replace: bool,
}

/// Reports publication guarantees independently from generic VFS moves.
pub trait Publisher {
    fn capabilities(&self) -> PublicationCapabilities;
}

/// Publisher used when the target is backed by a concrete OS path.
#[derive(Clone, Copy, Debug, Default)]
pub struct PhysicalPublisher;

impl Publisher for PhysicalPublisher {
    fn capabilities(&self) -> PublicationCapabilities {
        PublicationCapabilities {
            atomic_file_replace: true,
            atomic_dir_replace: true,
        }
    }
}

/// Publisher for backends that can only expose a completed tree manifest last.
#[derive(Clone, Copy, Debug, Default)]
pub struct ManifestLastPublisher;

impl Publisher for ManifestLastPublisher {
    fn capabilities(&self) -> PublicationCapabilities {
        PublicationCapabilities {
            atomic_file_replace: false,
            atomic_dir_replace: false,
        }
    }
}

/// Create an isolated in-memory VFS root.
pub fn memory_root() -> VfsPath {
    VfsPath::new(MemoryFS::new())
}

/// Create a physical VFS rooted below the supplied OS directory.
///
/// The root is a [`ContainedPhysicalFS`], so walking it and emptying it stay inside the OS
/// directory: a symlink or junction under the root is never followed, and one that is in the way of
/// a recursive removal is removed as a link. See that type for the exact rule.
pub fn physical_root(path: impl AsRef<Path>) -> VfsPath {
    VfsPath::new(ContainedPhysicalFS::new(path))
}
