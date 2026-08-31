use super::*;

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
        build_workspace_discovery_request(root.path(), &ConfigLoadOptions::project_only()).unwrap();

    assert!(
        request
            .development_root
            .entries
            .keys()
            .any(|path| path.as_str() == "packages/orders/cycle/orders/morphir.toml")
    );
    assert!(
        request.development_root.entries.keys().all(|path| {
            path.as_str() != "packages/orders/cycle/orders/cycle/orders/morphir.toml"
        })
    );
    assert!(request.development_root.entries.len() < 16);
}
