//! Confined native filesystem adapter for portable workspace discovery.

mod aliases;
mod mounts;
mod traversal;

#[cfg(test)]
mod tests;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use cap_std::{ambient_authority, fs::Dir};
use morphir_common::config::MorphirConfig;
use morphir_workspace::{
    DiscoveryRequest, RelativePath, WORKSPACE_DISCOVERY_PROTOCOL, WorkspaceSnapshot,
    discover_with_details,
};
use same_file::Handle;

use super::{
    discovery::{native_global_config_candidates, native_system_config_candidates},
    sources::ConfigLoadOptions,
};
use mounts::{apply_user_override_selection, selected_environment, selected_mount};
use traversal::build_tree_from_capability;

/// Discover a workspace by adapting a confined native directory to the
/// provider-neutral workspace protocol.
///
/// The granted `root` is canonicalized once and becomes the confinement
/// boundary. Native directories are traversed first, while confined directory
/// symlinks are recorded as aliases. Recognized entries are then copied from
/// the already-built real subtree to each alias without another filesystem
/// traversal. This keeps real paths visible, supports alias-only member globs,
/// and supports nested aliases. Alias edges already present in an expansion's
/// ancestry are skipped, so cycles cannot synthesize deeper paths indefinitely.
/// Fixed budgets bound alias edges, queued and processed expansions, generated
/// entries, and indexing/materialization work. Budget exhaustion returns the
/// stable `workspace.alias.resource-limit` code.
///
/// ```no_run
/// use morphir_devkit::{ConfigLoadOptions, discover_workspace};
/// use std::path::Path;
///
/// # fn main() -> anyhow::Result<()> {
/// let snapshot = discover_workspace(Path::new("."), &ConfigLoadOptions::default())?;
/// for project in snapshot.projects {
///     println!("{}: {}", project.relative_path.as_str(), project.name);
/// }
/// # Ok(())
/// # }
/// ```
pub fn discover_workspace(root: &Path, options: &ConfigLoadOptions) -> Result<WorkspaceSnapshot> {
    let request = build_workspace_discovery_request(root, options)?;
    morphir_workspace::discover(request)
        .into_result()
        .map_err(discovery_failure)
}

/// Native discovery output including decoded effective configurations from the
/// exact portable discovery pass and the canonical root bound by the adapter.
#[derive(Debug)]
pub struct NativeWorkspaceDiscovery {
    /// Canonical development root retained as the native confinement boundary.
    pub canonical_root: PathBuf,
    /// Provider-neutral discovery snapshot.
    pub snapshot: WorkspaceSnapshot,
    /// Fully merged root configuration.
    pub root_config: MorphirConfig,
    /// Fully merged configurations for valid projects, keyed by relative path.
    pub project_configs: BTreeMap<RelativePath, MorphirConfig>,
}

/// Discover a workspace and decode the exact effective configurations produced
/// by the portable engine without re-reading or re-merging any files.
pub fn discover_workspace_detailed(
    root: &Path,
    options: &ConfigLoadOptions,
) -> Result<NativeWorkspaceDiscovery> {
    let (canonical_root, request) = bind_workspace_discovery_request(root, options)?;
    let details = discover_with_details(request).map_err(discovery_failure)?;
    let root_config = decode_effective_config(details.root_effective)
        .context("Failed to decode effective root Morphir configuration")?;
    let project_configs = details
        .project_effective
        .into_iter()
        .map(|(path, value)| {
            if path == RelativePath::root() {
                Ok((path, root_config.clone()))
            } else {
                decode_effective_config(value)
                    .with_context(|| {
                        format!(
                            "Failed to decode effective Morphir configuration for `{}`",
                            path.as_str()
                        )
                    })
                    .map(|config| (path, config))
            }
        })
        .collect::<Result<_>>()?;
    Ok(NativeWorkspaceDiscovery {
        canonical_root,
        snapshot: details.snapshot,
        root_config,
        project_configs,
    })
}

fn decode_effective_config(mut value: serde_json::Value) -> Result<MorphirConfig> {
    if let Some(project) = value
        .get_mut("project")
        .and_then(serde_json::Value::as_object_mut)
    {
        project
            .entry("version")
            .or_insert_with(|| serde_json::Value::String(String::new()));
    }
    serde_json::from_value(value).map_err(Into::into)
}

/// Build the provider-neutral request used by [`discover_workspace`].
///
/// This lower-level API is useful to native hosts that need to serialize or
/// compare the exact request before running the pure discovery engine.
pub fn build_workspace_discovery_request(
    root: &Path,
    options: &ConfigLoadOptions,
) -> Result<DiscoveryRequest> {
    bind_workspace_discovery_request(root, options).map(|(_, request)| request)
}

fn bind_workspace_discovery_request(
    root: &Path,
    options: &ConfigLoadOptions,
) -> Result<(PathBuf, DiscoveryRequest)> {
    bind_workspace_discovery_request_with_hook(root, options, &mut |_| {})
}

fn bind_workspace_discovery_request_with_hook(
    root: &Path,
    options: &ConfigLoadOptions,
    root_opened_hook: &mut dyn FnMut(&Path),
) -> Result<(PathBuf, DiscoveryRequest)> {
    let granted_root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .context("Failed to resolve relative development root")?
            .join(root)
    };
    let root_capability =
        Dir::open_ambient_dir(&granted_root, ambient_authority()).with_context(|| {
            format!(
                "Failed to open development root: {}",
                granted_root.display()
            )
        })?;
    root_opened_hook(&granted_root);
    let canonical_root = fs::canonicalize(&granted_root).map_err(|error| {
        anyhow!(
            "workspace.path.not-confined: development root changed after binding `{}`: {error}",
            granted_root.display()
        )
    })?;
    let bound_identity = Handle::from_file(
        root_capability
            .try_clone()
            .context("Failed to clone bound development root capability")?
            .into_std_file(),
    )
    .context("Failed to inspect bound development root identity")?;
    let canonical_identity = Handle::from_path(&canonical_root).map_err(|error| {
        anyhow!(
            "workspace.path.not-confined: development root changed after binding `{}` while verifying `{}`: {error}",
            granted_root.display(),
            canonical_root.display()
        )
    })?;
    if bound_identity != canonical_identity {
        bail!(
            "workspace.path.not-confined: development root changed after binding `{}`; canonical path now identifies `{}`",
            granted_root.display(),
            canonical_root.display()
        );
    }
    let mut development_root =
        build_tree_from_capability(&root_capability, &canonical_root, &granted_root)?;
    apply_user_override_selection(&mut development_root, &options.user_override)?;

    let request = DiscoveryRequest {
        protocol_version: WORKSPACE_DISCOVERY_PROTOCOL,
        development_root,
        morphir_home: selected_mount(
            &options.global,
            native_global_config_candidates,
            "global user",
        )?,
        system_config: selected_mount(
            &options.system,
            || native_system_config_candidates().to_vec(),
            "system",
        )?,
        environment: selected_environment(options),
        cli_overlay: serde_json::json!({}),
    };
    Ok((canonical_root, request))
}

fn discovery_failure(failure: morphir_workspace::DiscoveryFailure) -> anyhow::Error {
    let path = failure
        .path
        .as_ref()
        .map(|path| format!(" at `{}`", path.as_str()))
        .unwrap_or_default();
    anyhow!("{}: {}{path}", failure.code, failure.message)
}
