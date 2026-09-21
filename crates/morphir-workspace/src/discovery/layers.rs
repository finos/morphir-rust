//! Configuration layer parsing and precedence.

use morphir_config::{builtin_defaults, env_config_value, merge_all, parse_config};
use serde_json::{Map, Value};

use crate::{
    DiscoveryFailure, DiscoveryRequest, FileTree, RelativePath, WORKSPACE_CONFIG_AMBIGUOUS,
    WORKSPACE_CONFIG_INVALID, WORKSPACE_CONFIG_MISSING,
    config::{found_adjacent_user_candidates, found_primary_candidates},
};

use super::diagnostics::failure;

pub(super) struct Layer {
    pub(super) path: RelativePath,
    pub(super) value: Value,
}

/// The configuration layers every discovery path derives the same way:
/// the system and Morphir Home mounts (each stripped of `project` and
/// `workspace` sections, since neither mount may set either), and the
/// request's environment values.
///
/// What each path does with these layers — which `merge_all` list they enter,
/// alongside which other layers — legitimately differs between the
/// manifest-projects and ad-hoc-sources paths. Only the derivation is shared.
pub(super) struct SharedLayers {
    pub(super) system_value: Option<Value>,
    pub(super) global_value: Option<Value>,
    pub(super) environment: Value,
}

pub(super) fn shared_layers(request: &DiscoveryRequest) -> Result<SharedLayers, DiscoveryFailure> {
    let system = optional_mount_layer(request.system_config.as_ref(), "system configuration")?;
    let global = optional_mount_layer(request.morphir_home.as_ref(), "Morphir Home")?;
    let system_value = system
        .as_ref()
        .map(|layer| without_project_or_workspace(&layer.value));
    let global_value = global
        .as_ref()
        .map(|layer| without_project_or_workspace(&layer.value));
    let environment = env_config_value(
        "MORPHIR",
        request
            .environment
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    Ok(SharedLayers {
        system_value,
        global_value,
        environment,
    })
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
