use anyhow::{Context, Result, bail};
use morphir_common::config::load_config_value;
use morphir_common::config::model::{MorphirConfig, ProjectSection};
use serde_json::Value;
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

/// Operating-system path conventions used for global user configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigPlatform {
    /// Linux and other systems that follow the XDG Base Directory specification.
    Xdg,
    /// macOS application-support path conventions, with XDG override support.
    MacOs,
    /// Windows Known Folder path conventions.
    Windows,
}

fn project_config_candidates(directory: &Path) -> [PathBuf; 4] {
    [
        directory.join("morphir.toml"),
        directory.join("morphir.yaml"),
        directory.join(".morphir").join("morphir.toml"),
        directory.join(".morphir").join("morphir.yaml"),
    ]
}

/// Return the only existing candidate, or report all conflicting candidates.
pub fn discover_config_candidates(candidates: &[PathBuf]) -> Result<Option<PathBuf>> {
    let found = candidates
        .iter()
        .filter(|candidate| candidate.is_file())
        .fold(Vec::new(), |mut found, candidate| {
            if !found.contains(candidate) {
                found.push(candidate.clone());
            }
            found
        });

    match found.as_slice() {
        [] => Ok(None),
        [config] => Ok(Some(config.clone())),
        configs => {
            let paths = configs
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("Ambiguous Morphir configuration; found: {paths}")
        }
    }
}

fn discover_config_in_directory(directory: &Path) -> Result<Option<PathBuf>> {
    let modern = discover_config_candidates(&project_config_candidates(directory))?;
    Ok(modern.or_else(|| {
        let legacy = directory.join("morphir.json");
        legacy.is_file().then_some(legacy)
    }))
}

/// Walk up the directory tree to find one project or workspace configuration.
pub fn discover_config(start_dir: &Path) -> Result<Option<PathBuf>> {
    let mut current = start_dir.to_path_buf();

    loop {
        if let Some(config) = discover_config_in_directory(&current)? {
            return Ok(Some(config));
        }

        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }

    Ok(None)
}

/// Build all global user configuration candidates for resolved platform roots.
pub fn global_config_candidates(
    platform: ConfigPlatform,
    home_dir: Option<&Path>,
    platform_config_dir: Option<&Path>,
    xdg_config_home: Option<&Path>,
) -> Vec<PathBuf> {
    let valid_xdg = xdg_config_home.filter(|path| is_absolute_for(platform, path));
    let config_dir = match platform {
        ConfigPlatform::Xdg | ConfigPlatform::MacOs => valid_xdg.or(platform_config_dir),
        ConfigPlatform::Windows => platform_config_dir,
    };

    config_dir
        .into_iter()
        .map(|root| root.join("morphir"))
        .chain(home_dir.into_iter().map(|root| root.join(".morphir")))
        .flat_map(|root| [root.join("morphir.toml"), root.join("morphir.yaml")])
        .collect()
}

fn is_absolute_for(platform: ConfigPlatform, path: &Path) -> bool {
    let value = path.to_string_lossy();
    match platform {
        ConfigPlatform::Xdg | ConfigPlatform::MacOs => value.starts_with('/'),
        ConfigPlatform::Windows => {
            value.starts_with(r"\\")
                || (value.as_bytes().get(1) == Some(&b':')
                    && matches!(value.as_bytes().get(2), Some(b'\\' | b'/')))
        }
    }
}

fn current_platform() -> ConfigPlatform {
    #[cfg(target_os = "windows")]
    return ConfigPlatform::Windows;
    #[cfg(target_os = "macos")]
    return ConfigPlatform::MacOs;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    ConfigPlatform::Xdg
}

/// Discover the global user configuration using native platform directories.
pub fn discover_global_config() -> Result<Option<PathBuf>> {
    let platform = current_platform();
    let home_dir = dirs::home_dir();
    let platform_config_dir = match platform {
        ConfigPlatform::Xdg => home_dir.as_ref().map(|home| home.join(".config")),
        ConfigPlatform::MacOs | ConfigPlatform::Windows => dirs::config_dir(),
    };
    let xdg_config_home = match platform {
        ConfigPlatform::Xdg | ConfigPlatform::MacOs => {
            std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
        }
        ConfigPlatform::Windows => None,
    };
    let candidates = global_config_candidates(
        platform,
        home_dir.as_deref(),
        platform_config_dir.as_deref(),
        xdg_config_home.as_deref(),
    );
    discover_config_candidates(&candidates)
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

fn deep_merge(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(&key) {
                    Some(base_value) => deep_merge(base_value, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

/// Load configuration and determine workspace/project context
pub fn load_config_context(config_path: &Path) -> Result<ConfigContext> {
    let global_config_path = discover_global_config()?;
    load_config_context_with_global(config_path, global_config_path.as_deref())
}

/// Load configuration with an explicitly selected global source.
pub fn load_config_context_with_global(
    config_path: &Path,
    global_config_path: Option<&Path>,
) -> Result<ConfigContext> {
    let mut config_value = match global_config_path {
        Some(path) => load_config_value(path)?,
        None => Value::Object(Default::default()),
    };
    deep_merge(&mut config_value, load_config_value(config_path)?);
    let config: MorphirConfig = serde_json::from_value(config_value).with_context(|| {
        format!(
            "Failed to decode merged Morphir config: {}",
            config_path.display()
        )
    })?;

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
                let project_config_path = discover_config_in_directory(&member_path)?;

                if let Some(project_config_path) = project_config_path {
                    let mut merged_value = serde_json::to_value(&config)
                        .context("Failed to normalize workspace config")?;
                    deep_merge(&mut merged_value, load_config_value(&project_config_path)?);
                    let merged: MorphirConfig =
                        serde_json::from_value(merged_value).with_context(|| {
                            format!(
                                "Failed to decode project config: {}",
                                project_config_path.display()
                            )
                        })?;

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
    name.replace(['/', ' ', '\\'], "-")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write_project_config(path: &Path) {
        std::fs::create_dir_all(path.parent().expect("config parent")).unwrap();
        std::fs::write(path, "project:\n  name: Acme.Project\n  version: 1.0.0\n").unwrap();
    }

    #[test]
    fn discovers_yaml_while_walking_parent_directories() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("src").join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let expected = root.path().join("morphir.yaml");
        write_project_config(&expected);

        assert_eq!(discover_config(&nested).unwrap(), Some(expected));
    }

    #[test]
    fn discovers_hidden_project_config() {
        let root = tempfile::tempdir().unwrap();
        let expected = root.path().join(".morphir").join("morphir.yaml");
        write_project_config(&expected);

        assert_eq!(discover_config(root.path()).unwrap(), Some(expected));
    }

    #[test]
    fn rejects_ambiguous_project_configs() {
        let root = tempfile::tempdir().unwrap();
        let toml = root.path().join("morphir.toml");
        let yaml = root.path().join("morphir.yaml");
        std::fs::write(&toml, "[project]\nname = \"Acme.Project\"\nversion = \"1\"").unwrap();
        write_project_config(&yaml);

        let error = discover_config(root.path()).expect_err("ambiguous config");
        let message = error.to_string();
        assert!(message.contains(toml.to_str().unwrap()));
        assert!(message.contains(yaml.to_str().unwrap()));
    }

    #[test]
    fn does_not_implicitly_discover_yml() {
        let root = tempfile::tempdir().unwrap();
        write_project_config(&root.path().join("morphir.yml"));

        assert_eq!(discover_config(root.path()).unwrap(), None);
    }

    #[test]
    fn resolves_linux_xdg_and_home_candidates() {
        let candidates = global_config_candidates(
            ConfigPlatform::Xdg,
            Some(Path::new("/home/alice")),
            Some(Path::new("/ignored/platform")),
            Some(Path::new("/srv/alice/config")),
        );

        assert_eq!(
            candidates,
            vec![
                PathBuf::from("/srv/alice/config/morphir/morphir.toml"),
                PathBuf::from("/srv/alice/config/morphir/morphir.yaml"),
                PathBuf::from("/home/alice/.morphir/morphir.toml"),
                PathBuf::from("/home/alice/.morphir/morphir.yaml"),
            ]
        );
    }

    #[test]
    fn ignores_relative_xdg_config_home() {
        let candidates = global_config_candidates(
            ConfigPlatform::Xdg,
            Some(Path::new("/home/alice")),
            Some(Path::new("/home/alice/.config")),
            Some(Path::new("relative/config")),
        );

        assert_eq!(
            candidates[0],
            PathBuf::from("/home/alice/.config/morphir/morphir.toml")
        );
    }

    #[test]
    fn uses_macos_application_support_and_home_candidates() {
        let candidates = global_config_candidates(
            ConfigPlatform::MacOs,
            Some(Path::new("/Users/Alice")),
            Some(Path::new("/Users/Alice/Library/Application Support")),
            None,
        );

        assert_eq!(
            candidates[0],
            PathBuf::from("/Users/Alice/Library/Application Support/morphir/morphir.toml")
        );
        assert_eq!(
            candidates[2],
            PathBuf::from("/Users/Alice/.morphir/morphir.toml")
        );
    }

    #[test]
    fn uses_windows_known_folder_candidates() {
        let candidates = global_config_candidates(
            ConfigPlatform::Windows,
            Some(Path::new(r"D:\Profiles\Alice")),
            Some(Path::new(r"D:\Profiles\Alice\Roaming")),
            Some(Path::new(r"D:\ignored-xdg")),
        );

        assert_eq!(
            candidates[0],
            PathBuf::from(r"D:\Profiles\Alice\Roaming").join("morphir/morphir.toml")
        );
        assert_eq!(
            candidates[2],
            PathBuf::from(r"D:\Profiles\Alice").join(".morphir/morphir.toml")
        );
    }

    #[test]
    fn rejects_ambiguous_global_configs() {
        let root = tempfile::tempdir().unwrap();
        let config_dir = root.path().join("config");
        let home_dir = root.path().join("home");
        let candidates = global_config_candidates(
            ConfigPlatform::Xdg,
            Some(&home_dir),
            Some(&config_dir),
            None,
        );
        std::fs::create_dir_all(candidates[0].parent().unwrap()).unwrap();
        std::fs::create_dir_all(candidates[3].parent().unwrap()).unwrap();
        std::fs::write(&candidates[0], "[morphir]\nversion = \"1\"").unwrap();
        std::fs::write(&candidates[3], "morphir:\n  version: '1'\n").unwrap();

        let error = discover_config_candidates(&candidates).expect_err("ambiguous config");
        let message = error.to_string();
        assert!(message.contains(candidates[0].to_str().unwrap()));
        assert!(message.contains(candidates[3].to_str().unwrap()));
    }

    #[test]
    fn merges_yaml_global_config_below_toml_project_config() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("global").join("morphir.yaml");
        let project = root.path().join("project").join("morphir.toml");
        std::fs::create_dir_all(global.parent().unwrap()).unwrap();
        std::fs::create_dir_all(project.parent().unwrap()).unwrap();
        std::fs::write(
            &global,
            "frontend:\n  language: elm\nir:\n  strict_mode: true\n",
        )
        .unwrap();
        std::fs::write(
            &project,
            "[project]\nname = \"Acme.Project\"\nversion = \"1.0.0\"\n\n[ir]\nstrict_mode = false\n",
        )
        .unwrap();

        let context = load_config_context_with_global(&project, Some(&global)).unwrap();

        assert_eq!(
            context.config.frontend.unwrap().language.as_deref(),
            Some("elm")
        );
        assert!(!context.config.ir.unwrap().strict_mode);
    }
}
