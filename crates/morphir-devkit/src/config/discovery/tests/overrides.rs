use super::*;

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
