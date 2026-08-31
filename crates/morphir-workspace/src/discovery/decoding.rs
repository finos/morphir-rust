//! Workspace and project configuration decoding.

use serde::Deserialize;
use serde_json::Value;

use crate::{
    DiscoveryFailure, ProjectSnapshot, ProjectState, RelativePath, WORKSPACE_CONFIG_INVALID,
    WORKSPACE_PATH_NOT_CONFINED,
};

use super::diagnostics::failure;

#[derive(Debug, Default, Deserialize)]
pub(super) struct WorkspaceView {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) members: Vec<String>,
    #[serde(default)]
    pub(super) exclude: Vec<String>,
    #[serde(default)]
    pub(super) default_member: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProjectView {
    name: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default = "default_source_directory")]
    source_directory: String,
}

pub(super) enum ProjectDecodeError {
    Invalid(String),
    NotConfined(String),
}

pub(super) struct DecodedProject {
    pub(super) name: String,
    pub(super) version: Option<String>,
    pub(super) source_directory: RelativePath,
}

pub(super) fn decode_workspace(
    effective: &Value,
    path: &RelativePath,
) -> Result<WorkspaceView, DiscoveryFailure> {
    match effective.get("workspace") {
        None => Ok(WorkspaceView::default()),
        Some(value) => {
            let workspace: WorkspaceView =
                serde_json::from_value(value.clone()).map_err(|error| {
                    failure(
                        WORKSPACE_CONFIG_INVALID,
                        format!("invalid workspace section at `{}`: {error}", path.as_str()),
                        Some(path.clone()),
                    )
                })?;
            if let Some(default_member) = &workspace.default_member {
                RelativePath::parse(default_member.clone()).map_err(|error| {
                    failure(
                        WORKSPACE_PATH_NOT_CONFINED,
                        error.to_string(),
                        Some(path.clone()),
                    )
                })?;
            }
            Ok(workspace)
        }
    }
}

pub(super) fn decode_root_project(
    effective: &Value,
    anchor: &RelativePath,
) -> Result<ProjectSnapshot, DiscoveryFailure> {
    let project =
        decode_project(effective, &RelativePath::root()).map_err(|problem| match problem {
            ProjectDecodeError::Invalid(message) => failure(
                WORKSPACE_CONFIG_INVALID,
                format!("invalid root project at `{}`: {message}", anchor.as_str()),
                Some(anchor.clone()),
            ),
            ProjectDecodeError::NotConfined(message) => {
                failure(WORKSPACE_PATH_NOT_CONFINED, message, Some(anchor.clone()))
            }
        })?;
    Ok(ProjectSnapshot {
        name: project.name,
        version: project.version,
        relative_path: RelativePath::root(),
        config_anchor: Some(anchor.clone()),
        source_directory: project.source_directory,
        state: ProjectState::Unloaded,
        diagnostics: Vec::new(),
    })
}

pub(super) fn decode_project(
    effective: &Value,
    project_path: &RelativePath,
) -> Result<DecodedProject, ProjectDecodeError> {
    let value = effective
        .get("project")
        .ok_or_else(|| ProjectDecodeError::Invalid("missing project section".to_owned()))?;
    let project: ProjectView = serde_json::from_value(value.clone()).map_err(|error| {
        ProjectDecodeError::Invalid(format!("invalid project section: {error}"))
    })?;
    let source_directory = RelativePath::parse(project.source_directory)
        .map_err(|error| ProjectDecodeError::NotConfined(error.to_string()))?;
    project_path
        .join(source_directory.as_str())
        .map_err(|error| ProjectDecodeError::NotConfined(error.to_string()))?;
    Ok(DecodedProject {
        name: project.name,
        version: project.version,
        source_directory,
    })
}

fn default_source_directory() -> String {
    "src".to_owned()
}
