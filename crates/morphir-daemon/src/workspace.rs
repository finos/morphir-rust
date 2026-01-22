#![allow(clippy::ptr_arg)]
//! Workspace management for multi-project Morphir development

use std::collections::HashMap;
use std::path::PathBuf;

use crate::Result;
use morphir_common::config::MorphirConfig;

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
    /// Path relative to workspace root
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
    pub projects: HashMap<String, Project>,
}

impl Workspace {
    /// Create a new workspace at the given root
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            name: None,
            state: WorkspaceState::Closed,
            config: MorphirConfig::default(),
            projects: HashMap::new(),
        }
    }

    /// Open an existing workspace
    pub fn open(root: PathBuf) -> Result<Self> {
        let config_path = root.join("morphir.toml");
        let config = MorphirConfig::load(&config_path)?;

        let mut workspace = Self {
            root,
            name: config.project.as_ref().map(|p| p.name.clone()),
            state: WorkspaceState::Initializing,
            config,
            projects: HashMap::new(),
        };

        workspace.discover_projects()?;
        workspace.state = WorkspaceState::Open;

        Ok(workspace)
    }

    /// Discover projects in the workspace based on member patterns
    fn discover_projects(&mut self) -> Result<()> {
        let patterns: Vec<String> = self
            .config
            .workspace
            .as_ref()
            .map(|ws| ws.members.clone())
            .unwrap_or_default();

        if !patterns.is_empty() {
            for pattern in patterns {
                self.discover_projects_matching(&pattern)?;
            }
        } else if self.config.project.is_some() {
            // Single project mode - the root is the project
            let root = self.root.clone();
            self.load_project(&root)?;
        }
        Ok(())
    }

    /// Discover projects matching a glob pattern
    fn discover_projects_matching(&mut self, pattern: &str) -> Result<()> {
        let base = self.root.join(pattern);
        let pattern_str = base.to_string_lossy();

        // Simple glob handling - for now just handle "packages/*" style patterns
        if let Some(parent) = pattern_str.strip_suffix("/*") {
            let parent_path = PathBuf::from(parent);
            if parent_path.is_dir() {
                for entry in std::fs::read_dir(&parent_path)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.is_dir() && path.join("morphir.toml").exists() {
                        self.load_project(&path)?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Load a project from a directory
    fn load_project(&mut self, path: &PathBuf) -> Result<()> {
        let config_path = path.join("morphir.toml");
        let config = MorphirConfig::load(&config_path)?;

        if let Some(ref project_config) = config.project {
            let project = Project {
                name: project_config.name.clone(),
                version: project_config.version.clone(),
                path: path.clone(),
                state: ProjectState::Unloaded,
                source_dir: project_config.source_directory.clone(),
                config,
            };
            self.projects.insert(project.name.clone(), project);
        }

        Ok(())
    }

    /// Get a project by name
    pub fn get_project(&self, name: &str) -> Option<&Project> {
        self.projects.get(name)
    }

    /// Get a mutable project by name
    pub fn get_project_mut(&mut self, name: &str) -> Option<&mut Project> {
        self.projects.get_mut(name)
    }

    /// List all project names
    pub fn list_projects(&self) -> Vec<&str> {
        self.projects.keys().map(|s| s.as_str()).collect()
    }

    /// Close the workspace
    pub fn close(&mut self) {
        self.state = WorkspaceState::Closed;
        self.projects.clear();
    }
}
