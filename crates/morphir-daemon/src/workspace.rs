//! Workspace management for multi-project Morphir development

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::Result;
use morphir_common::config::MorphirConfig;
use morphir_devkit::{ConfigLoadOptions, NativeWorkspaceDiscovery, discover_workspace_detailed};

/// Workspace state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceState {
    /// Workspace is not active
    Closed,
    /// Workspace is being loaded
    Initializing,
    /// Workspace is ready for operations
    Open,
    /// Workspace has unrecoverable errors
    Error,
}

/// Project state within a workspace
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectState {
    /// Project metadata loaded, IR not compiled
    Unloaded,
    /// Project is being compiled
    Loading,
    /// Project IR is loaded and valid
    Ready,
    /// Source files changed, needs recompilation
    Stale,
    /// Project has compilation errors
    Error,
}

/// A project within a workspace
#[derive(Debug, Clone)]
pub struct Project {
    /// Project name (org/name format)
    pub name: String,
    /// Project version
    pub version: String,
    /// Native project path under the workspace root
    ///
    /// This remains an absolute/native path for compatibility. Workspace
    /// storage and [`Workspace::get_project_at`] use the provider-neutral
    /// relative path identity.
    pub path: PathBuf,
    /// Current state
    pub state: ProjectState,
    /// Source directory
    pub source_dir: String,
    /// Project configuration
    pub config: MorphirConfig,
}

/// A Morphir workspace managing multiple projects
#[derive(Debug)]
pub struct Workspace {
    /// Workspace root directory
    pub root: PathBuf,
    /// Workspace name
    pub name: Option<String>,
    /// Current state
    pub state: WorkspaceState,
    /// Workspace configuration
    pub config: MorphirConfig,
    /// Projects in the workspace
    pub projects: BTreeMap<PathBuf, Project>,
}

impl Workspace {
    /// Create a new workspace at the given root
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            name: None,
            state: WorkspaceState::Closed,
            config: MorphirConfig::default(),
            projects: BTreeMap::new(),
        }
    }

    /// Open an existing workspace
    pub fn open(root: PathBuf) -> Result<Self> {
        let discovery = discover_workspace_detailed(&root, &ConfigLoadOptions::default())?;
        Ok(Self::from_discovery(discovery))
    }

    fn from_discovery(discovery: NativeWorkspaceDiscovery) -> Self {
        let NativeWorkspaceDiscovery {
            canonical_root: root,
            snapshot,
            root_config: config,
            mut project_configs,
        } = discovery;
        let projects = snapshot
            .projects
            .iter()
            .map(|project| {
                let relative_path = PathBuf::from(project.relative_path.as_str());
                let native_path = if project.relative_path.as_str() == "." {
                    root.clone()
                } else {
                    root.join(&relative_path)
                };
                (
                    relative_path,
                    Project {
                        name: project.name.clone(),
                        version: project.version.clone().unwrap_or_default(),
                        path: native_path,
                        state: map_project_state(project.state),
                        source_dir: project.source_directory.as_str().to_owned(),
                        config: if project.state == morphir_workspace::ProjectState::Error {
                            MorphirConfig::default()
                        } else {
                            project_configs
                                .remove(&project.relative_path)
                                .expect("portable details include every valid project config")
                        },
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let name = snapshot.name.or_else(|| {
            projects
                .get(Path::new("."))
                .map(|project| project.name.clone())
        });

        Self {
            root,
            name,
            state: map_workspace_state(snapshot.state),
            config,
            projects,
        }
    }

    /// Get a project by name when that name identifies exactly one project.
    ///
    /// Duplicate project names are intentionally ambiguous and return `None`;
    /// use [`Self::get_project_at`] to select them by relative path.
    pub fn get_project(&self, name: &str) -> Option<&Project> {
        let mut matches = self
            .projects
            .values()
            .filter(|project| project.name == name);
        let project = matches.next()?;
        matches.next().is_none().then_some(project)
    }

    /// Get a mutable project by name when that name is unambiguous.
    pub fn get_project_mut(&mut self, name: &str) -> Option<&mut Project> {
        let mut keys = self
            .projects
            .iter()
            .filter(|(_, project)| project.name == name)
            .map(|(path, _)| path.clone());
        let key = keys.next()?;
        if keys.next().is_some() {
            return None;
        }
        self.projects.get_mut(&key)
    }

    /// Get a project by its path relative to the workspace root.
    pub fn get_project_at(&self, relative_path: &Path) -> Option<&Project> {
        let key = if relative_path.as_os_str().is_empty() {
            Path::new(".")
        } else {
            relative_path
        };
        self.projects.get(key)
    }

    /// Iterate over every project in deterministic relative-path order.
    pub fn projects(&self) -> impl Iterator<Item = &Project> {
        self.projects.values()
    }

    /// List all project names in deterministic relative-path order.
    ///
    /// Duplicate names remain present in the returned list.
    pub fn list_projects(&self) -> Vec<&str> {
        self.projects
            .values()
            .map(|project| project.name.as_str())
            .collect()
    }

    /// Close the workspace
    pub fn close(&mut self) {
        self.state = WorkspaceState::Closed;
        self.projects.clear();
    }
}

fn map_workspace_state(state: morphir_workspace::WorkspaceState) -> WorkspaceState {
    match state {
        morphir_workspace::WorkspaceState::Open => WorkspaceState::Open,
        morphir_workspace::WorkspaceState::Error => WorkspaceState::Error,
    }
}

fn map_project_state(state: morphir_workspace::ProjectState) -> ProjectState {
    match state {
        morphir_workspace::ProjectState::Unloaded => ProjectState::Unloaded,
        morphir_workspace::ProjectState::Error => ProjectState::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_a_yaml_project() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("morphir.yaml"),
            "project:\n  name: acme/orders\n  version: 1.0.0\n",
        )
        .unwrap();

        let workspace = Workspace::open(root.path().to_path_buf()).unwrap();

        assert_eq!(workspace.state, WorkspaceState::Open);
        assert_eq!(workspace.name.as_deref(), Some("acme/orders"));
        assert!(workspace.get_project("acme/orders").is_some());
    }

    #[test]
    fn prefers_a_named_workspace_over_its_root_project_name() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("morphir.yaml"),
            "workspace:\n  name: order-domain\nproject:\n  name: acme/orders\n",
        )
        .unwrap();

        let workspace = Workspace::open(root.path().to_path_buf()).unwrap();

        assert_eq!(workspace.name.as_deref(), Some("order-domain"));
    }

    #[test]
    fn discovers_yaml_workspace_members() {
        let root = tempfile::tempdir().unwrap();
        let member = root.path().join("packages").join("orders");
        std::fs::create_dir_all(&member).unwrap();
        std::fs::write(
            root.path().join("morphir.yaml"),
            "workspace:\n  members:\n    - packages/*\n",
        )
        .unwrap();
        std::fs::write(
            member.join("morphir.yaml"),
            "project:\n  name: acme/orders\n  version: 1.0.0\n",
        )
        .unwrap();

        let workspace = Workspace::open(root.path().to_path_buf()).unwrap();

        assert!(workspace.get_project("acme/orders").is_some());
    }

    fn workspace_fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/workspace-discovery/valid-monorepo")
    }

    #[test]
    fn keeps_duplicate_names_addressable_by_relative_path() {
        let workspace = Workspace::open(workspace_fixture_root()).unwrap();

        assert!(workspace.get_project("acme/risk").is_none());
        assert_eq!(
            workspace
                .get_project_at(Path::new("packages/risk"))
                .unwrap()
                .version,
            "2.0.0"
        );
        assert_eq!(
            workspace
                .get_project_at(Path::new("packages/duplicate"))
                .unwrap()
                .version,
            "2.1.0"
        );
    }

    #[test]
    fn project_iteration_is_complete_and_deterministic() {
        let mut workspace = Workspace::open(workspace_fixture_root()).unwrap();

        assert_eq!(workspace.state, WorkspaceState::Error);
        assert_eq!(
            workspace
                .get_project_at(Path::new("packages/broken"))
                .unwrap()
                .state,
            ProjectState::Error
        );
        assert!(
            workspace
                .get_project_at(Path::new("packages/broken"))
                .unwrap()
                .config
                .project
                .is_none()
        );
        assert_eq!(
            workspace
                .projects()
                .map(|project| {
                    let relative = project.path.strip_prefix(&workspace.root).unwrap();
                    if relative.as_os_str().is_empty() {
                        Path::new(".")
                    } else {
                        relative
                    }
                })
                .collect::<Vec<_>>(),
            [
                Path::new("."),
                Path::new("packages/broken"),
                Path::new("packages/duplicate"),
                Path::new("packages/orders"),
                Path::new("packages/risk"),
            ]
        );
        assert_eq!(
            workspace.get_project("acme/orders").unwrap().name,
            "acme/orders"
        );
        assert_eq!(
            workspace.list_projects().into_iter().collect::<Vec<_>>(),
            [
                "acme/root",
                "packages/broken",
                "acme/risk",
                "acme/orders",
                "acme/risk",
            ]
        );
        let orders = workspace.get_project("acme/orders").unwrap();
        let projected = orders.config.project.as_ref().unwrap();
        assert_eq!(projected.name, "acme/orders");
        assert_eq!(projected.version, "1.2.0");
        assert_eq!(projected.source_directory, "elm");
        assert!(workspace.get_project_mut("acme/risk").is_none());
    }

    #[test]
    fn preserves_full_effective_root_and_member_configs() {
        let root = tempfile::tempdir().unwrap();
        let member = root.path().join("packages/orders");
        std::fs::create_dir_all(&member).unwrap();
        std::fs::write(
            root.path().join("morphir.toml"),
            r#"
[workspace]
members = ["packages/*"]
out_dir = "root-output"

[project]
name = "acme/root"
version = "1.0.0"
description = "root description"
authors = ["Root Author"]
source_directory = "root-src"

[frontend]
language = "elm"
dialect = "2024"

[ir]
format_version = 3
strict_mode = true

[codegen]
targets = ["scala"]
output_format = "root-format"

[dependencies]
shared = "^1.0"

[tasks.build]
description = "Build everything"
run = "morphir make"
"#,
        )
        .unwrap();
        std::fs::write(
            member.join("morphir.toml"),
            r#"
[project]
name = "acme/orders"
version = "2.0.0"
description = "orders description"
source_directory = "elm"

[codegen]
output_format = "member-format"
"#,
        )
        .unwrap();

        let workspace = Workspace::open(root.path().to_path_buf()).unwrap();
        let root_project = workspace.config.project.as_ref().unwrap();
        assert_eq!(
            root_project.description.as_deref(),
            Some("root description")
        );
        assert_eq!(
            workspace.config.workspace.as_ref().unwrap().out_dir,
            "root-output"
        );
        assert_eq!(
            workspace
                .config
                .frontend
                .as_ref()
                .unwrap()
                .language
                .as_deref(),
            Some("elm")
        );
        assert!(workspace.config.ir.as_ref().unwrap().strict_mode);
        assert_eq!(
            workspace.config.codegen.as_ref().unwrap().output_format,
            "root-format"
        );
        assert!(workspace.config.dependencies.contains_key("shared"));
        assert!(workspace.config.tasks.contains_key("build"));

        let orders = workspace.get_project("acme/orders").unwrap();
        let project = orders.config.project.as_ref().unwrap();
        assert_eq!(project.description.as_deref(), Some("orders description"));
        assert_eq!(
            orders.config.codegen.as_ref().unwrap().output_format,
            "member-format"
        );
        assert!(orders.config.dependencies.contains_key("shared"));
        assert!(orders.config.tasks.contains_key("build"));
    }

    #[test]
    fn preserves_an_explicitly_empty_workspace_section() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("morphir.toml"),
            "[workspace]\nmembers = []\nout_dir = 'empty-output'\n",
        )
        .unwrap();

        let workspace = Workspace::open(root.path().to_path_buf()).unwrap();

        assert!(workspace.projects.is_empty());
        let config = workspace.config.workspace.as_ref().unwrap();
        assert!(config.members.is_empty());
        assert_eq!(config.out_dir, "empty-output");
    }

    #[test]
    fn relative_root_is_stored_canonically_and_projects_are_absolute() {
        let current = std::env::current_dir().unwrap();
        let root = tempfile::Builder::new()
            .prefix("morphir-daemon-relative-")
            .tempdir_in(&current)
            .unwrap();
        std::fs::write(
            root.path().join("morphir.toml"),
            "[project]\nname = 'acme/relative'\nversion = '1.0.0'\n",
        )
        .unwrap();
        let relative = root.path().strip_prefix(&current).unwrap().to_path_buf();

        let workspace = Workspace::open(relative).unwrap();

        let canonical = std::fs::canonicalize(root.path()).unwrap();
        assert_eq!(workspace.root, canonical);
        assert_eq!(
            workspace.get_project("acme/relative").unwrap().path,
            canonical
        );
    }
}
