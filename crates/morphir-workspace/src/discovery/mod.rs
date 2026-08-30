//! Canonical, filesystem-free workspace discovery.

mod decoding;
mod diagnostics;
mod layers;
mod members;
mod patterns;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use morphir_config::{builtin_defaults, env_config_value, merge_all};
use serde_json::{Map, Value};

use crate::{
    DiscoveryFailure, DiscoveryRequest, DiscoveryResponse, FileEntry, FileTree, ProjectState,
    RelativePath, WORKSPACE_DISCOVERY_PROTOCOL, WORKSPACE_PROTOCOL_UNSUPPORTED,
    WORKSPACE_SYMLINK_UNSUPPORTED, WorkspaceDiscoveryDetails, WorkspaceSnapshot, WorkspaceState,
};
use decoding::{decode_root_project, decode_workspace};
use diagnostics::{duplicate_name_diagnostics, failure, sort_diagnostics};
use layers::{
    optional_mount_layer, optional_user_layer, required_layer, without_project_or_workspace,
};
use members::discover_member;
use patterns::member_directories;

/// Discovers a Morphir workspace from portable, root-confined inputs.
#[must_use]
pub fn discover(request: DiscoveryRequest) -> DiscoveryResponse {
    match discover_internal(request, None) {
        Ok(snapshot) => DiscoveryResponse::Success { snapshot },
        Err(error) => DiscoveryResponse::Failure { error },
    }
}

/// Discovers a workspace and returns the exact merged configuration values
/// produced by the same pass as [`discover`].
pub fn discover_with_details(
    request: DiscoveryRequest,
) -> Result<WorkspaceDiscoveryDetails, DiscoveryFailure> {
    let mut collector = DetailsCollector::default();
    let snapshot = discover_internal(request, Some(&mut collector))?;
    Ok(collector.finish(snapshot))
}

pub(super) trait EffectiveConfigCollector {
    fn root(&mut self, effective: &Value);
    fn project(&mut self, path: &RelativePath, effective: &Value);
}

#[derive(Default)]
struct DetailsCollector {
    root_effective: Option<Value>,
    root_is_project: bool,
    project_effective: BTreeMap<RelativePath, Value>,
}

impl EffectiveConfigCollector for DetailsCollector {
    fn root(&mut self, effective: &Value) {
        self.root_effective = Some(effective.clone());
    }

    fn project(&mut self, path: &RelativePath, effective: &Value) {
        if path == &RelativePath::root() {
            self.root_is_project = true;
        } else {
            self.project_effective
                .insert(path.clone(), effective.clone());
        }
    }
}

impl DetailsCollector {
    fn finish(mut self, snapshot: WorkspaceSnapshot) -> WorkspaceDiscoveryDetails {
        let root_effective = self
            .root_effective
            .expect("successful detailed discovery collects the root config");
        if self.root_is_project {
            self.project_effective
                .insert(RelativePath::root(), root_effective.clone());
        }
        WorkspaceDiscoveryDetails {
            snapshot,
            root_effective,
            project_effective: self.project_effective,
        }
    }
}

fn discover_internal(
    request: DiscoveryRequest,
    mut collector: Option<&mut dyn EffectiveConfigCollector>,
) -> Result<WorkspaceSnapshot, DiscoveryFailure> {
    if request.protocol_version != WORKSPACE_DISCOVERY_PROTOCOL {
        return Err(failure(
            WORKSPACE_PROTOCOL_UNSUPPORTED,
            format!(
                "unsupported workspace discovery protocol {}; supported version is {}",
                request.protocol_version, WORKSPACE_DISCOVERY_PROTOCOL
            ),
            None,
        ));
    }
    reject_unmaterialized_symlinks(&request.development_root, "development root")?;
    if let Some(tree) = request.morphir_home.as_ref() {
        reject_unmaterialized_symlinks(tree, "Morphir Home")?;
    }
    if let Some(tree) = request.system_config.as_ref() {
        reject_unmaterialized_symlinks(tree, "system configuration")?;
    }

    let root = RelativePath::root();
    let workspace_primary = required_layer(&request.development_root, &root, "workspace root")?;
    let workspace_user = optional_user_layer(&request.development_root, &workspace_primary.path)?;
    let system = optional_mount_layer(request.system_config.as_ref(), "system configuration")?;
    let global = optional_mount_layer(request.morphir_home.as_ref(), "Morphir Home")?;
    let empty = Value::Object(Map::new());
    let system_value = system
        .as_ref()
        .map(|layer| without_project_or_workspace(&layer.value));
    let global_value = global
        .as_ref()
        .map(|layer| without_project_or_workspace(&layer.value));
    let shared_workspace_user = workspace_user
        .as_ref()
        .map(|layer| without_project_or_workspace(&layer.value));
    let environment = env_config_value(
        "MORPHIR",
        request
            .environment
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    let workspace_effective = merge_all([
        &builtin_defaults(),
        system_value.as_ref().unwrap_or(&empty),
        global_value.as_ref().unwrap_or(&empty),
        &workspace_primary.value,
        workspace_user
            .as_ref()
            .map(|layer| &layer.value)
            .unwrap_or(&empty),
        &environment,
        &request.cli_overlay,
    ]);
    let workspace = decode_workspace(&workspace_effective, &workspace_primary.path)?;
    let root_has_project = workspace_effective.get("project").is_some();

    let member_directories = member_directories(
        &request.development_root,
        &workspace_primary.path,
        &workspace.members,
        &workspace.exclude,
    )?;
    let mut projects = Vec::new();
    if let Some(collector) = collector.as_deref_mut() {
        collector.root(&workspace_effective);
    }
    if root_has_project {
        projects.push(decode_root_project(
            &workspace_effective,
            &workspace_primary.path,
        )?);
        if let Some(collector) = collector.as_deref_mut() {
            collector.project(&root, &workspace_effective);
        }
    }

    let shared_workspace = without_project_or_workspace(&workspace_primary.value);
    for directory in member_directories {
        if directory == root {
            continue;
        }
        if let Some(project) = discover_member(
            &request.development_root,
            &directory,
            system_value.as_ref().unwrap_or(&empty),
            global_value.as_ref().unwrap_or(&empty),
            &shared_workspace,
            shared_workspace_user.as_ref().unwrap_or(&empty),
            &environment,
            &request.cli_overlay,
            &mut collector,
        ) {
            projects.push(project);
        }
    }

    projects.sort_by(|left, right| {
        left.relative_path
            .cmp(&right.relative_path)
            .then_with(|| left.name.cmp(&right.name))
    });
    for project in &mut projects {
        sort_diagnostics(&mut project.diagnostics);
    }
    let mut diagnostics = duplicate_name_diagnostics(&projects);
    sort_diagnostics(&mut diagnostics);
    let state = if projects
        .iter()
        .any(|project| project.state == ProjectState::Error)
    {
        WorkspaceState::Error
    } else {
        WorkspaceState::Open
    };

    Ok(WorkspaceSnapshot {
        protocol_version: WORKSPACE_DISCOVERY_PROTOCOL,
        config_anchor: workspace_primary.path,
        name: workspace.name,
        state,
        projects,
        diagnostics,
    })
}

fn reject_unmaterialized_symlinks(tree: &FileTree, mount: &str) -> Result<(), DiscoveryFailure> {
    if let Some((path, FileEntry::Symlink { target })) = tree
        .entries
        .iter()
        .find(|(_, entry)| matches!(entry, FileEntry::Symlink { .. }))
    {
        return Err(failure(
            WORKSPACE_SYMLINK_UNSUPPORTED,
            format!(
                "unmaterialized symlink `{}` to `{}` in {mount}; protocol-v1 hosts must materialize confined symlink targets before discovery",
                path.as_str(),
                target.as_str()
            ),
            Some(path.clone()),
        ));
    }
    Ok(())
}
