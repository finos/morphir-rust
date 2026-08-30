//! Capability-confined native traversal and recognized-config capture.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use cap_std::fs::Dir;
use morphir_workspace::{FileEntry, FileTree, RelativePath};

use super::aliases::{
    AliasBudgets, DirectoryAlias, materialize_directory_aliases, record_directory_alias,
};

pub(super) fn build_tree_from_capability(
    root: &Dir,
    canonical_root: &Path,
    granted_root: &Path,
) -> Result<FileTree> {
    build_tree_with(
        root,
        canonical_root,
        granted_root,
        AliasBudgets::DEFAULT,
        &mut |_, _| {},
    )
}

pub(super) fn build_tree_with(
    root: &Dir,
    canonical_root: &Path,
    granted_root: &Path,
    budgets: AliasBudgets,
    boundary_hook: &mut dyn FnMut(BoundaryEvent, &RelativePath),
) -> Result<FileTree> {
    let mut entries = BTreeMap::from([(RelativePath::root(), FileEntry::Directory)]);
    let mut visited_directories = BTreeSet::from([RelativePath::root()]);
    let mut directory_aliases = Vec::new();
    Traversal {
        root,
        canonical_root,
        granted_root,
        visited_directories: &mut visited_directories,
        directory_aliases: &mut directory_aliases,
        entries: &mut entries,
        budgets,
        boundary_hook,
    }
    .walk(root, &RelativePath::root(), &RelativePath::root())?;
    materialize_directory_aliases(&mut directory_aliases, &mut entries, budgets)?;
    Ok(FileTree { entries })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BoundaryEvent {
    BeforeOpenDirectory,
    BeforeReadConfig,
}

struct Traversal<'a> {
    root: &'a Dir,
    canonical_root: &'a Path,
    granted_root: &'a Path,
    visited_directories: &'a mut BTreeSet<RelativePath>,
    directory_aliases: &'a mut Vec<DirectoryAlias>,
    entries: &'a mut BTreeMap<RelativePath, FileEntry>,
    budgets: AliasBudgets,
    boundary_hook: &'a mut dyn FnMut(BoundaryEvent, &RelativePath),
}

impl Traversal<'_> {
    fn walk(
        &mut self,
        directory: &Dir,
        canonical_directory: &RelativePath,
        lexical_directory: &RelativePath,
    ) -> Result<()> {
        let mut children = directory
            .entries()
            .with_context(|| format!("Failed to read directory `{}`", lexical_directory.as_str()))?
            .collect::<std::io::Result<Vec<_>>>()?;
        children.sort_by_key(|entry| entry.file_name());

        for child in children {
            let file_name = child.file_name();
            let canonical_child = join_native_path(canonical_directory, &file_name);
            let link_metadata = self
                .root
                .symlink_metadata(&canonical_child)
                .with_context(|| format!("Failed to inspect `{}`", canonical_child.display()))?;
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
                let target_metadata = self
                    .root
                    .metadata(relative_native_path(&canonical_target))
                    .with_context(|| {
                        format!(
                            "Failed to inspect symlink target `{}` for `{}`",
                            canonical_target.as_str(),
                            lexical_path.as_str()
                        )
                    })?;
                if target_metadata.is_dir() {
                    record_directory_alias(self.directory_aliases, self.budgets, || {
                        DirectoryAlias {
                            lexical_path,
                            canonical_target,
                        }
                    })?;
                } else if target_metadata.is_file() && is_recognized_config(&lexical_path) {
                    let confined_target = relative_native_path(&canonical_target).to_path_buf();
                    insert_config_file(
                        self.root,
                        self.entries,
                        lexical_path,
                        confined_target,
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
                self.entries
                    .insert(lexical_path.clone(), FileEntry::Directory);
                if self.visited_directories.insert(canonical_directory.clone()) {
                    (self.boundary_hook)(BoundaryEvent::BeforeOpenDirectory, &lexical_path);
                    let opened = self
                        .root
                        .open_dir(relative_native_path(&canonical_directory))
                        .with_context(|| {
                            format!(
                                "Failed to open confined directory `{}`",
                                lexical_path.as_str()
                            )
                        })?;
                    self.walk(&opened, &canonical_directory, &lexical_path)?;
                }
            } else if link_metadata.is_file() && is_recognized_config(&lexical_path) {
                insert_config_file(
                    self.root,
                    self.entries,
                    lexical_path,
                    canonical_child,
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

fn insert_config_file(
    root: &Dir,
    entries: &mut BTreeMap<RelativePath, FileEntry>,
    lexical_path: RelativePath,
    confined_path: PathBuf,
    boundary_hook: &mut dyn FnMut(BoundaryEvent, &RelativePath),
) -> Result<()> {
    boundary_hook(BoundaryEvent::BeforeReadConfig, &lexical_path);
    let text = root.read_to_string(&confined_path).with_context(|| {
        format!(
            "Failed to read confined UTF-8 Morphir configuration `{}` from `{}`",
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
