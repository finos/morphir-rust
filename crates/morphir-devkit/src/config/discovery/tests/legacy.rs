use super::*;

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
