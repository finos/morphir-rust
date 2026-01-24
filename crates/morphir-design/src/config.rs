use anyhow::{Context, Result};
use morphir_common::config::model::{MorphirConfig, ProjectSection, WorkspaceSection};
use std::path::{Path, PathBuf};

/// Configuration context containing loaded config and resolved paths
#[derive(Debug, Clone)]
pub struct ConfigContext {
    /// Loaded configuration (merged workspace + project)
    pub config: MorphirConfig,
    /// Path to the config file
    pub config_path: PathBuf,
    /// Path to `.morphir/` directory (canonical folder)
    pub morphir_dir: PathBuf,
    /// Workspace root if in workspace
    pub workspace_root: Option<PathBuf>,
    /// Project root if in project
    pub project_root: Option<PathBuf>,
    /// Current project if in workspace
    pub current_project: Option<ProjectSection>,
}

/// Walk up directory tree to find morphir.toml or morphir.json
pub fn discover_config(start_dir: &Path) -> Option<PathBuf> {
    let mut current = start_dir.to_path_buf();

    loop {
        // Check for morphir.toml
        let toml_path = current.join("morphir.toml");
        if toml_path.exists() {
            return Some(toml_path);
        }

        // Check for morphir.json
        let json_path = current.join("morphir.json");
        if json_path.exists() {
            return Some(json_path);
        }

        // Move up one directory
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }

    None
}

/// Walk up directory tree to find `.morphir/` directory
pub fn discover_morphir_dir(start_dir: &Path) -> Option<PathBuf> {
    let mut current = start_dir.to_path_buf();

    loop {
        let morphir_path = current.join(".morphir");
        if morphir_path.exists() && morphir_path.is_dir() {
            return Some(morphir_path);
        }

        // Move up one directory
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }

    None
}

/// Merge workspace and project configurations
fn merge_configs(workspace: Option<&MorphirConfig>, project: &MorphirConfig) -> MorphirConfig {
    let mut merged = if let Some(ws) = workspace {
        ws.clone()
    } else {
        MorphirConfig::default()
    };

    // Project config overrides workspace
    if let Some(proj) = &project.project {
        merged.project = Some(proj.clone());
    }

    // Merge frontend config
    if let Some(frontend) = &project.frontend {
        merged.frontend = Some(frontend.clone());
    }

    // Merge codegen config
    if let Some(codegen) = &project.codegen {
        merged.codegen = Some(codegen.clone());
    }

    // Merge IR config
    if let Some(ir) = &project.ir {
        merged.ir = Some(ir.clone());
    }

    // Merge extensions (project extensions override workspace)
    for (key, value) in &project.extensions {
        merged.extensions.insert(key.clone(), value.clone());
    }

    merged
}

/// Load configuration and determine workspace/project context
pub fn load_config_context(config_path: &Path) -> Result<ConfigContext> {
    // Load config file
    let config_content = std::fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read config file: {:?}", config_path))?;

    let config: MorphirConfig = if config_path.extension().and_then(|s| s.to_str()) == Some("json")
    {
        serde_json::from_str(&config_content)
            .with_context(|| format!("Failed to parse JSON config: {:?}", config_path))?
    } else {
        toml::from_str(&config_content)
            .with_context(|| format!("Failed to parse TOML config: {:?}", config_path))?
    };

    let config_dir = config_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Config file has no parent directory"))?;

    // Check if this is a workspace config
    let workspace_root = if config.is_workspace() {
        Some(config_dir.to_path_buf())
    } else {
        None
    };

    // If in workspace, try to find project configs
    let (project_root, current_project, merged_config) = if let Some(ws_root) = &workspace_root {
        if let Some(ws) = &config.workspace {
            // Try to find default member or first member
            let default_member = ws.default_member.as_ref().or_else(|| ws.members.first());

            if let Some(member) = default_member {
                // Resolve member path (could be a glob pattern, for now treat as literal)
                let member_path = ws_root.join(member);
                let project_config_path = member_path.join("morphir.toml");

                if project_config_path.exists() {
                    // Load project config
                    let project_content = std::fs::read_to_string(&project_config_path)
                        .with_context(|| {
                            format!("Failed to read project config: {:?}", project_config_path)
                        })?;

                    let project_config: MorphirConfig = toml::from_str(&project_content)
                        .with_context(|| {
                            format!("Failed to parse project config: {:?}", project_config_path)
                        })?;

                    // Merge workspace and project configs
                    let merged = merge_configs(Some(&config), &project_config);

                    (Some(member_path), merged.project.clone(), merged)
                } else {
                    (None, config.project.clone(), config)
                }
            } else {
                (None, config.project.clone(), config)
            }
        } else {
            (None, config.project.clone(), config)
        }
    } else {
        // Not in workspace, use config as-is
        (
            Some(config_dir.to_path_buf()),
            config.project.clone(),
            config,
        )
    };

    // Find or create .morphir/ directory
    let morphir_dir = discover_morphir_dir(config_dir).unwrap_or_else(|| {
        // Use project root if available, otherwise config dir
        project_root
            .as_ref()
            .map_or(config_dir, |v| v.as_path())
            .join(".morphir")
    });

    Ok(ConfigContext {
        config: merged_config,
        config_path: config_path.to_path_buf(),
        morphir_dir,
        workspace_root,
        project_root,
        current_project,
    })
}

/// Resolve compile output path using Mill-inspired structure
pub fn resolve_compile_output(project: &str, language: &str, morphir_dir: &Path) -> PathBuf {
    morphir_dir
        .join("out")
        .join(sanitize_project_name(project))
        .join("compile")
        .join(language)
}

/// Resolve generate output path using Mill-inspired structure
pub fn resolve_generate_output(project: &str, target: &str, morphir_dir: &Path) -> PathBuf {
    morphir_dir
        .join("out")
        .join(sanitize_project_name(project))
        .join("generate")
        .join(target)
}

/// Resolve distribution output path
pub fn resolve_dist_output(project: &str, morphir_dir: &Path) -> PathBuf {
    morphir_dir
        .join("out")
        .join(sanitize_project_name(project))
        .join("dist")
}

/// Resolve test fixture path
pub fn resolve_test_fixture(name: &str, morphir_dir: &Path) -> PathBuf {
    morphir_dir.join("test").join("fixtures").join(name)
}

/// Resolve test scenario path
pub fn resolve_test_scenario(name: &str, morphir_dir: &Path) -> PathBuf {
    morphir_dir.join("test").join("scenarios").join(name)
}

/// Sanitize project name for filesystem use
pub fn sanitize_project_name(name: &str) -> String {
    // Replace invalid characters, but preserve structure
    // For now, just replace slashes and spaces
    name.replace('/', "-").replace(' ', "-").replace('\\', "-")
}

/// Resolve path relative to config file location
pub fn resolve_path_relative_to_config(path: &Path, config_path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        config_path.parent().unwrap_or(Path::new(".")).join(path)
    }
}

/// Resolve path relative to workspace root
pub fn resolve_path_relative_to_workspace(path: &Path, workspace_root: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}

/// Ensure .morphir/ folder structure is created
pub fn ensure_morphir_structure(morphir_dir: &Path) -> Result<()> {
    // Create base directories
    std::fs::create_dir_all(morphir_dir.join("out"))?;
    std::fs::create_dir_all(morphir_dir.join("test").join("fixtures"))?;
    std::fs::create_dir_all(morphir_dir.join("test").join("scenarios"))?;
    std::fs::create_dir_all(morphir_dir.join("logs"))?;
    std::fs::create_dir_all(morphir_dir.join("cache"))?;
    Ok(())
}
