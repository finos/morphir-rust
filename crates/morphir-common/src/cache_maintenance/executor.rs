use super::{
    CacheEntryState, CacheInventoryError, CacheInventoryLimits, CacheNamespace, CleanupPlan,
    inventory_cache_namespace,
};
use crate::home::MorphirHome;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
#[cfg(windows)]
use cap_std::fs::MetadataExt as CapMetadataExt;
use cap_std::fs::{Dir, Metadata as CapMetadata, OpenOptions as CapOpenOptions};
use fs2::FileExt;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Hard removal-count and byte budgets for one cleanup execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheExecutionLimits {
    max_removals: usize,
    max_bytes: u64,
}

impl CacheExecutionLimits {
    /// Construct nonzero per-run removal and byte budgets.
    pub fn new(max_removals: usize, max_bytes: u64) -> Result<Self, CacheExecutionError> {
        if max_removals == 0 || max_bytes == 0 {
            return Err(CacheExecutionError::InvalidLimits);
        }
        Ok(Self {
            max_removals,
            max_bytes,
        })
    }
}

/// Result of revalidating and executing one planner-selected entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheExecutionDisposition {
    /// The exact revalidated entry was removed.
    Removed,
    /// The entry disappeared after inventory and required no action.
    Missing,
    /// The entry's observed bytes changed after the plan was created.
    Stale,
    /// A lease acquired after planning now protects the entry.
    ActiveLease,
    /// The path became link-like, special, or otherwise unclassified.
    Unclassified,
    /// The current ownership snapshot does not register the namespace.
    Unregistered,
    /// This and subsequent selected entries were left for a later bounded run.
    DeferredLimit,
}

/// Stable execution result for one planner-selected entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheExecutionItem {
    namespace: String,
    path: String,
    planned_bytes: u64,
    observed_bytes: Option<u64>,
    disposition: CacheExecutionDisposition,
}

impl CacheExecutionItem {
    fn new(
        namespace: impl Into<String>,
        path: impl Into<String>,
        planned_bytes: u64,
        observed_bytes: Option<u64>,
        disposition: CacheExecutionDisposition,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            path: path.into(),
            planned_bytes,
            observed_bytes,
            disposition,
        }
    }

    /// Namespace owning the selected entry.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Portable path relative to the namespace root.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Bytes recorded by the plan.
    pub fn planned_bytes(&self) -> u64 {
        self.planned_bytes
    }

    /// Bytes observed during execution, when the entry still existed.
    pub fn observed_bytes(&self) -> Option<u64> {
        self.observed_bytes
    }

    /// Outcome of executing this selected entry.
    pub fn disposition(&self) -> CacheExecutionDisposition {
        self.disposition
    }
}

/// Deterministic, serializable result of a bounded cleanup execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheExecutionReport {
    removed_bytes: u64,
    items: Vec<CacheExecutionItem>,
}

impl CacheExecutionReport {
    /// Bytes actually removed from active cache namespaces.
    pub fn removed_bytes(&self) -> u64 {
        self.removed_bytes
    }

    /// Results in the planner's stable namespace-and-path order.
    pub fn items(&self) -> &[CacheExecutionItem] {
        &self.items
    }
}

/// A fail-closed cleanup execution error.
#[derive(Debug, Error)]
pub enum CacheExecutionError {
    /// Execution budgets must both be nonzero.
    #[error("cache execution limits must be nonzero")]
    InvalidLimits,
    /// Each ownership namespace may be supplied only once.
    #[error("duplicate cache ownership namespace {namespace}")]
    DuplicateNamespace {
        /// Repeated namespace identifier.
        namespace: String,
    },
    /// Filesystem inventory failed while revalidating a selected entry.
    #[error(transparent)]
    Inventory(#[from] CacheInventoryError),
    /// A maintenance lock or trash path was replaced with an unsafe object.
    #[error("refusing to use unsafe maintenance path {path}")]
    UnsafeMaintenancePath {
        /// Path that failed the safety check.
        path: PathBuf,
    },
    /// Execution byte accounting overflowed.
    #[error("cache execution byte total exceeds the supported range")]
    ByteCountOverflow,
    /// A filesystem operation failed.
    #[error("cache cleanup failed at {path}: {source}")]
    Io {
        /// Path being operated on.
        path: PathBuf,
        /// Underlying filesystem failure.
        #[source]
        source: io::Error,
    },
}

impl CacheExecutionError {
    fn code(&self) -> &'static str {
        match self {
            Self::InvalidLimits => "invalid-limits",
            Self::DuplicateNamespace { .. } => "duplicate-namespace",
            Self::Inventory(_) => "inventory-failed",
            Self::UnsafeMaintenancePath { .. } => "unsafe-maintenance-path",
            Self::ByteCountOverflow => "byte-count-overflow",
            Self::Io { .. } => "io-failed",
        }
    }
}

/// Execute only removal decisions produced by the in-memory planner.
///
/// The executor takes the shared maintenance lock, re-inventories each selected
/// entry against the current ownership and lease registrations, refuses stale
/// or unsafe paths, and moves removals beneath Morphir Home's maintenance trash
/// before deleting them. The per-run limits make the same operation suitable
/// for manual and opportunistic automatic cleanup.
///
/// # Example
///
/// ```no_run
/// use morphir_common::cache_maintenance::{
///     CacheExecutionLimits, CacheInventoryLimits, CacheNamespace, CachePolicy, CleanupMode,
///     execute_cache_cleanup, inventory_cache_namespace, plan_cache_cleanup,
/// };
/// use morphir_common::home::MorphirHome;
/// use std::time::Duration;
///
/// let home = MorphirHome::resolve()?;
/// let downloads = CacheNamespace::new("downloads")?
///     .with_disposable("desktop.tar.gz", 1_000)?;
/// let inventory = inventory_cache_namespace(
///     &home,
///     &downloads,
///     CacheInventoryLimits::default(),
/// )?;
/// let plan = plan_cache_cleanup(
///     inventory,
///     CachePolicy::new(Duration::from_secs(30 * 24 * 60 * 60), 1_000_000_000),
///     2_000,
///     CleanupMode::Policy,
/// )?;
/// let report = execute_cache_cleanup(
///     &home,
///     &plan,
///     &[downloads],
///     CacheInventoryLimits::default(),
///     CacheExecutionLimits::new(100, 100_000_000)?,
/// )?;
/// println!("removed {} bytes", report.removed_bytes());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn execute_cache_cleanup(
    home: &MorphirHome,
    plan: &CleanupPlan,
    ownership: &[CacheNamespace],
    inventory_limits: CacheInventoryLimits,
    limits: CacheExecutionLimits,
) -> Result<CacheExecutionReport, CacheExecutionError> {
    let selected_entries = plan
        .decisions()
        .iter()
        .filter(|decision| decision.will_remove())
        .count();
    info!(
        event = "cache_cleanup_started",
        selected_entries,
        max_removals = limits.max_removals,
        max_bytes = limits.max_bytes,
        "cache cleanup started"
    );
    let result = execute_cache_cleanup_inner(home, plan, ownership, inventory_limits, limits);
    match &result {
        Ok(report) => {
            for (entry_index, item) in report.items().iter().enumerate() {
                debug!(
                    event = "cache_cleanup_entry_finished",
                    entry_index,
                    namespace = item.namespace(),
                    planned_bytes = item.planned_bytes(),
                    observed_bytes = item.observed_bytes(),
                    disposition = ?item.disposition(),
                    "cache cleanup entry finished"
                );
            }
            info!(
                event = "cache_cleanup_finished",
                selected_entries,
                result_entries = report.items().len(),
                removed_bytes = report.removed_bytes(),
                "cache cleanup finished"
            );
        }
        Err(error) => warn!(
            event = "cache_cleanup_failed",
            selected_entries,
            error_code = error.code(),
            "cache cleanup failed"
        ),
    }
    result
}

fn execute_cache_cleanup_inner(
    home: &MorphirHome,
    plan: &CleanupPlan,
    ownership: &[CacheNamespace],
    inventory_limits: CacheInventoryLimits,
    limits: CacheExecutionLimits,
) -> Result<CacheExecutionReport, CacheExecutionError> {
    let _guard = MaintenanceGuard::acquire(home)?;
    let trash = open_maintenance_trash(home)?;
    sweep_existing_trash(&trash)?;
    let inventories = inventory_namespaces(home, ownership, inventory_limits)?;
    let selected = plan
        .decisions()
        .iter()
        .filter(|decision| decision.will_remove());
    let mut items = Vec::new();
    let mut attempted = 0_usize;
    let mut budgeted_bytes = 0_u64;
    let mut removed_bytes = 0_u64;
    let mut deferred = false;
    let mut trash_run: Option<TrashRun> = None;

    for decision in selected {
        let entry = decision.entry();
        let next_budgeted_bytes = budgeted_bytes
            .checked_add(entry.bytes())
            .ok_or(CacheExecutionError::ByteCountOverflow)?;
        if deferred || attempted == limits.max_removals || next_budgeted_bytes > limits.max_bytes {
            deferred = true;
            items.push(execution_item(
                entry,
                None,
                CacheExecutionDisposition::DeferredLimit,
            ));
            continue;
        }
        attempted += 1;
        budgeted_bytes = next_budgeted_bytes;

        match revalidate_entry(&inventories, entry.namespace(), entry.path(), entry.bytes()) {
            RevalidatedEntry::Missing => items.push(execution_item(
                entry,
                None,
                CacheExecutionDisposition::Missing,
            )),
            RevalidatedEntry::ActiveLease { observed_bytes } => items.push(execution_item(
                entry,
                Some(observed_bytes),
                CacheExecutionDisposition::ActiveLease,
            )),
            RevalidatedEntry::Unclassified { observed_bytes } => items.push(execution_item(
                entry,
                Some(observed_bytes),
                CacheExecutionDisposition::Unclassified,
            )),
            RevalidatedEntry::Unregistered => items.push(execution_item(
                entry,
                None,
                CacheExecutionDisposition::Unregistered,
            )),
            RevalidatedEntry::Stale { observed_bytes } => items.push(execution_item(
                entry,
                Some(observed_bytes),
                CacheExecutionDisposition::Stale,
            )),
            RevalidatedEntry::Ready => {
                if trash_run.is_none() {
                    trash_run = Some(create_trash_run(&trash)?);
                }
                let run = trash_run.as_ref().expect("trash run was just initialized");
                remove_revalidated_entry(home, entry.namespace(), entry.path(), run, items.len())?;
                removed_bytes = removed_bytes
                    .checked_add(entry.bytes())
                    .ok_or(CacheExecutionError::ByteCountOverflow)?;
                items.push(execution_item(
                    entry,
                    Some(entry.bytes()),
                    CacheExecutionDisposition::Removed,
                ));
            }
        }
    }

    if let Some(run) = trash_run {
        run.finish()?;
    }
    Ok(CacheExecutionReport {
        removed_bytes,
        items,
    })
}

fn execution_item(
    entry: &super::CacheEntry,
    observed_bytes: Option<u64>,
    disposition: CacheExecutionDisposition,
) -> CacheExecutionItem {
    CacheExecutionItem::new(
        entry.namespace(),
        entry.path(),
        entry.bytes(),
        observed_bytes,
        disposition,
    )
}

enum RevalidatedEntry {
    Missing,
    ActiveLease { observed_bytes: u64 },
    Unclassified { observed_bytes: u64 },
    Unregistered,
    Stale { observed_bytes: u64 },
    Ready,
}

fn inventory_namespaces(
    home: &MorphirHome,
    ownership: &[CacheNamespace],
    limits: CacheInventoryLimits,
) -> Result<BTreeMap<String, Vec<super::CacheEntry>>, CacheExecutionError> {
    let mut inventories = BTreeMap::new();
    for namespace in ownership {
        if inventories.contains_key(namespace.name()) {
            return Err(CacheExecutionError::DuplicateNamespace {
                namespace: namespace.name().to_owned(),
            });
        }
        inventories.insert(
            namespace.name().to_owned(),
            inventory_cache_namespace(home, namespace, limits)?,
        );
    }
    Ok(inventories)
}

fn revalidate_entry(
    inventories: &BTreeMap<String, Vec<super::CacheEntry>>,
    namespace: &str,
    path: &str,
    planned_bytes: u64,
) -> RevalidatedEntry {
    let Some(inventory) = inventories.get(namespace) else {
        return RevalidatedEntry::Unregistered;
    };
    if let Some(observed) = inventory.iter().find(|entry| entry.path() == path) {
        return match observed.state() {
            CacheEntryState::Disposable { .. } if observed.bytes() == planned_bytes => {
                RevalidatedEntry::Ready
            }
            CacheEntryState::Disposable { .. } => RevalidatedEntry::Stale {
                observed_bytes: observed.bytes(),
            },
            CacheEntryState::ActiveLease { .. } => RevalidatedEntry::ActiveLease {
                observed_bytes: observed.bytes(),
            },
            CacheEntryState::Unclassified => RevalidatedEntry::Unclassified {
                observed_bytes: observed.bytes(),
            },
        };
    }
    let unsafe_ancestor = inventory.iter().find(|entry| {
        entry.state() == CacheEntryState::Unclassified
            && path
                .strip_prefix(entry.path())
                .is_some_and(|rest| rest.starts_with('/'))
    });
    match unsafe_ancestor {
        Some(entry) => RevalidatedEntry::Unclassified {
            observed_bytes: entry.bytes(),
        },
        None => RevalidatedEntry::Missing,
    }
}

fn remove_revalidated_entry(
    home: &MorphirHome,
    namespace: &str,
    relative: &str,
    trash_run: &TrashRun,
    index: usize,
) -> Result<(), CacheExecutionError> {
    let (parent, leaf, source) = open_removal_parent(home, namespace, relative)?;
    let destination_name = format!("{index:08x}");
    let destination = trash_run.path.join(&destination_name);
    parent
        .rename(&leaf, &trash_run.dir, &destination_name)
        .map_err(|error| io_error(&source, error))?;
    let metadata = trash_run
        .dir
        .symlink_metadata(&destination_name)
        .map_err(|source| io_error(&destination, source))?;
    if metadata.is_dir() && !cap_is_link_like(&metadata) {
        trash_run
            .dir
            .remove_dir_all(&destination_name)
            .map_err(|source| io_error(&destination, source))
    } else {
        trash_run
            .dir
            .remove_file(&destination_name)
            .map_err(|source| io_error(&destination, source))
    }
}

fn open_removal_parent(
    home: &MorphirHome,
    namespace: &str,
    relative: &str,
) -> Result<(Dir, String, PathBuf), CacheExecutionError> {
    let home_dir = Dir::open_ambient_dir(home.root(), ambient_authority())
        .map_err(|source| io_error(home.root(), source))?;
    let cache_path = home.cache_dir();
    let cache_dir = home_dir
        .open_dir_nofollow("cache")
        .map_err(|source| io_error(&cache_path, source))?;
    let namespace_path = cache_path.join(namespace);
    let mut parent = cache_dir
        .open_dir_nofollow(namespace)
        .map_err(|source| io_error(&namespace_path, source))?;
    let mut display_parent = namespace_path;
    let mut segments = relative.split('/').peekable();
    let mut leaf = None;
    while let Some(segment) = segments.next() {
        if segments.peek().is_none() {
            leaf = Some(segment.to_owned());
            break;
        }
        display_parent.push(segment);
        parent = parent
            .open_dir_nofollow(segment)
            .map_err(|source| io_error(&display_parent, source))?;
    }
    let leaf = leaf.expect("validated cache identities contain at least one segment");
    let source = display_parent.join(&leaf);
    Ok((parent, leaf, source))
}

struct MaintenanceTrash {
    dir: Dir,
    path: PathBuf,
}

struct TrashRun {
    root: Dir,
    dir: Dir,
    name: String,
    path: PathBuf,
}

impl TrashRun {
    fn finish(self) -> Result<(), CacheExecutionError> {
        let Self {
            root,
            dir,
            name,
            path,
        } = self;
        drop(dir);
        root.remove_dir(&name)
            .map_err(|source| io_error(&path, source))
    }
}

fn open_maintenance_trash(home: &MorphirHome) -> Result<MaintenanceTrash, CacheExecutionError> {
    fs::create_dir_all(home.root()).map_err(|source| io_error(home.root(), source))?;
    let home_dir = Dir::open_ambient_dir(home.root(), ambient_authority())
        .map_err(|source| io_error(home.root(), source))?;
    let temp_path = home.temp_dir();
    let temp_dir = open_or_create_directory(&home_dir, "tmp", &temp_path)?;
    let path = home.maintenance_trash_dir();
    let dir = open_or_create_directory(&temp_dir, "maintenance-trash", &path)?;
    Ok(MaintenanceTrash { dir, path })
}

fn open_or_create_directory(
    parent: &Dir,
    name: &str,
    display_path: &Path,
) -> Result<Dir, CacheExecutionError> {
    match parent.open_dir_nofollow(name) {
        Ok(directory) => Ok(directory),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match parent.create_dir(name) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(source) => return Err(io_error(display_path, source)),
            }
            parent
                .open_dir_nofollow(name)
                .map_err(|source| io_error(display_path, source))
        }
        Err(source) => Err(io_error(display_path, source)),
    }
}

fn sweep_existing_trash(trash: &MaintenanceTrash) -> Result<(), CacheExecutionError> {
    let mut entries = trash
        .dir
        .entries()
        .map_err(|source| io_error(&trash.path, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| io_error(&trash.path, source))?;
    entries.sort_by_key(cap_std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        let Some(name_text) = name.to_str() else {
            continue;
        };
        if !is_trash_run_name(name_text) {
            continue;
        }
        let path = trash.path.join(&name);
        let metadata = trash
            .dir
            .symlink_metadata(&name)
            .map_err(|source| io_error(&path, source))?;
        if cap_is_link_like(&metadata) || !metadata.is_dir() {
            return Err(CacheExecutionError::UnsafeMaintenancePath { path });
        }
        let run = trash
            .dir
            .open_dir_nofollow(&name)
            .map_err(|source| io_error(&path, source))?;
        remove_directory_contents(&run, &path)?;
        drop(run);
        trash
            .dir
            .remove_dir(&name)
            .map_err(|source| io_error(&path, source))?;
        debug!(
            event = "cache_cleanup_trash_recovered",
            "recovered interrupted trash run"
        );
    }
    Ok(())
}

fn remove_directory_contents(
    directory: &Dir,
    display_path: &Path,
) -> Result<(), CacheExecutionError> {
    let mut entries = directory
        .entries()
        .map_err(|source| io_error(display_path, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| io_error(display_path, source))?;
    entries.sort_by_key(cap_std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        let path = display_path.join(&name);
        let metadata = directory
            .symlink_metadata(&name)
            .map_err(|source| io_error(&path, source))?;
        if metadata.is_dir() && !cap_is_link_like(&metadata) {
            directory
                .remove_dir_all(&name)
                .map_err(|source| io_error(&path, source))?;
        } else {
            directory
                .remove_file(&name)
                .map_err(|source| io_error(&path, source))?;
        }
    }
    Ok(())
}

fn is_trash_run_name(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn create_trash_run(trash: &MaintenanceTrash) -> Result<TrashRun, CacheExecutionError> {
    for _ in 0..8 {
        let name = Uuid::new_v4().simple().to_string();
        let path = trash.path.join(&name);
        match trash.dir.create_dir(&name) {
            Ok(()) => {
                let dir = trash
                    .dir
                    .open_dir_nofollow(&name)
                    .map_err(|source| io_error(&path, source))?;
                let root = trash
                    .dir
                    .try_clone()
                    .map_err(|source| io_error(&trash.path, source))?;
                return Ok(TrashRun {
                    root,
                    dir,
                    name,
                    path,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(io_error(&path, source)),
        }
    }
    Err(io_error(
        &trash.path,
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a unique maintenance trash run",
        ),
    ))
}

struct MaintenanceGuard {
    file: File,
}

impl MaintenanceGuard {
    fn acquire(home: &MorphirHome) -> Result<Self, CacheExecutionError> {
        fs::create_dir_all(home.root()).map_err(|source| io_error(home.root(), source))?;
        let home_dir = Dir::open_ambient_dir(home.root(), ambient_authority())
            .map_err(|source| io_error(home.root(), source))?;
        let locks_path = home.locks_dir();
        let locks_dir = open_or_create_directory(&home_dir, "locks", &locks_path)?;
        let path = home.maintenance_lock_file();
        match locks_dir.symlink_metadata("maintenance.lock") {
            Ok(metadata) if cap_is_link_like(&metadata) || !metadata.is_file() => {
                return Err(CacheExecutionError::UnsafeMaintenancePath { path });
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(io_error(&path, source)),
        }
        let mut options = CapOpenOptions::new();
        options
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .follow(FollowSymlinks::No);
        let file = locks_dir
            .open_with("maintenance.lock", &options)
            .map_err(|source| io_error(&path, source))?
            .into_std();
        FileExt::lock_exclusive(&file).map_err(|source| io_error(&path, source))?;
        Ok(Self { file })
    }
}

impl Drop for MaintenanceGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn io_error(path: &Path, source: io::Error) -> CacheExecutionError {
    CacheExecutionError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(windows)]
fn cap_is_link_like(metadata: &CapMetadata) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || CapMetadataExt::file_attributes(metadata) & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn cap_is_link_like(metadata: &CapMetadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::{create_trash_run, open_maintenance_trash, remove_revalidated_entry};
    use crate::home::MorphirHome;
    use tempfile::TempDir;

    #[test]
    fn removal_refuses_a_link_like_source_ancestor() {
        let root = TempDir::new().unwrap();
        let home = MorphirHome::resolve_from(Some(root.path().as_os_str()), None).unwrap();
        let outside = TempDir::new().unwrap();
        std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
        std::fs::write(outside.path().join("keep"), b"outside").unwrap();
        if create_directory_link(outside.path(), &home.downloads_cache_dir().join("owned")).is_err()
        {
            return;
        }
        let trash = open_maintenance_trash(&home).unwrap();
        let trash_run = create_trash_run(&trash).unwrap();

        assert!(remove_revalidated_entry(&home, "downloads", "owned/keep", &trash_run, 0).is_err());
        assert_eq!(
            std::fs::read(outside.path().join("keep")).unwrap(),
            b"outside"
        );
    }

    #[cfg(unix)]
    fn create_directory_link(
        target: &std::path::Path,
        link: &std::path::Path,
    ) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_directory_link(
        target: &std::path::Path,
        link: &std::path::Path,
    ) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }
}
