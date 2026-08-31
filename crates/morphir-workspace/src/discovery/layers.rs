//! Configuration layer parsing and precedence.

use morphir_config::{builtin_defaults, merge_all, parse_config};
use serde_json::{Map, Value};

use crate::{
    DiscoveryFailure, FileTree, RelativePath, WORKSPACE_CONFIG_AMBIGUOUS, WORKSPACE_CONFIG_INVALID,
    WORKSPACE_CONFIG_MISSING,
    config::{found_adjacent_user_candidates, found_primary_candidates},
};

use super::diagnostics::failure;

pub(super) struct Layer {
    pub(super) path: RelativePath,
    pub(super) value: Value,
}

pub(super) struct MemberConfigLayers<'a> {
    pub(super) system: &'a Value,
    pub(super) global: &'a Value,
    pub(super) shared_workspace: &'a Value,
    pub(super) member_primary: &'a Value,
    pub(super) shared_workspace_user: &'a Value,
    pub(super) member_user: Option<&'a Value>,
    pub(super) environment: &'a Value,
    pub(super) cli_overlay: &'a Value,
}

pub(super) fn optional_mount_layer(
    tree: Option<&FileTree>,
    mount_name: &str,
) -> Result<Option<Layer>, DiscoveryFailure> {
    tree.map(|tree| optional_layer(tree, &RelativePath::root(), mount_name))
        .transpose()
        .map(Option::flatten)
}

pub(super) fn required_layer(
    tree: &FileTree,
    directory: &RelativePath,
    description: &str,
) -> Result<Layer, DiscoveryFailure> {
    optional_layer(tree, directory, description)?.ok_or_else(|| {
        failure(
            WORKSPACE_CONFIG_MISSING,
            format!("no Morphir configuration found at {description}"),
            Some(directory.clone()),
        )
    })
}

fn optional_layer(
    tree: &FileTree,
    directory: &RelativePath,
    description: &str,
) -> Result<Option<Layer>, DiscoveryFailure> {
    let candidates = found_primary_candidates(tree, directory);
    match candidates.as_slice() {
        [] => Ok(None),
        [path] => parse_layer(tree, path).map(Some),
        paths => Err(ambiguous_failure(description, paths)),
    }
}

pub(super) fn optional_user_layer(
    tree: &FileTree,
    primary: &RelativePath,
) -> Result<Option<Layer>, DiscoveryFailure> {
    let candidates = found_adjacent_user_candidates(tree, primary);
    match candidates.as_slice() {
        [] => Ok(None),
        [path] => parse_layer(tree, path).map(Some),
        paths => Err(ambiguous_failure("workspace user override", paths)),
    }
}

fn parse_layer(tree: &FileTree, path: &RelativePath) -> Result<Layer, DiscoveryFailure> {
    let text = tree.file_text(path).expect("candidate must be a text file");
    let value = parse_config(path.as_str(), text).map_err(|error| {
        failure(
            WORKSPACE_CONFIG_INVALID,
            format!(
                "invalid Morphir configuration at `{path}`: {error}",
                path = path.as_str()
            ),
            Some(path.clone()),
        )
    })?;
    Ok(Layer {
        path: path.clone(),
        value,
    })
}

fn ambiguous_failure(description: &str, paths: &[RelativePath]) -> DiscoveryFailure {
    let listed = paths
        .iter()
        .map(|path| format!("`{}`", path.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    failure(
        WORKSPACE_CONFIG_AMBIGUOUS,
        format!("multiple Morphir configurations found for {description}: {listed}"),
        paths.first().cloned(),
    )
}

pub(super) fn member_effective_config(layers: MemberConfigLayers<'_>) -> Value {
    let empty = Value::Object(Map::new());
    merge_all([
        &builtin_defaults(),
        layers.system,
        layers.global,
        layers.shared_workspace,
        layers.member_primary,
        layers.shared_workspace_user,
        layers.member_user.unwrap_or(&empty),
        layers.environment,
        layers.cli_overlay,
    ])
}

pub(super) fn without_project_or_workspace(value: &Value) -> Value {
    let mut stripped = value.clone();
    if let Some(object) = stripped.as_object_mut() {
        object.remove("project");
        object.remove("workspace");
    }
    stripped
}
