//! Locating configuration files: project and workspace discovery, global user,
//! system, and user-override candidates, and the platform rules behind them.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

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

pub(crate) fn project_config_candidates(directory: &Path) -> [PathBuf; 6] {
    [
        directory.join("morphir.toml"),
        directory.join("morphir.yaml"),
        directory.join(".morphir").join("morphir.toml"),
        directory.join(".morphir").join("morphir.yaml"),
        directory.join(".config/morphir/config.toml"),
        directory.join(".config/morphir/config.yaml"),
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

/// Find one configuration directly inside a project or workspace directory.
pub fn discover_config_at(directory: &Path) -> Result<Option<PathBuf>> {
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
        if let Some(config) = discover_config_at(&current)? {
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

pub(crate) fn native_global_config_candidates() -> Vec<PathBuf> {
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
    global_config_candidates(
        platform,
        home_dir.as_deref(),
        platform_config_dir.as_deref(),
        xdg_config_home.as_deref(),
    )
}

/// Discover the global user configuration using native platform directories.
pub fn discover_global_config() -> Result<Option<PathBuf>> {
    discover_config_candidates(&native_global_config_candidates())
}

/// Return the system configuration directory for a platform.
///
/// Unix-like systems use `/etc`; Windows uses `%PROGRAMDATA%` and falls back to
/// `C:\ProgramData` when the variable is not set.
pub fn default_system_config_dir(platform: ConfigPlatform, program_data: Option<&Path>) -> PathBuf {
    match platform {
        ConfigPlatform::Windows => program_data
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData")),
        ConfigPlatform::Xdg | ConfigPlatform::MacOs => PathBuf::from("/etc"),
    }
}

/// Build the system configuration candidates below a system configuration directory.
pub fn system_config_candidates(system_config_dir: &Path) -> [PathBuf; 2] {
    let root = system_config_dir.join("morphir");
    [root.join("morphir.toml"), root.join("morphir.yaml")]
}

pub(crate) fn native_system_config_candidates() -> [PathBuf; 2] {
    let platform = current_platform();
    let program_data = match platform {
        ConfigPlatform::Windows => std::env::var_os("PROGRAMDATA").map(PathBuf::from),
        ConfigPlatform::Xdg | ConfigPlatform::MacOs => None,
    };
    system_config_candidates(&default_system_config_dir(
        platform,
        program_data.as_deref(),
    ))
}

/// Discover the system configuration using native platform directories.
pub fn discover_system_config() -> Result<Option<PathBuf>> {
    discover_config_candidates(&native_system_config_candidates())
}

/// The standard location of a primary Morphir configuration file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigLayout {
    /// `morphir.toml` or `morphir.yaml` directly in the project root.
    Root,
    /// `.morphir/morphir.toml` or `.morphir/morphir.yaml`.
    MorphirDirectory,
    /// `.config/morphir/config.toml` or `.config/morphir/config.yaml`.
    DotConfigDirectory,
}

/// Identify the standard layout used by a primary configuration path.
pub fn config_layout(config_path: &Path) -> Option<ConfigLayout> {
    let file_name = config_path.file_name()?.to_str()?;
    let parent = config_path.parent()?;

    match file_name {
        "morphir.toml" | "morphir.yaml" => {
            if parent.file_name().and_then(|name| name.to_str()) == Some(".morphir") {
                Some(ConfigLayout::MorphirDirectory)
            } else {
                Some(ConfigLayout::Root)
            }
        }
        "config.toml" | "config.yaml"
            if parent.file_name().and_then(|name| name.to_str()) == Some("morphir")
                && parent
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str())
                    == Some(".config") =>
        {
            Some(ConfigLayout::DotConfigDirectory)
        }
        _ => None,
    }
}

/// Build the adjacent user override candidates for a standard primary configuration path.
pub fn user_override_candidates(config_path: &Path) -> Option<[PathBuf; 2]> {
    let parent = config_path.parent()?;
    match config_layout(config_path)? {
        ConfigLayout::Root | ConfigLayout::MorphirDirectory => Some([
            parent.join("morphir.user.toml"),
            parent.join("morphir.user.yaml"),
        ]),
        ConfigLayout::DotConfigDirectory => Some([
            parent.join("config.user.toml"),
            parent.join("config.user.yaml"),
        ]),
    }
}

/// Find the adjacent user override, rejecting sibling TOML and YAML files.
pub fn discover_user_override(config_path: &Path) -> Result<Option<PathBuf>> {
    match user_override_candidates(config_path) {
        Some(candidates) => discover_config_candidates(&candidates),
        None => Ok(None),
    }
}

/// Return the project or workspace root represented by a configuration path.
pub fn config_root(config_path: &Path) -> Option<&Path> {
    let parent = config_path.parent()?;
    match config_layout(config_path) {
        Some(ConfigLayout::MorphirDirectory) => parent.parent(),
        Some(ConfigLayout::DotConfigDirectory) => parent.parent()?.parent(),
        Some(ConfigLayout::Root) | None => Some(parent),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().expect("config parent")).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn write_project_config(path: &Path) {
        write_file(path, "project:\n  name: Acme.Project\n  version: 1.0.0\n");
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
    fn discovers_dot_config_morphir_layout() {
        let root = tempfile::tempdir().unwrap();
        let expected = root.path().join(".config/morphir/config.toml");
        write_project_config(&expected);

        assert_eq!(
            discover_config(root.path()).unwrap(),
            Some(expected.clone())
        );
        assert_eq!(config_root(&expected), Some(root.path()));
        assert_eq!(
            user_override_candidates(&expected).unwrap(),
            [
                root.path().join(".config/morphir/config.user.toml"),
                root.path().join(".config/morphir/config.user.yaml"),
            ]
        );
    }

    #[test]
    fn user_override_is_adjacent_to_each_standard_layout() {
        let root = Path::new("/work");

        assert_eq!(
            user_override_candidates(&root.join("morphir.toml")).unwrap(),
            [
                root.join("morphir.user.toml"),
                root.join("morphir.user.yaml")
            ]
        );
        assert_eq!(
            user_override_candidates(&root.join(".morphir/morphir.yaml")).unwrap(),
            [
                root.join(".morphir/morphir.user.toml"),
                root.join(".morphir/morphir.user.yaml")
            ]
        );
    }

    #[test]
    fn rejects_root_primary_and_dot_config_primary_together() {
        let root = tempfile::tempdir().unwrap();
        let root_primary = root.path().join("morphir.toml");
        let dot_config_primary = root.path().join(".config/morphir/config.yaml");
        write_project_config(&root_primary);
        write_project_config(&dot_config_primary);

        let error = discover_config(root.path()).expect_err("ambiguous config");
        let message = error.to_string();
        assert!(message.contains(root_primary.to_str().unwrap()));
        assert!(message.contains(dot_config_primary.to_str().unwrap()));
    }

    #[test]
    fn rejects_sibling_adjacent_user_override_serializations() {
        let root = tempfile::tempdir().unwrap();
        let primary = root.path().join(".config/morphir/config.toml");
        let [toml, yaml] = user_override_candidates(&primary).expect("standard layout");
        write_file(&toml, "[ui]\ntheme = \"light\"\n");
        write_file(&yaml, "ui:\n  theme: dark\n");

        let error = discover_user_override(&primary).expect_err("ambiguous override");
        let message = error.to_string();
        assert!(message.contains(toml.to_str().unwrap()));
        assert!(message.contains(yaml.to_str().unwrap()));
    }

    #[test]
    fn nonstandard_primary_has_no_implicit_user_override() {
        assert_eq!(
            user_override_candidates(Path::new("configs/project.yaml")),
            None
        );
    }

    #[test]
    fn falls_back_to_legacy_json() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("morphir.json");
        write_file(&legacy, r#"{"name": "Legacy", "sourceDirectory": "src"}"#);

        assert_eq!(discover_config_at(root.path()).unwrap(), Some(legacy));
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
    fn rejects_hidden_and_visible_project_configs_together() {
        let root = tempfile::tempdir().unwrap();
        let visible = root.path().join("morphir.yaml");
        let hidden = root.path().join(".morphir").join("morphir.yaml");
        write_project_config(&visible);
        write_project_config(&hidden);

        let error = discover_config(root.path()).expect_err("ambiguous config");
        let message = error.to_string();
        assert!(message.contains(visible.to_str().unwrap()));
        assert!(message.contains(hidden.to_str().unwrap()));
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
        write_file(&candidates[0], "[morphir]\nversion = \"1\"");
        write_file(&candidates[3], "morphir:\n  version: '1'\n");

        let error = discover_config_candidates(&candidates).expect_err("ambiguous config");
        let message = error.to_string();
        assert!(message.contains(candidates[0].to_str().unwrap()));
        assert!(message.contains(candidates[3].to_str().unwrap()));
    }

    #[test]
    fn resolves_system_config_candidates_per_platform() {
        assert_eq!(
            system_config_candidates(&default_system_config_dir(ConfigPlatform::Xdg, None)),
            [
                PathBuf::from("/etc/morphir/morphir.toml"),
                PathBuf::from("/etc/morphir/morphir.yaml"),
            ]
        );
        assert_eq!(
            default_system_config_dir(ConfigPlatform::MacOs, Some(Path::new("/ignored"))),
            PathBuf::from("/etc")
        );
        assert_eq!(
            system_config_candidates(&default_system_config_dir(
                ConfigPlatform::Windows,
                Some(Path::new(r"D:\ProgramData"))
            ))[1],
            PathBuf::from(r"D:\ProgramData").join("morphir/morphir.yaml")
        );
        assert_eq!(
            default_system_config_dir(ConfigPlatform::Windows, None),
            PathBuf::from(r"C:\ProgramData")
        );
    }

    #[test]
    fn discovers_user_override_and_rejects_sibling_serializations() {
        let root = tempfile::tempdir().unwrap();
        let primary = root.path().join(".morphir").join("morphir.toml");
        let [toml, yaml] = user_override_candidates(&primary).expect("standard layout");

        assert_eq!(discover_user_override(&primary).unwrap(), None);

        write_file(&yaml, "ui:\n  theme: dark\n");
        assert_eq!(
            discover_user_override(&primary).unwrap(),
            Some(yaml.clone())
        );

        write_file(&toml, "[ui]\ntheme = \"light\"\n");
        let error = discover_user_override(&primary).expect_err("ambiguous override");
        let message = error.to_string();
        assert!(message.contains(toml.to_str().unwrap()));
        assert!(message.contains(yaml.to_str().unwrap()));
    }

    #[test]
    fn config_root_skips_hidden_directory_for_project_files_only() {
        assert_eq!(
            config_root(Path::new("/p/.morphir/morphir.yaml")),
            Some(Path::new("/p"))
        );
        assert_eq!(
            config_root(Path::new("/p/morphir.toml")),
            Some(Path::new("/p"))
        );
        assert_eq!(
            config_root(Path::new("/p/.morphir/morphir.user.toml")),
            Some(Path::new("/p/.morphir"))
        );
    }
}
