//! Capability-confined native traversal and recognized-config capture.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use cap_std::fs::Dir;
use morphir_workspace::{FileEntry, FileTree, RelativePath};

use super::aliases::{
    AliasBudgets, DirectoryAlias, materialize_directory_aliases, record_directory_alias,
};
use super::budget::{PayloadBudget, PayloadKind, resource_limit};

pub(super) fn build_tree_from_capability(
    root: &Dir,
    canonical_root: &Path,
    granted_root: &Path,
    alias_budgets: AliasBudgets,
    traversal_budgets: TraversalBudgets,
    payload: &mut PayloadBudget,
    classify_config: &dyn Fn(&RelativePath) -> PayloadKind,
) -> Result<FileTree> {
    build_tree_with_payload(
        root,
        canonical_root,
        granted_root,
        alias_budgets,
        traversal_budgets,
        payload,
        classify_config,
        &mut |_, _| {},
    )
}

#[cfg(test)]
pub(super) fn build_tree_with(
    root: &Dir,
    canonical_root: &Path,
    granted_root: &Path,
    alias_budgets: AliasBudgets,
    traversal_budgets: TraversalBudgets,
    boundary_hook: &mut dyn FnMut(BoundaryEvent, &RelativePath),
) -> Result<FileTree> {
    let mut payload = PayloadBudget::new(traversal_budgets.config_bytes);
    let account_all = |_: &RelativePath| PayloadKind::Final;
    build_tree_with_payload(
        root,
        canonical_root,
        granted_root,
        alias_budgets,
        traversal_budgets,
        &mut payload,
        &account_all,
        boundary_hook,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_tree_with_payload(
    root: &Dir,
    canonical_root: &Path,
    granted_root: &Path,
    alias_budgets: AliasBudgets,
    traversal_budgets: TraversalBudgets,
    payload: &mut PayloadBudget,
    classify_config: &dyn Fn(&RelativePath) -> PayloadKind,
    boundary_hook: &mut dyn FnMut(BoundaryEvent, &RelativePath),
) -> Result<FileTree> {
    let mut entries = BTreeMap::from([(RelativePath::root(), FileEntry::Directory)]);
    let mut visited_directories = BTreeSet::from([RelativePath::root()]);
    let mut directory_aliases = Vec::new();
    let mut stats = TraversalStats::with_root(traversal_budgets)?;
    Traversal {
        root,
        canonical_root,
        granted_root,
        visited_directories: &mut visited_directories,
        directory_aliases: &mut directory_aliases,
        entries: &mut entries,
        alias_budgets,
        traversal_budgets,
        stats: &mut stats,
        payload,
        classify_config,
        boundary_hook,
    }
    .walk(root, &RelativePath::root(), &RelativePath::root(), 0)?;
    materialize_directory_aliases(
        &mut directory_aliases,
        &mut entries,
        alias_budgets,
        payload,
        classify_config,
    )?;
    Ok(FileTree { entries })
}

#[derive(Clone, Copy, Debug)]
pub(super) struct TraversalBudgets {
    pub(super) real_directories: usize,
    pub(super) real_entries: usize,
    pub(super) max_depth: usize,
    pub(super) config_bytes: usize,
}

impl TraversalBudgets {
    pub(super) const DEFAULT: Self = Self {
        real_directories: 65_536,
        real_entries: 262_144,
        max_depth: 128,
        config_bytes: 64 * 1024 * 1024,
    };
}

#[derive(Default)]
struct TraversalStats {
    real_directories: usize,
    real_entries: usize,
}

impl TraversalStats {
    fn with_root(budgets: TraversalBudgets) -> Result<Self> {
        let mut stats = Self::default();
        stats.enter_directory(budgets)?;
        Ok(stats)
    }

    fn enter_directory(&mut self, budgets: TraversalBudgets) -> Result<()> {
        increment_bounded(
            &mut self.real_directories,
            budgets.real_directories,
            "real directories",
        )
    }

    fn record_entry(&mut self, budgets: TraversalBudgets) -> Result<()> {
        increment_bounded(&mut self.real_entries, budgets.real_entries, "real entries")
    }

    fn check_depth(&self, depth: usize, budgets: TraversalBudgets) -> Result<()> {
        if depth > budgets.max_depth {
            return traversal_resource_limit("depth", budgets.max_depth);
        }
        Ok(())
    }
}

fn increment_bounded(value: &mut usize, limit: usize, resource: &str) -> Result<()> {
    if *value >= limit {
        return traversal_resource_limit(resource, limit);
    }
    *value += 1;
    Ok(())
}

fn traversal_resource_limit<T>(resource: &str, limit: usize) -> Result<T> {
    bail!(
        "workspace.traversal.resource-limit: confined native traversal exceeded fixed {resource} budget {limit}"
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BoundaryEvent {
    InspectEntry,
    InspectSymlinkTarget,
    OpenDirectory,
    ReadConfig,
}

struct Traversal<'a> {
    root: &'a Dir,
    canonical_root: &'a Path,
    granted_root: &'a Path,
    visited_directories: &'a mut BTreeSet<RelativePath>,
    directory_aliases: &'a mut Vec<DirectoryAlias>,
    entries: &'a mut BTreeMap<RelativePath, FileEntry>,
    alias_budgets: AliasBudgets,
    traversal_budgets: TraversalBudgets,
    stats: &'a mut TraversalStats,
    payload: &'a mut PayloadBudget,
    classify_config: &'a dyn Fn(&RelativePath) -> PayloadKind,
    boundary_hook: &'a mut dyn FnMut(BoundaryEvent, &RelativePath),
}

impl Traversal<'_> {
    fn walk(
        &mut self,
        directory: &Dir,
        canonical_directory: &RelativePath,
        lexical_directory: &RelativePath,
        depth: usize,
    ) -> Result<()> {
        let children = directory.entries().with_context(|| {
            format!(
                "workspace.traversal.unreadable: Failed to read granted directory `{}`",
                lexical_directory.as_str()
            )
        })?;
        let mut buffered_children = Vec::new();
        for child in children {
            let child = child.with_context(|| {
                format!(
                    "workspace.traversal.unreadable: Failed to enumerate granted directory `{}`",
                    lexical_directory.as_str()
                )
            })?;
            self.stats.record_entry(self.traversal_budgets)?;
            buffered_children.push(child);
        }
        buffered_children.sort_by_key(|entry| entry.file_name());

        for child in buffered_children {
            let file_name = child.file_name();
            let canonical_child = join_native_path(canonical_directory, &file_name);
            let lexical_path = file_name
                .to_str()
                .and_then(|file_name| lexical_directory.join(file_name).ok());
            if let Some(lexical_path) = &lexical_path {
                (self.boundary_hook)(BoundaryEvent::InspectEntry, lexical_path);
            }
            let link_metadata =
                self.root
                    .symlink_metadata(&canonical_child)
                    .with_context(|| {
                        format!(
                            "workspace.traversal.unreadable: Failed to inspect confined entry `{}`",
                            canonical_child.display()
                        )
                    })?;
            let Some(file_name) = file_name.to_str() else {
                if link_metadata.file_type().is_symlink() {
                    confined_canonicalize(
                        self.root,
                        self.canonical_root,
                        self.granted_root,
                        &canonical_child,
                        None,
                    )?;
                }
                continue;
            };
            let lexical_path = lexical_directory.join(file_name).map_err(|error| {
                anyhow!(
                    "workspace.path.not-confined: invalid path below `{}`: {error}",
                    lexical_directory.as_str()
                )
            })?;
            if link_metadata.file_type().is_symlink() {
                let canonical_target = confined_canonicalize(
                    self.root,
                    self.canonical_root,
                    self.granted_root,
                    &canonical_child,
                    Some(&lexical_path),
                )?;
                (self.boundary_hook)(BoundaryEvent::InspectSymlinkTarget, &lexical_path);
                let target_metadata = self
                    .root
                    .metadata(relative_native_path(&canonical_target))
                    .with_context(|| {
                        format!(
                            "workspace.traversal.unreadable: Failed to inspect confined symlink target `{}` for `{}`",
                            canonical_target.as_str(),
                            lexical_path.as_str()
                        )
                    })?;
                if target_metadata.is_dir() {
                    record_directory_alias(self.directory_aliases, self.alias_budgets, || {
                        DirectoryAlias {
                            lexical_path,
                            canonical_target,
                        }
                    })?;
                } else if target_metadata.is_file() && is_recognized_config(&lexical_path) {
                    let confined_target = relative_native_path(&canonical_target).to_path_buf();
                    let payload_kind = (self.classify_config)(&lexical_path);
                    insert_config_file(
                        self.root,
                        self.entries,
                        lexical_path,
                        confined_target,
                        self.traversal_budgets,
                        self.payload,
                        payload_kind,
                        self.boundary_hook,
                    )?;
                }
                continue;
            }

            if link_metadata.is_dir() {
                let canonical_directory = confined_canonicalize(
                    self.root,
                    self.canonical_root,
                    self.granted_root,
                    &canonical_child,
                    Some(&lexical_path),
                )?;
                if !self.visited_directories.contains(&canonical_directory) {
                    let child_depth = depth.saturating_add(1);
                    self.stats
                        .check_depth(child_depth, self.traversal_budgets)?;
                    self.stats.enter_directory(self.traversal_budgets)?;
                    self.visited_directories.insert(canonical_directory.clone());
                    self.entries
                        .insert(lexical_path.clone(), FileEntry::Directory);
                    (self.boundary_hook)(BoundaryEvent::OpenDirectory, &lexical_path);
                    let opened = self
                        .root
                        .open_dir(relative_native_path(&canonical_directory))
                        .with_context(|| {
                            format!(
                                "workspace.traversal.unreadable: Failed to open confined directory `{}`",
                                lexical_path.as_str()
                            )
                        })?;
                    self.walk(&opened, &canonical_directory, &lexical_path, child_depth)?;
                } else {
                    self.entries
                        .insert(lexical_path.clone(), FileEntry::Directory);
                }
            } else if link_metadata.is_file() && is_recognized_config(&lexical_path) {
                let payload_kind = (self.classify_config)(&lexical_path);
                insert_config_file(
                    self.root,
                    self.entries,
                    lexical_path,
                    canonical_child,
                    self.traversal_budgets,
                    self.payload,
                    payload_kind,
                    self.boundary_hook,
                )?;
            }
        }
        Ok(())
    }
}

fn native_path_to_relative(path: &Path) -> Result<RelativePath> {
    if path.as_os_str().is_empty() || path == Path::new(".") {
        return Ok(RelativePath::root());
    }
    let normalized = path
        .components()
        .map(|component| {
            component.as_os_str().to_str().ok_or_else(|| {
                anyhow!(
                    "workspace.path.not-confined: confined path `{}` is not UTF-8",
                    path.display()
                )
            })
        })
        .collect::<Result<Vec<_>>>()?
        .join("/");
    RelativePath::parse(normalized).map_err(|error| anyhow!(error))
}

fn confined_canonicalize(
    root: &Dir,
    canonical_root: &Path,
    granted_root: &Path,
    path: &Path,
    lexical_path: Option<&RelativePath>,
) -> Result<RelativePath> {
    match root.canonicalize(path) {
        Ok(path) => native_path_to_relative(&path),
        Err(first_error) => {
            let target = root.read_link_contents(path).ok();
            if let Some(target) = target.as_deref().filter(|target| target.is_absolute()) {
                for accepted_root in [canonical_root, granted_root] {
                    if let Ok(relative) = target.strip_prefix(accepted_root)
                        && let Ok(path) = root.canonicalize(relative)
                    {
                        return native_path_to_relative(&path);
                    }
                }
            }
            let link = lexical_path
                .map(RelativePath::as_str)
                .unwrap_or_else(|| path.to_str().unwrap_or("<non-UTF-8>"));
            let target = target
                .map(|target| target.display().to_string())
                .unwrap_or_else(|| "<unresolved>".to_owned());
            bail!(
                "workspace.path.not-confined: `{link}` resolves through `{target}` outside the bound development root or could not be resolved safely: {first_error}"
            )
        }
    }
}

fn relative_native_path(path: &RelativePath) -> &Path {
    Path::new(path.as_str())
}

fn join_native_path(directory: &RelativePath, name: &OsStr) -> PathBuf {
    if directory.as_str() == "." {
        PathBuf::from(name)
    } else {
        Path::new(directory.as_str()).join(name)
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_config_file(
    root: &Dir,
    entries: &mut BTreeMap<RelativePath, FileEntry>,
    lexical_path: RelativePath,
    confined_path: PathBuf,
    budgets: TraversalBudgets,
    payload: &mut PayloadBudget,
    payload_kind: PayloadKind,
    boundary_hook: &mut dyn FnMut(BoundaryEvent, &RelativePath),
) -> Result<()> {
    let remaining = match payload_kind {
        PayloadKind::Final => payload.remaining(),
        PayloadKind::Transient => payload.transient_remaining(),
        PayloadKind::Omit => return Ok(()),
    };
    let metadata = root.metadata(&confined_path).with_context(|| {
        format!(
            "workspace.traversal.unreadable: Failed to inspect confined Morphir configuration `{}` from `{}`",
            lexical_path.as_str(),
            confined_path.display()
        )
    })?;
    if metadata.len() > remaining as u64 {
        return resource_limit(budgets.config_bytes);
    }
    boundary_hook(BoundaryEvent::ReadConfig, &lexical_path);
    let file = root.open(&confined_path).with_context(|| {
        format!(
            "workspace.traversal.unreadable: Failed to read confined UTF-8 Morphir configuration `{}` from `{}`",
            lexical_path.as_str(),
            confined_path.display()
        )
    })?;
    let read_limit = u64::try_from(remaining.saturating_add(1)).unwrap_or(u64::MAX);
    let mut bytes = Vec::new();
    file.take(read_limit).read_to_end(&mut bytes).with_context(|| {
        format!(
            "workspace.traversal.unreadable: Failed to read confined UTF-8 Morphir configuration `{}` from `{}`",
            lexical_path.as_str(),
            confined_path.display()
        )
    })?;
    match payload_kind {
        PayloadKind::Final => payload.reserve(bytes.len())?,
        PayloadKind::Transient => payload.reserve_transient(bytes.len())?,
        PayloadKind::Omit => unreachable!("omitted payload returned before reading"),
    }
    let text = String::from_utf8(bytes).with_context(|| {
        format!(
            "workspace.traversal.unreadable: Failed to read confined UTF-8 Morphir configuration `{}` from `{}`",
            lexical_path.as_str(),
            confined_path.display()
        )
    })?;
    entries.insert(lexical_path, FileEntry::File { text });
    Ok(())
}

fn is_recognized_config(path: &RelativePath) -> bool {
    let components = path.as_str().split('/').collect::<Vec<_>>();
    let Some(name) = components.last().copied() else {
        return false;
    };
    match name {
        "morphir.toml" | "morphir.yaml" | "morphir.json" | "morphir.user.toml"
        | "morphir.user.yaml" => true,
        "config.toml" | "config.yaml" | "config.user.toml" | "config.user.yaml" => {
            components.len() >= 3
                && components[components.len() - 2] == "morphir"
                && components[components.len() - 3] == ".config"
        }
        _ => false,
    }
}
