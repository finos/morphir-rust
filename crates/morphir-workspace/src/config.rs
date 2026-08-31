//! Pure configuration candidate discovery over a portable file tree.

use crate::{FileTree, RelativePath};

/// The supported implicit modern primary paths, relative to a candidate directory.
pub const MODERN_PRIMARY_PATHS: [&str; 6] = [
    "morphir.toml",
    "morphir.yaml",
    ".morphir/morphir.toml",
    ".morphir/morphir.yaml",
    ".config/morphir/config.toml",
    ".config/morphir/config.yaml",
];

/// Builds the six implicit modern primary candidates in their canonical order.
#[must_use]
pub fn modern_primary_candidates(directory: &RelativePath) -> Vec<RelativePath> {
    MODERN_PRIMARY_PATHS
        .into_iter()
        .map(|candidate| {
            directory
                .join(candidate)
                .expect("built-in candidate paths are confined")
        })
        .collect()
}

/// Finds primary configuration candidates at `directory`.
///
/// Modern TOML/YAML candidates take precedence as a set. Legacy `morphir.json`
/// is considered only when no modern candidate exists.
#[must_use]
pub fn found_primary_candidates(tree: &FileTree, directory: &RelativePath) -> Vec<RelativePath> {
    let modern = modern_primary_candidates(directory)
        .into_iter()
        .filter(|candidate| tree.contains_file(candidate))
        .collect::<Vec<_>>();
    if !modern.is_empty() {
        return modern;
    }

    let legacy = directory
        .join("morphir.json")
        .expect("legacy candidate path is confined");
    tree.contains_file(&legacy)
        .then_some(legacy)
        .into_iter()
        .collect()
}

/// Builds adjacent user override candidates for a supported modern primary.
#[must_use]
pub fn adjacent_user_candidates(primary: &RelativePath) -> Vec<RelativePath> {
    let primary_name = primary.as_str().rsplit('/').next().unwrap_or_default();
    let names = match primary_name {
        "morphir.toml" | "morphir.yaml" => Some(["morphir.user.toml", "morphir.user.yaml"]),
        "config.toml" | "config.yaml" => Some(["config.user.toml", "config.user.yaml"]),
        _ => None,
    };

    names
        .into_iter()
        .flatten()
        .map(|name| {
            primary
                .parent()
                .join(name)
                .expect("built-in user override paths are confined")
        })
        .collect()
}

/// Finds every adjacent user override candidate in deterministic order.
#[must_use]
pub fn found_adjacent_user_candidates(
    tree: &FileTree,
    primary: &RelativePath,
) -> Vec<RelativePath> {
    adjacent_user_candidates(primary)
        .into_iter()
        .filter(|candidate| tree.contains_file(candidate))
        .collect()
}
