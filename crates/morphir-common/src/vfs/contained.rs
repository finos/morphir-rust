//! A physical VFS that never traverses a link.
//!
//! The `vfs` crate's own [`PhysicalFS`] resolves through symlinks and junctions: `metadata` follows
//! them, `read_dir` lists them as ordinary entries, and `VfsPath::remove_dir_all` — which is plain
//! recursion over `read_dir` and `metadata` — therefore deletes the *target* of a link rather than
//! the link. A writer that empties a directory before writing into it, as the document-tree sink
//! does, can then destroy files outside the root it was given.
//!
//! [`ContainedPhysicalFS`] closes that. Its rule is one sentence: **a link is never followed, and a
//! link that is in the way is removed as a link.** Concretely —
//!
//! - `read_dir` omits every linked child, and answers nothing at all for a path that is itself a
//!   link, so no walk and no recursive delete can reach through one.
//! - `metadata` reports a link as a zero-length `File`, because `VfsFileType` has no third case and
//!   a link is not a directory anything here may descend.
//! - `remove_dir` removes a link as a link; on a real directory it first removes the linked children
//!   `read_dir` hid, so `remove_dir_all` can still finish.
//!
//! Everything else delegates to [`PhysicalFS`]. The guarantee is about *traversal*: reading and
//! removing stay inside the root. Writing through a path a caller names explicitly is still the
//! caller's business.

use std::path::{Path, PathBuf};

use ::vfs::{
    FileSystem, PhysicalFS, SeekAndRead, SeekAndWrite, VfsFileType, VfsMetadata, VfsResult,
};

/// A physical filesystem, rooted at an OS directory, that never follows a link.
#[derive(Debug)]
pub struct ContainedPhysicalFS {
    root: PathBuf,
    inner: PhysicalFS,
}

impl ContainedPhysicalFS {
    /// Create a contained physical filesystem rooted in `root`.
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        Self {
            inner: PhysicalFS::new(&root),
            root,
        }
    }

    /// The OS path a VFS path names, by the same rule [`PhysicalFS`] uses: paths are absolute and
    /// start with `/`, except the root, which is the empty string.
    fn os_path(&self, path: &str) -> PathBuf {
        self.root.join(path.strip_prefix('/').unwrap_or(path))
    }

    /// Whether a VFS path names a link rather than the thing it points at.
    ///
    /// `symlink_metadata` does not follow, and Rust reports a Windows junction as a symlink, so one
    /// question covers symlinks, directory symlinks and junctions alike. A path that cannot be
    /// inspected at all — it is gone, or unreadable — is not a link to skip; whatever the caller
    /// does next will report the real error.
    fn is_link(&self, path: &str) -> bool {
        std::fs::symlink_metadata(self.os_path(path))
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
    }

    /// The linked children of a directory, which `read_dir` hides.
    fn linked_children(&self, path: &str) -> VfsResult<Vec<String>> {
        Ok(self
            .inner
            .read_dir(path)?
            .filter(|name| self.is_link(&child_path(path, name)))
            .collect())
    }

    /// Removes a link, whichever of the two ways the platform spells it: a symlink to a file, and
    /// every symlink on Unix, goes through `remove_file`; a directory symlink or junction on
    /// Windows goes through `remove_dir`. Neither follows the link.
    fn remove_link(&self, path: &str) -> VfsResult<()> {
        let os_path = self.os_path(path);
        match std::fs::remove_file(&os_path) {
            Ok(()) => Ok(()),
            Err(_) => Ok(std::fs::remove_dir(&os_path)?),
        }
    }
}

fn child_path(path: &str, name: &str) -> String {
    format!("{path}/{name}")
}

impl FileSystem for ContainedPhysicalFS {
    fn read_dir(&self, path: &str) -> VfsResult<Box<dyn Iterator<Item = String> + Send>> {
        if self.is_link(path) {
            return Ok(Box::new(std::iter::empty()));
        }
        let kept: Vec<String> = self
            .inner
            .read_dir(path)?
            .filter(|name| !self.is_link(&child_path(path, name)))
            .collect();
        Ok(Box::new(kept.into_iter()))
    }

    fn create_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.create_dir(path)
    }

    fn open_file(&self, path: &str) -> VfsResult<Box<dyn SeekAndRead + Send>> {
        self.inner.open_file(path)
    }

    fn create_file(&self, path: &str) -> VfsResult<Box<dyn SeekAndWrite + Send>> {
        self.inner.create_file(path)
    }

    fn append_file(&self, path: &str) -> VfsResult<Box<dyn SeekAndWrite + Send>> {
        self.inner.append_file(path)
    }

    fn metadata(&self, path: &str) -> VfsResult<VfsMetadata> {
        if self.is_link(path) {
            return Ok(VfsMetadata {
                file_type: VfsFileType::File,
                len: 0,
                created: None,
                modified: None,
                accessed: None,
            });
        }
        self.inner.metadata(path)
    }

    fn exists(&self, path: &str) -> VfsResult<bool> {
        if self.is_link(path) {
            return Ok(true);
        }
        self.inner.exists(path)
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        if self.is_link(path) {
            return self.remove_link(path);
        }
        for name in self.linked_children(path)? {
            self.remove_link(&child_path(path, name.as_str()))?;
        }
        self.inner.remove_dir(path)
    }

    fn copy_file(&self, src: &str, dest: &str) -> VfsResult<()> {
        self.inner.copy_file(src, dest)
    }

    fn move_file(&self, src: &str, dest: &str) -> VfsResult<()> {
        self.inner.move_file(src, dest)
    }

    fn move_dir(&self, src: &str, dest: &str) -> VfsResult<()> {
        self.inner.move_dir(src, dest)
    }
}
