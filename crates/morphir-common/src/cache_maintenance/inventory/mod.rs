mod identity;
mod registration;

pub use registration::{CacheNamespace, CacheRegistrationError};

use self::{identity::portable_identity, registration::portable_comparison_key};
use super::{CacheEntry, CacheModelError};
use crate::home::MorphirHome;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
#[cfg(windows)]
use cap_std::fs::MetadataExt;
use cap_std::fs::{Dir, DirEntry, Metadata, OpenOptions};
use same_file::Handle;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

const DEFAULT_MAX_INVENTORY_ENTRIES: usize = 100_000;
const DEFAULT_MAX_INVENTORY_DEPTH: usize = 64;

/// Hard bounds for a single namespace inventory walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheInventoryLimits {
    max_entries: usize,
    max_depth: usize,
}

impl CacheInventoryLimits {
    /// Construct nonzero entry-count and depth limits.
    pub fn new(max_entries: usize, max_depth: usize) -> Result<Self, CacheInventoryError> {
        if max_entries == 0 || max_depth == 0 {
            return Err(CacheInventoryError::InvalidLimits);
        }
        Ok(Self {
            max_entries,
            max_depth,
        })
    }
}

impl Default for CacheInventoryLimits {
    fn default() -> Self {
        Self {
            max_entries: DEFAULT_MAX_INVENTORY_ENTRIES,
            max_depth: DEFAULT_MAX_INVENTORY_DEPTH,
        }
    }
}

/// A fail-closed inventory error. No cleanup plan should execute after this.
#[derive(Debug, Error)]
pub enum CacheInventoryError {
    /// Inventory limits must both be nonzero.
    #[error("cache inventory limits must be nonzero")]
    InvalidLimits,
    /// The shared cache root must be an ordinary directory, not a link or junction.
    #[error("refusing to inspect link-like cache root {path}")]
    LinkLikeCacheRoot {
        /// Shared cache root that failed the safety check.
        path: PathBuf,
    },
    /// The shared cache root exists but is not a directory.
    #[error("cache root is not a directory: {path}")]
    InvalidCacheRoot {
        /// Shared cache root that failed the type check.
        path: PathBuf,
    },
    /// A namespace root must be an ordinary directory, not a link or junction.
    #[error("refusing to inspect link-like namespace root {path}")]
    LinkLikeNamespaceRoot {
        /// Namespace root that failed the safety check.
        path: PathBuf,
    },
    /// A namespace root exists but is not a directory.
    #[error("cache namespace root is not a directory: {path}")]
    InvalidNamespaceRoot {
        /// Namespace root that failed the type check.
        path: PathBuf,
    },
    /// The entry-count budget was exhausted.
    #[error("cache inventory entry limit of {limit} was exceeded")]
    EntryLimitExceeded {
        /// Configured entry-count limit.
        limit: usize,
    },
    /// The directory-depth budget was exhausted.
    #[error("cache inventory depth limit of {limit} was exceeded")]
    DepthLimitExceeded {
        /// Configured directory-depth limit.
        limit: usize,
    },
    /// Observed byte totals exceeded the supported range.
    #[error("cache inventory byte total exceeds the supported range")]
    ByteCountOverflow,
    /// An observed filesystem name could not be represented as a protected identity.
    #[error(transparent)]
    InvalidObservedIdentity(#[from] CacheModelError),
    /// Filesystem inspection failed.
    #[error("failed to inspect cache path {path}: {source}")]
    Io {
        /// Path being inspected.
        path: PathBuf,
        /// Underlying filesystem failure.
        #[source]
        source: io::Error,
    },
}

/// Inventory one registered namespace without following links or junctions.
///
/// The walk is bounded and returns unknown filesystem objects as protected,
/// unclassified entries. Callers should stop cleanup if inventory returns an
/// error.
///
/// ```no_run
/// use morphir_common::cache_maintenance::{
///     CacheInventoryLimits, CacheNamespace, inventory_cache_namespace,
/// };
/// use morphir_common::home::MorphirHome;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let home = MorphirHome::resolve()?;
/// let downloads = CacheNamespace::new("downloads")?
///     .with_disposable("desktop/1.2.3", 1_735_689_600)?;
/// let entries = inventory_cache_namespace(
///     &home,
///     &downloads,
///     CacheInventoryLimits::new(10_000, 32)?,
/// )?;
/// for entry in entries {
///     println!("{}: {} bytes", entry.path(), entry.bytes());
/// }
/// # Ok(())
/// # }
/// ```
pub fn inventory_cache_namespace(
    home: &MorphirHome,
    namespace: &CacheNamespace,
    limits: CacheInventoryLimits,
) -> Result<Vec<CacheEntry>, CacheInventoryError> {
    let home_root = home.root();
    let home_dir = match Dir::open_ambient_dir(home_root, ambient_authority()) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(io_error(home_root, source)),
    };

    let cache_root = home.cache_dir();
    let cache_metadata = match home_dir.symlink_metadata("cache") {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(io_error(&cache_root, source)),
    };
    if is_link_like(&cache_metadata) {
        return Err(CacheInventoryError::LinkLikeCacheRoot { path: cache_root });
    }
    if !cache_metadata.is_dir() {
        return Err(CacheInventoryError::InvalidCacheRoot { path: cache_root });
    }
    let cache_dir = home_dir
        .open_dir_nofollow("cache")
        .map_err(|source| io_error(&cache_root, source))?;

    let root = cache_root.join(namespace.name());
    let metadata = match cache_dir.symlink_metadata(namespace.name()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(io_error(&root, source)),
    };
    if is_link_like(&metadata) {
        return Err(CacheInventoryError::LinkLikeNamespaceRoot { path: root });
    }
    if !metadata.is_dir() {
        return Err(CacheInventoryError::InvalidNamespaceRoot { path: root });
    }
    let root_dir = cache_dir
        .open_dir_nofollow(namespace.name())
        .map_err(|source| io_error(&root, source))?;

    let mut inventory = InventoryWalk {
        namespace,
        limits,
        visited: 0,
        entries: Vec::new(),
    };
    inventory.scan_children(&root_dir, &root, Path::new(""), 0)?;
    inventory
        .entries
        .sort_by(|left, right| left.path().cmp(right.path()));
    Ok(inventory.entries)
}

struct InventoryWalk<'a> {
    namespace: &'a CacheNamespace,
    limits: CacheInventoryLimits,
    visited: usize,
    entries: Vec<CacheEntry>,
}

impl InventoryWalk<'_> {
    fn scan_children(
        &mut self,
        directory: &Dir,
        display_path: &Path,
        relative: &Path,
        depth: usize,
    ) -> Result<(), CacheInventoryError> {
        self.check_depth(depth)?;
        for child in self.read_children(directory, display_path)? {
            let child_path = display_path.join(child.file_name());
            let child_relative = relative.join(child.file_name());
            let identity = portable_identity(&child_relative);
            let comparison_key = portable_comparison_key(&identity);
            let metadata = directory
                .symlink_metadata(child.file_name())
                .map_err(|source| io_error(&child_path, source))?;

            if let Some(template) = self
                .registered_entry(directory, &child, &metadata, &identity, &comparison_key)
                .cloned()
            {
                let measured =
                    self.measure(directory, &child, &child_path, &metadata, depth + 1)?;
                let entry = if measured.safe {
                    template.with_observation(identity, measured.bytes)?
                } else {
                    CacheEntry::unclassified(self.namespace.name.clone(), identity, measured.bytes)?
                };
                self.entries.push(entry);
            } else if self.has_registered_descendant(directory, &child, &metadata, &identity) {
                if is_link_like(&metadata) || !metadata.is_dir() {
                    let measured =
                        self.measure(directory, &child, &child_path, &metadata, depth + 1)?;
                    self.entries.push(CacheEntry::unclassified(
                        self.namespace.name.clone(),
                        identity,
                        measured.bytes,
                    )?);
                } else {
                    match directory.open_dir_nofollow(child.file_name()) {
                        Ok(child_dir) => {
                            self.scan_children(&child_dir, &child_path, &child_relative, depth + 1)?
                        }
                        Err(_) => self.entries.push(CacheEntry::unclassified(
                            self.namespace.name.clone(),
                            identity,
                            metadata.len(),
                        )?),
                    }
                }
            } else {
                let measured =
                    self.measure(directory, &child, &child_path, &metadata, depth + 1)?;
                self.entries.push(CacheEntry::unclassified(
                    self.namespace.name.clone(),
                    identity,
                    measured.bytes,
                )?);
            }
        }
        Ok(())
    }

    fn measure(
        &mut self,
        parent: &Dir,
        entry: &DirEntry,
        path: &Path,
        metadata: &Metadata,
        depth: usize,
    ) -> Result<MeasuredEntry, CacheInventoryError> {
        if is_link_like(metadata) {
            return Ok(MeasuredEntry {
                bytes: metadata.len(),
                safe: false,
            });
        }
        if metadata.is_file() {
            return Ok(MeasuredEntry {
                bytes: metadata.len(),
                safe: true,
            });
        }
        if !metadata.is_dir() {
            return Ok(MeasuredEntry {
                bytes: metadata.len(),
                safe: false,
            });
        }

        self.check_depth(depth)?;
        let directory = match parent.open_dir_nofollow(entry.file_name()) {
            Ok(directory) => directory,
            Err(_) => {
                return Ok(MeasuredEntry {
                    bytes: metadata.len(),
                    safe: false,
                });
            }
        };
        let mut bytes = 0_u64;
        let mut safe = true;
        for child in self.read_children(&directory, path)? {
            let child_path = path.join(child.file_name());
            let child_metadata = directory
                .symlink_metadata(child.file_name())
                .map_err(|source| io_error(&child_path, source))?;
            let measured =
                self.measure(&directory, &child, &child_path, &child_metadata, depth + 1)?;
            bytes = bytes
                .checked_add(measured.bytes)
                .ok_or(CacheInventoryError::ByteCountOverflow)?;
            safe &= measured.safe;
        }
        Ok(MeasuredEntry { bytes, safe })
    }

    fn registered_entry(
        &self,
        directory: &Dir,
        child: &DirEntry,
        metadata: &Metadata,
        identity: &str,
        comparison_key: &str,
    ) -> Option<&CacheEntry> {
        let entry = self.namespace.entries.get(comparison_key)?;
        self.native_spelling_matches(directory, child, metadata, identity, entry.path())
            .then_some(entry)
    }

    fn has_registered_descendant(
        &self,
        directory: &Dir,
        child: &DirEntry,
        metadata: &Metadata,
        identity: &str,
    ) -> bool {
        let prefix = format!("{}/", portable_comparison_key(identity));
        self.namespace.entries.iter().any(|(path, entry)| {
            path.starts_with(&prefix)
                && self.native_spelling_matches(directory, child, metadata, identity, entry.path())
        })
    }

    fn native_spelling_matches(
        &self,
        directory: &Dir,
        child: &DirEntry,
        metadata: &Metadata,
        observed_identity: &str,
        registered_path: &str,
    ) -> bool {
        let depth = observed_identity.split('/').count();
        let registered_prefix = registered_path
            .split('/')
            .take(depth)
            .collect::<Vec<_>>()
            .join("/");
        if registered_prefix == observed_identity {
            return true;
        }
        let Some(registered_component) = registered_prefix.rsplit('/').next() else {
            return false;
        };
        same_object(
            directory,
            OsStr::new(registered_component),
            child.file_name().as_os_str(),
            metadata,
        )
    }

    fn visit(&mut self) -> Result<(), CacheInventoryError> {
        self.visited =
            self.visited
                .checked_add(1)
                .ok_or(CacheInventoryError::EntryLimitExceeded {
                    limit: self.limits.max_entries,
                })?;
        if self.visited > self.limits.max_entries {
            return Err(CacheInventoryError::EntryLimitExceeded {
                limit: self.limits.max_entries,
            });
        }
        Ok(())
    }

    fn read_children(
        &mut self,
        directory: &Dir,
        display_path: &Path,
    ) -> Result<Vec<DirEntry>, CacheInventoryError> {
        let reader = directory
            .entries()
            .map_err(|source| io_error(display_path, source))?;
        let mut entries = Vec::new();
        for entry in reader {
            self.visit()?;
            entries.push(entry.map_err(|source| io_error(display_path, source))?);
        }
        entries.sort_by_key(DirEntry::file_name);
        Ok(entries)
    }

    fn check_depth(&self, depth: usize) -> Result<(), CacheInventoryError> {
        if depth > self.limits.max_depth {
            return Err(CacheInventoryError::DepthLimitExceeded {
                limit: self.limits.max_depth,
            });
        }
        Ok(())
    }
}

struct MeasuredEntry {
    bytes: u64,
    safe: bool,
}

fn same_object(
    directory: &Dir,
    registered_name: &OsStr,
    observed_name: &OsStr,
    observed_metadata: &Metadata,
) -> bool {
    if is_link_like(observed_metadata) {
        return false;
    }
    let Ok(registered_metadata) = directory.symlink_metadata(registered_name) else {
        return false;
    };
    if is_link_like(&registered_metadata)
        || registered_metadata.is_dir() != observed_metadata.is_dir()
        || registered_metadata.is_file() != observed_metadata.is_file()
    {
        return false;
    }

    let registered = object_handle(directory, registered_name, &registered_metadata);
    let observed = object_handle(directory, observed_name, observed_metadata);
    matches!((registered, observed), (Ok(left), Ok(right)) if left == right)
}

fn object_handle(directory: &Dir, name: &OsStr, metadata: &Metadata) -> io::Result<Handle> {
    let file = if metadata.is_dir() {
        directory.open_dir_nofollow(name)?.into_std_file()
    } else {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        directory.open_with(name, &options)?.into_std()
    };
    Handle::from_file(file)
}

fn io_error(path: &Path, source: io::Error) -> CacheInventoryError {
    CacheInventoryError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(windows)]
fn is_link_like(metadata: &Metadata) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_link_like(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}
