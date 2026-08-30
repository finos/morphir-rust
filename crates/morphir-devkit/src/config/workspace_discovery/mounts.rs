//! Native configuration mounts, user overrides, and environment selection.

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use morphir_common::config::env::env_config_value;
use morphir_workspace::{FileEntry, FileTree, RelativePath};

use super::super::{
    discovery::discover_config_candidates,
    sources::{ConfigLoadOptions, EnvSelection, SourceSelection},
};

pub(super) fn selected_mount(
    selection: &SourceSelection,
    candidates: impl FnOnce() -> Vec<PathBuf>,
    description: &str,
) -> Result<Option<FileTree>> {
    let selected = match selection {
        SourceSelection::Skip => return Ok(None),
        SourceSelection::Explicit(path) => match fs::symlink_metadata(path) {
            Ok(_) => Some(path.clone()),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Failed to inspect explicit {description} config: {}",
                        path.display()
                    )
                });
            }
        },
        SourceSelection::Discover => discover_config_candidates(&candidates())?,
    };
    selected
        .map(|path| config_mount(&path, description))
        .transpose()
}

fn config_mount(path: &Path, description: &str) -> Result<FileTree> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("Failed to inspect {description} config: {}", path.display()))?;
    let read_path = if metadata.file_type().is_symlink() {
        fs::canonicalize(path).with_context(|| {
            format!("Failed to resolve {description} config: {}", path.display())
        })?
    } else {
        path.to_path_buf()
    };
    let virtual_name = virtual_primary_name(path).ok_or_else(|| {
        anyhow!(
            "Unsupported {description} config serialization at {}; expected TOML, YAML, or JSON",
            path.display()
        )
    })?;
    let text = fs::read_to_string(&read_path)
        .with_context(|| format!("Failed to read {description} config: {}", path.display()))?;
    Ok(FileTree {
        entries: BTreeMap::from([
            (RelativePath::root(), FileEntry::Directory),
            (
                RelativePath::parse(virtual_name).expect("virtual primary name is confined"),
                FileEntry::File { text },
            ),
        ]),
    })
}

fn virtual_primary_name(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("toml") => Some("morphir.toml"),
        Some("yaml" | "yml") => Some("morphir.yaml"),
        Some("json") => Some("morphir.json"),
        _ => None,
    }
}

pub(super) fn apply_user_override_selection(
    tree: &mut FileTree,
    selection: &SourceSelection,
) -> Result<()> {
    match selection {
        SourceSelection::Discover => Ok(()),
        SourceSelection::Skip => {
            tree.entries.retain(|path, entry| {
                !is_user_override_path(path) || !matches!(entry, FileEntry::File { .. })
            });
            Ok(())
        }
        SourceSelection::Explicit(source) => {
            let primary = modern_root_primary(tree, source)?;
            let candidates = morphir_workspace::config::adjacent_user_candidates(&primary);
            for candidate in &candidates {
                if matches!(tree.entries.get(candidate), Some(FileEntry::File { .. })) {
                    tree.entries.remove(candidate);
                }
            }
            materialize_explicit_user_override(tree, source, &primary)
        }
    }
}

fn is_user_override_path(path: &RelativePath) -> bool {
    matches!(
        path.as_str().rsplit('/').next(),
        Some("morphir.user.toml" | "morphir.user.yaml" | "config.user.toml" | "config.user.yaml")
    )
}

fn modern_root_primary(tree: &FileTree, source: &Path) -> Result<RelativePath> {
    let root = RelativePath::root();
    let primaries = morphir_workspace::config::found_primary_candidates(tree, &root);
    match primaries.as_slice() {
        [primary] if primary.as_str() != "morphir.json" => Ok(primary.clone()),
        [..] => {
            bail!(
                "Cannot apply explicit user override {}; explicit user overrides require exactly one modern TOML/YAML root config",
                source.display()
            )
        }
    }
}

fn materialize_explicit_user_override(
    tree: &mut FileTree,
    source: &Path,
    primary: &RelativePath,
) -> Result<()> {
    let source_name = source.display();
    let serialization = match source
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("toml") => "toml",
        Some("yaml" | "yml") => "yaml",
        _ => {
            bail!(
                "Explicit user override {source_name} is unsupported; explicit user overrides require a modern TOML/YAML root config and TOML/YAML override"
            )
        }
    };
    let text = fs::read_to_string(source)
        .with_context(|| format!("Failed to read explicit user override: {source_name}"))?;
    morphir_config::parse_config(&source.to_string_lossy(), &text)
        .with_context(|| format!("Invalid explicit user override: {source_name}"))?;

    let candidates = morphir_workspace::config::adjacent_user_candidates(primary);
    let target = candidates
        .into_iter()
        .find(|candidate| candidate.as_str().ends_with(serialization))
        .ok_or_else(|| {
            anyhow!(
                "Cannot apply explicit user override {source_name}; explicit user overrides require a modern TOML/YAML root config"
            )
        })?;
    if tree.entries.contains_key(&target) {
        bail!(
            "Cannot materialize explicit user override {} at `{}` because that path is already occupied",
            source.display(),
            target.as_str()
        );
    }
    tree.entries.insert(target, FileEntry::File { text });
    Ok(())
}

pub(super) fn selected_environment(options: &ConfigLoadOptions) -> BTreeMap<String, String> {
    let vars = match &options.env {
        EnvSelection::Skip => return BTreeMap::new(),
        EnvSelection::Explicit(vars) => vars.clone(),
        EnvSelection::Process => std::env::vars_os()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect(),
    };
    let configured_prefix = options
        .env_prefix
        .trim_end_matches('_')
        .to_ascii_uppercase();
    let prefix = format!("{configured_prefix}_");

    vars.into_iter()
        .filter(|(key, value)| {
            key != "MORPHIR_HOME"
                && key != "MORPHIR_LOG_DIR"
                && env_config_value(&options.env_prefix, [(key, value)])
                    .as_object()
                    .is_some_and(|object| !object.is_empty())
        })
        .filter_map(|(key, value)| {
            let uppercase = key.to_ascii_uppercase();
            uppercase.strip_prefix(&prefix).map(|suffix| {
                let suffix = suffix.trim_start_matches('_');
                let portable_key = if configured_prefix == "MORPHIR" {
                    format!("MORPHIR_{suffix}")
                } else {
                    format!("MORPHIR__{suffix}")
                };
                (portable_key, value)
            })
        })
        .collect()
}
