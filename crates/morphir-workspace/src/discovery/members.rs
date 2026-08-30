//! Workspace member discovery and isolated member failures.

use morphir_config::parse_config;
use serde_json::Value;

use crate::{
    FileTree, ProjectSnapshot, ProjectState, RelativePath, WORKSPACE_MEMBER_INVALID,
    WORKSPACE_PATH_NOT_CONFINED,
    config::{found_adjacent_user_candidates, found_primary_candidates},
};

use super::{
    EffectiveConfigCollector,
    decoding::{ProjectDecodeError, decode_project},
    diagnostics::error_project,
    layers::{Layer, MemberConfigLayers, member_effective_config},
};

struct MemberProblem {
    message: String,
    path: Option<RelativePath>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn discover_member(
    tree: &FileTree,
    directory: &RelativePath,
    system: &Value,
    global: &Value,
    shared_workspace: &Value,
    shared_workspace_user: &Value,
    environment: &Value,
    cli_overlay: &Value,
    collector: &mut Option<&mut dyn EffectiveConfigCollector>,
) -> Option<ProjectSnapshot> {
    let primary_candidates = found_primary_candidates(tree, directory);
    let primary_path = match primary_candidates.as_slice() {
        [] => return None,
        [path] => path.clone(),
        paths => {
            let listed = paths
                .iter()
                .map(|path| format!("`{}`", path.as_str()))
                .collect::<Vec<_>>()
                .join(", ");
            return Some(error_project(
                directory,
                paths.first().cloned(),
                WORKSPACE_MEMBER_INVALID,
                format!("multiple member configurations found: {listed}"),
            ));
        }
    };
    let member_primary = match parse_member_layer(tree, &primary_path) {
        Ok(layer) => layer,
        Err(problem) => {
            return Some(error_project(
                directory,
                problem.path,
                WORKSPACE_MEMBER_INVALID,
                problem.message,
            ));
        }
    };
    let member_user = match member_user_layer(tree, &primary_path) {
        Ok(layer) => layer,
        Err(problem) => {
            return Some(error_project(
                directory,
                problem.path,
                WORKSPACE_MEMBER_INVALID,
                problem.message,
            ));
        }
    };
    let effective = member_effective_config(MemberConfigLayers {
        system,
        global,
        shared_workspace,
        member_primary: &member_primary.value,
        shared_workspace_user,
        member_user: member_user.as_ref().map(|layer| &layer.value),
        environment,
        cli_overlay,
    });
    let project = match decode_project(&effective, directory) {
        Ok(project) => project,
        Err(ProjectDecodeError::Invalid(message)) => {
            return Some(error_project(
                directory,
                Some(primary_path),
                WORKSPACE_MEMBER_INVALID,
                message,
            ));
        }
        Err(ProjectDecodeError::NotConfined(message)) => {
            return Some(error_project(
                directory,
                Some(primary_path),
                WORKSPACE_PATH_NOT_CONFINED,
                message,
            ));
        }
    };

    if let Some(collector) = collector.as_deref_mut() {
        collector.project(directory, &effective);
    }
    Some(ProjectSnapshot {
        name: project.name,
        version: project.version,
        relative_path: directory.clone(),
        config_anchor: Some(primary_path),
        source_directory: project.source_directory,
        state: ProjectState::Unloaded,
        diagnostics: Vec::new(),
    })
}

fn parse_member_layer(tree: &FileTree, path: &RelativePath) -> Result<Layer, MemberProblem> {
    let text = tree.file_text(path).expect("candidate must be a text file");
    let value = parse_config(path.as_str(), text).map_err(|error| MemberProblem {
        message: format!(
            "invalid member configuration at `{}`: {error}",
            path.as_str()
        ),
        path: Some(path.clone()),
    })?;
    Ok(Layer {
        path: path.clone(),
        value,
    })
}

fn member_user_layer(
    tree: &FileTree,
    primary: &RelativePath,
) -> Result<Option<Layer>, MemberProblem> {
    let candidates = found_adjacent_user_candidates(tree, primary);
    match candidates.as_slice() {
        [] => Ok(None),
        [path] => parse_member_layer(tree, path).map(Some),
        paths => Err(MemberProblem {
            message: format!(
                "multiple member user overrides found: {}",
                paths
                    .iter()
                    .map(|path| format!("`{}`", path.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            path: paths.first().cloned(),
        }),
    }
}
