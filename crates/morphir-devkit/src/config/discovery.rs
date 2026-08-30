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
///
/// `morphir_home` is the relocated Morphir home directory (from `MORPHIR_HOME`);
/// when present it replaces the `<home>/.morphir` candidate root.
pub fn global_config_candidates(
    platform: ConfigPlatform,
    home_dir: Option<&Path>,
    platform_config_dir: Option<&Path>,
    xdg_config_home: Option<&Path>,
    morphir_home: Option<&Path>,
) -> Vec<PathBuf> {
    let valid_xdg = xdg_config_home.filter(|path| is_absolute_for(platform, path));
    let config_dir = match platform {
        ConfigPlatform::Xdg | ConfigPlatform::MacOs => valid_xdg.or(platform_config_dir),
        ConfigPlatform::Windows => platform_config_dir,
    };

    let home_root = match morphir_home {
        Some(relocated) => Some(relocated.to_path_buf()),
        None => home_dir.map(|root| root.join(".morphir")),
    };

    config_dir
        .into_iter()
        .map(|root| root.join("morphir"))
        .chain(home_root)
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
    let morphir_home = morphir_common::home::MorphirHome::resolve()
        .ok()
        .filter(|home| home.is_relocated())
        .map(|home| home.root().to_path_buf());
    global_config_candidates(
        platform,
        home_dir.as_deref(),
        platform_config_dir.as_deref(),
        xdg_config_home.as_deref(),
        morphir_home.as_deref(),
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
mod tests;
