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
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use morphir_workspace::{
        DiscoveryRequest, FileEntry, FileTree, ProjectState, RelativePath,
        WORKSPACE_DISCOVERY_PROTOCOL,
    };

    use crate::{
        ConfigLoadOptions, SourceSelection, build_workspace_discovery_request, discover_workspace,
    };

    fn write_file(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().expect("config parent")).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn write_project_config(path: &Path) {
        write_file(path, "project:\n  name: Acme.Project\n  version: 1.0.0\n");
    }

    fn workspace_fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/workspace-discovery/valid-monorepo")
    }

    fn fixture_request() -> DiscoveryRequest {
        fn walk(root: &Path, directory: &Path, entries: &mut BTreeMap<RelativePath, FileEntry>) {
            let mut children = std::fs::read_dir(directory)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>();
            children.sort();
            for child in children {
                let relative = child
                    .strip_prefix(root)
                    .unwrap()
                    .components()
                    .map(|component| component.as_os_str().to_str().unwrap())
                    .collect::<Vec<_>>()
                    .join("/");
                let relative = RelativePath::parse(relative).unwrap();
                if child.is_dir() {
                    entries.insert(relative, FileEntry::Directory);
                    walk(root, &child, entries);
                } else {
                    entries.insert(
                        relative,
                        FileEntry::File {
                            text: std::fs::read_to_string(&child).unwrap(),
                        },
                    );
                }
            }
        }

        let root = workspace_fixture_root();
        let mut entries = BTreeMap::from([(RelativePath::root(), FileEntry::Directory)]);
        walk(&root, &root, &mut entries);
        DiscoveryRequest {
            protocol_version: WORKSPACE_DISCOVERY_PROTOCOL,
            development_root: FileTree { entries },
            morphir_home: None,
            system_config: None,
            environment: BTreeMap::new(),
            cli_overlay: serde_json::json!({}),
        }
    }

    #[test]
    fn native_request_and_snapshot_match_portable_fixture_discovery() {
        let options = ConfigLoadOptions {
            user_override: SourceSelection::Discover,
            ..ConfigLoadOptions::project_only()
        };
        let expected_request = fixture_request();

        let actual_request = build_workspace_discovery_request(&workspace_fixture_root(), &options)
            .expect("native request");
        let actual_snapshot =
            discover_workspace(&workspace_fixture_root(), &options).expect("native discovery");
        let expected_snapshot = morphir_workspace::discover(expected_request.clone())
            .into_result()
            .expect("portable discovery");

        assert_eq!(actual_request, expected_request);
        assert_eq!(actual_snapshot, expected_snapshot);
        assert_eq!(actual_snapshot.projects[0].relative_path.as_str(), ".");
        assert_eq!(actual_snapshot.projects[0].name, "acme/root");
        assert!(
            actual_snapshot
                .projects
                .iter()
                .all(|project| project.relative_path.as_str() != "packages/ignored")
        );
        assert_eq!(
            actual_snapshot
                .projects
                .iter()
                .find(|project| project.relative_path.as_str() == "packages/broken")
                .unwrap()
                .state,
            ProjectState::Error
        );
        assert_eq!(
            actual_snapshot
                .projects
                .iter()
                .filter(|project| project.name == "acme/risk")
                .map(|project| project.relative_path.as_str())
                .collect::<Vec<_>>(),
            ["packages/duplicate", "packages/risk"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn native_tree_rejects_symlink_that_escapes_root() {
        use std::os::unix::fs::symlink;

        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        let outside = base.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(
            root.join("morphir.toml"),
            "[workspace]\nmembers = [\"linked\"]\n",
        )
        .unwrap();
        std::fs::write(
            outside.join("morphir.toml"),
            "[project]\nname = \"outside/project\"\n",
        )
        .unwrap();
        symlink(&outside, root.join("linked")).unwrap();

        let error = discover_workspace(&root, &ConfigLoadOptions::project_only()).unwrap_err();
        let message = error.to_string();

        assert!(message.contains("workspace.path.not-confined"));
        assert!(message.contains("linked"));
        assert!(message.contains(&outside.display().to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn native_tree_terminates_directory_symlink_cycle() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("morphir.toml"),
            "[workspace]\nmembers = [\"packages/*\"]\n",
        )
        .unwrap();
        let member = root.path().join("packages/orders");
        std::fs::create_dir_all(&member).unwrap();
        std::fs::write(
            member.join("morphir.toml"),
            "[project]\nname = \"acme/orders\"\n",
        )
        .unwrap();
        symlink(root.path().join("packages"), member.join("cycle")).unwrap();

        let snapshot = discover_workspace(root.path(), &ConfigLoadOptions::project_only()).unwrap();

        assert_eq!(snapshot.projects.len(), 1);
        assert_eq!(snapshot.projects[0].name, "acme/orders");
    }

    #[cfg(unix)]
    #[test]
    fn internal_symlink_alias_that_sorts_first_does_not_hide_real_member() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("morphir.toml"),
            "[workspace]\nmembers = [\"real/*\"]\n",
        )
        .unwrap();
        let member = root.path().join("real/orders");
        std::fs::create_dir_all(root.path().join("aliases")).unwrap();
        std::fs::create_dir_all(&member).unwrap();
        std::fs::write(
            member.join("morphir.toml"),
            "[project]\nname = \"acme/orders\"\n",
        )
        .unwrap();
        symlink(&member, root.path().join("aliases/orders")).unwrap();

        let snapshot = discover_workspace(root.path(), &ConfigLoadOptions::project_only()).unwrap();

        assert_eq!(snapshot.projects.len(), 1);
        assert_eq!(snapshot.projects[0].relative_path.as_str(), "real/orders");
        assert_eq!(snapshot.projects[0].name, "acme/orders");
    }

    #[cfg(unix)]
    #[test]
    fn internal_symlink_alias_materializes_an_alias_only_member_once() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("morphir.toml"),
            "[workspace]\nmembers = [\"aliases/*\"]\n",
        )
        .unwrap();
        let member = root.path().join("real/orders");
        std::fs::create_dir_all(root.path().join("aliases")).unwrap();
        std::fs::create_dir_all(&member).unwrap();
        std::fs::write(
            member.join("morphir.toml"),
            "[project]\nname = \"acme/orders\"\n",
        )
        .unwrap();
        symlink(&member, root.path().join("aliases/orders")).unwrap();

        let snapshot = discover_workspace(root.path(), &ConfigLoadOptions::project_only()).unwrap();

        assert_eq!(snapshot.projects.len(), 1);
        assert_eq!(
            snapshot.projects[0].relative_path.as_str(),
            "aliases/orders"
        );
        assert_eq!(snapshot.projects[0].name, "acme/orders");
    }

    #[cfg(unix)]
    #[test]
    fn nested_internal_alias_materializes_an_alias_only_member() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("morphir.toml"),
            "[workspace]\nmembers = [\"alias/linked\"]\n",
        )
        .unwrap();
        let outer = root.path().join("real/outer");
        let project = root.path().join("projects/orders");
        std::fs::create_dir_all(&outer).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("morphir.toml"),
            "[project]\nname = \"acme/orders\"\n",
        )
        .unwrap();
        symlink(&project, outer.join("linked")).unwrap();
        symlink(&outer, root.path().join("alias")).unwrap();

        let snapshot = discover_workspace(root.path(), &ConfigLoadOptions::project_only()).unwrap();

        assert_eq!(snapshot.projects.len(), 1);
        assert_eq!(snapshot.projects[0].relative_path.as_str(), "alias/linked");
        assert_eq!(snapshot.projects[0].name, "acme/orders");
    }

    #[cfg(unix)]
    #[test]
    fn nested_alias_cycle_has_one_bounded_synthetic_layer() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("morphir.toml"),
            "[workspace]\nmembers = [\"packages/*\"]\n",
        )
        .unwrap();
        let member = root.path().join("packages/orders");
        std::fs::create_dir_all(&member).unwrap();
        std::fs::write(
            member.join("morphir.toml"),
            "[project]\nname = \"acme/orders\"\n",
        )
        .unwrap();
        symlink(root.path().join("packages"), member.join("cycle")).unwrap();

        let request =
            build_workspace_discovery_request(root.path(), &ConfigLoadOptions::project_only())
                .unwrap();

        assert!(
            request
                .development_root
                .entries
                .keys()
                .any(|path| path.as_str() == "packages/orders/cycle/orders/morphir.toml")
        );
        assert!(request.development_root.entries.keys().all(|path| {
            path.as_str() != "packages/orders/cycle/orders/cycle/orders/morphir.toml"
        }));
        assert!(request.development_root.entries.len() < 16);
    }

    #[test]
    fn explicit_user_override_replaces_natural_root_override() {
        let root = tempfile::tempdir().unwrap();
        let explicit = root.path().join("selected.yaml");
        std::fs::write(
            root.path().join("morphir.toml"),
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("morphir.user.toml"),
            "[project]\nversion = \"2.0.0\"\n",
        )
        .unwrap();
        std::fs::write(&explicit, "project:\n  version: 3.0.0\n").unwrap();
        let options = ConfigLoadOptions {
            user_override: SourceSelection::Explicit(explicit),
            ..ConfigLoadOptions::project_only()
        };

        let snapshot = discover_workspace(root.path(), &options).unwrap();

        assert_eq!(snapshot.projects[0].version.as_deref(), Some("3.0.0"));
    }

    #[test]
    fn explicit_root_user_override_preserves_member_adjacent_override_precedence() {
        let root = tempfile::tempdir().unwrap();
        let explicit = root.path().join("selected.toml");
        let member = root.path().join("packages/orders");
        std::fs::create_dir_all(&member).unwrap();
        std::fs::write(
            root.path().join("morphir.toml"),
            "[workspace]\nmembers = [\"packages/*\"]\n\n[project]\nname = \"acme/root\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        std::fs::write(
            member.join("morphir.toml"),
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        std::fs::write(
            member.join("morphir.user.toml"),
            "[project]\nversion = \"3.0.0\"\n",
        )
        .unwrap();
        std::fs::write(&explicit, "[project]\nversion = \"2.0.0\"\n").unwrap();
        let options = ConfigLoadOptions {
            user_override: SourceSelection::Explicit(explicit),
            ..ConfigLoadOptions::project_only()
        };

        let snapshot = discover_workspace(root.path(), &options).unwrap();

        assert_eq!(
            snapshot
                .projects
                .iter()
                .find(|project| project.relative_path.as_str() == ".")
                .unwrap()
                .version
                .as_deref(),
            Some("2.0.0")
        );
        assert_eq!(
            snapshot
                .projects
                .iter()
                .find(|project| project.relative_path.as_str() == "packages/orders")
                .unwrap()
                .version
                .as_deref(),
            Some("3.0.0")
        );
    }

    #[test]
    fn explicit_user_override_reports_directory_collision() {
        let root = tempfile::tempdir().unwrap();
        let explicit = root.path().join("selected.toml");
        std::fs::write(
            root.path().join("morphir.toml"),
            "[project]\nname = \"acme/root\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        std::fs::create_dir(root.path().join("morphir.user.toml")).unwrap();
        std::fs::write(&explicit, "[project]\nversion = \"2.0.0\"\n").unwrap();
        let options = ConfigLoadOptions {
            user_override: SourceSelection::Explicit(explicit.clone()),
            ..ConfigLoadOptions::project_only()
        };

        let error = discover_workspace(root.path(), &options).unwrap_err();
        let message = error.to_string();

        assert!(message.contains(&explicit.display().to_string()));
        assert!(message.contains("morphir.user.toml"));
        assert!(message.contains("already occupied"));
    }

    #[test]
    fn explicit_user_override_rejects_legacy_root() {
        let root = tempfile::tempdir().unwrap();
        let explicit = root.path().join("selected.toml");
        std::fs::write(
            root.path().join("morphir.json"),
            r#"{"name":"acme/legacy","sourceDirectory":"src"}"#,
        )
        .unwrap();
        std::fs::write(&explicit, "[project]\nversion = \"3.0.0\"\n").unwrap();
        let options = ConfigLoadOptions {
            user_override: SourceSelection::Explicit(explicit.clone()),
            ..ConfigLoadOptions::project_only()
        };

        let error = discover_workspace(root.path(), &options).unwrap_err();
        let message = error.to_string();

        assert!(message.contains(&explicit.display().to_string()));
        assert!(message.contains("modern TOML/YAML root config"));
    }

    #[test]
    fn native_request_keeps_only_configuration_environment_variables() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("morphir.toml"),
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        let options = ConfigLoadOptions {
            env: crate::EnvSelection::Explicit(vec![
                ("MORPHIR_PROJECT__VERSION".to_owned(), "2.0.0".to_owned()),
                ("MORPHIR_HOME".to_owned(), "/not/config".to_owned()),
                ("PATH".to_owned(), "/bin".to_owned()),
            ]),
            ..ConfigLoadOptions::project_only()
        };

        let request = build_workspace_discovery_request(root.path(), &options).unwrap();
        let snapshot = discover_workspace(root.path(), &options).unwrap();

        assert_eq!(
            request.environment,
            BTreeMap::from([("MORPHIR_PROJECT__VERSION".to_owned(), "2.0.0".to_owned())])
        );
        assert_eq!(snapshot.projects[0].version.as_deref(), Some("2.0.0"));
    }

    #[test]
    fn custom_environment_prefix_preserves_reserved_looking_configuration_keys() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("morphir.toml"),
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        let options = ConfigLoadOptions {
            env: crate::EnvSelection::Explicit(vec![
                ("APP_HOME".to_owned(), "project-home".to_owned()),
                ("APP_LOG_DIR".to_owned(), "project-logs".to_owned()),
                ("APP_IR__STRICT_MODE".to_owned(), "true".to_owned()),
                ("APP_PROJECT__VERSION".to_owned(), "2.0.0".to_owned()),
                ("MORPHIR_HOME".to_owned(), "/operational-home".to_owned()),
                ("MORPHIR_LOG_DIR".to_owned(), "/operational-logs".to_owned()),
            ]),
            env_prefix: "APP".to_owned(),
            ..ConfigLoadOptions::project_only()
        };

        let request = build_workspace_discovery_request(root.path(), &options).unwrap();
        let snapshot = discover_workspace(root.path(), &options).unwrap();

        assert_eq!(
            request.environment,
            BTreeMap::from([
                ("MORPHIR__HOME".to_owned(), "project-home".to_owned()),
                ("MORPHIR__IR__STRICT_MODE".to_owned(), "true".to_owned()),
                ("MORPHIR__LOG_DIR".to_owned(), "project-logs".to_owned()),
                ("MORPHIR__PROJECT__VERSION".to_owned(), "2.0.0".to_owned()),
            ])
        );
        assert_eq!(snapshot.projects[0].version.as_deref(), Some("2.0.0"));
    }

    #[test]
    fn missing_explicit_system_config_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("morphir.toml"),
            "[project]\nname = \"acme/orders\"\n",
        )
        .unwrap();
        let missing = root.path().join("missing-system.toml");
        let options = ConfigLoadOptions {
            system: SourceSelection::Explicit(missing.clone()),
            ..ConfigLoadOptions::project_only()
        };

        let error = build_workspace_discovery_request(root.path(), &options).unwrap_err();

        assert!(error.to_string().contains("explicit system config"));
        assert!(error.to_string().contains(&missing.display().to_string()));
    }

    #[test]
    fn missing_explicit_global_config_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("morphir.toml"),
            "[project]\nname = \"acme/orders\"\n",
        )
        .unwrap();
        let missing = root.path().join("missing-global.yaml");
        let options = ConfigLoadOptions {
            global: SourceSelection::Explicit(missing.clone()),
            ..ConfigLoadOptions::project_only()
        };

        let error = build_workspace_discovery_request(root.path(), &options).unwrap_err();

        assert!(error.to_string().contains("explicit global user config"));
        assert!(error.to_string().contains(&missing.display().to_string()));
    }

    #[test]
    fn native_tree_does_not_read_unrecognized_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("morphir.toml"),
            "[project]\nname = \"acme/orders\"\n",
        )
        .unwrap();
        std::fs::write(root.path().join("asset.bin"), [0xff, 0xfe, 0xfd]).unwrap();

        let request =
            build_workspace_discovery_request(root.path(), &ConfigLoadOptions::project_only())
                .unwrap();

        assert!(
            !request
                .development_root
                .entries
                .contains_key(&RelativePath::parse("asset.bin").unwrap())
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_tree_ignores_unrecognized_non_utf8_file_names() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("morphir.toml"),
            "[project]\nname = \"acme/orders\"\n",
        )
        .unwrap();
        std::fs::write(root.path().join(OsString::from_vec(vec![0xff])), "ignored").unwrap();

        let snapshot = discover_workspace(root.path(), &ConfigLoadOptions::project_only()).unwrap();

        assert_eq!(snapshot.projects[0].name, "acme/orders");
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
            None,
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
    fn relocated_morphir_home_replaces_home_candidates() {
        let candidates = global_config_candidates(
            ConfigPlatform::Xdg,
            Some(Path::new("/home/alice")),
            Some(Path::new("/home/alice/.config")),
            None,
            Some(Path::new("/sandbox/mh")),
        );

        assert_eq!(
            candidates,
            vec![
                PathBuf::from("/home/alice/.config/morphir/morphir.toml"),
                PathBuf::from("/home/alice/.config/morphir/morphir.yaml"),
                PathBuf::from("/sandbox/mh/morphir.toml"),
                PathBuf::from("/sandbox/mh/morphir.yaml"),
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
            None,
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
            None,
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
