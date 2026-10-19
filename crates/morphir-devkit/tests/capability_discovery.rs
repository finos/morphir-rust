use cap_std::{ambient_authority, fs::Dir};
use morphir_devkit::WorkspaceDiscoveryRoot;
#[test]
fn supplied_directory_must_match_host_label_before_config_reads() {
    let owned = tempfile::tempdir().unwrap();
    let foreign = tempfile::tempdir().unwrap();
    std::fs::write(
        foreign.path().join("morphir.toml"),
        "[project\nsecret = 'must not be read'\n",
    )
    .unwrap();
    let directory = Dir::open_ambient_dir(owned.path(), ambient_authority()).unwrap();
    let error = WorkspaceDiscoveryRoot::from_directory(directory, foreign.path()).unwrap_err();
    assert!(
        error.to_string().contains("workspace.path.not-confined"),
        "{error:#}"
    );
    assert!(!error.to_string().contains("must not be read"));
}
#[test]
fn renamed_and_replaced_root_is_refused_before_replacement_config_capture() {
    let parent = tempfile::tempdir().unwrap();
    let label = parent.path().join("workspace");
    std::fs::create_dir(&label).unwrap();
    std::fs::write(
        label.join("morphir.toml"),
        "[project]\nname = 'owned/root'\n",
    )
    .unwrap();
    let directory = Dir::open_ambient_dir(&label, ambient_authority()).unwrap();
    let bound = WorkspaceDiscoveryRoot::from_directory(directory, &label).unwrap();
    std::fs::rename(&label, parent.path().join("original-root")).unwrap();
    std::fs::create_dir(&label).unwrap();
    std::fs::write(
        label.join("morphir.toml"),
        "[project\nsecret = 'foreign replacement'\n",
    )
    .unwrap();
    let error = bound
        .capture(&morphir_devkit::ConfigLoadOptions::project_only())
        .unwrap_err();
    assert!(
        error.to_string().contains("workspace.path.not-confined"),
        "{error:#}"
    );
    assert!(!error.to_string().contains("foreign replacement"));
}
#[test]
fn captured_details_survive_config_mutation_and_removal_of_the_live_root() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().to_owned();
    let original = "[project]\nname = 'captured/root'\nversion = '1.0.0'\n";
    std::fs::write(path.join("morphir.toml"), original).unwrap();
    let directory = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
    let bound = WorkspaceDiscoveryRoot::from_directory(directory, &path).unwrap();
    let captured = bound
        .capture(&morphir_devkit::ConfigLoadOptions::project_only())
        .unwrap();
    assert_eq!(
        captured
            .request()
            .development_root
            .file_text(&morphir_workspace::RelativePath::parse("morphir.toml").unwrap()),
        Some(original)
    );
    std::fs::write(
        path.join("morphir.toml"),
        "[project]\nname = 'replacement/root'\nversion = '2.0.0'\n",
    )
    .unwrap();
    drop(bound);
    root.close().unwrap();
    let details = captured.into_typed_details().unwrap();
    let project = details.root_config.project.unwrap();
    assert_eq!(project.name, "captured/root");
    assert_eq!(project.version, "1.0.0");
    assert_eq!(details.snapshot.projects[0].name, "captured/root");
}

#[test]
fn one_capture_keeps_member_configs_and_explicit_overlays_without_rereading() {
    use morphir_devkit::{ConfigLoadOptions, EnvSelection, SourceSelection};
    use morphir_workspace::RelativePath;
    let root = tempfile::tempdir().unwrap();
    let mounts = tempfile::tempdir().unwrap();
    let member = root.path().join("packages/orders");
    std::fs::create_dir_all(&member).unwrap();
    std::fs::write(
        root.path().join("morphir.toml"),
        "[workspace]\nmembers = ['packages/*']\n[ir]\nstrict_mode = false\n",
    )
    .unwrap();
    std::fs::write(
        member.join("morphir.toml"),
        "[project]\nname = 'captured/orders'\nversion = '1.0.0'\n",
    )
    .unwrap();
    let system = mounts.path().join("system.toml");
    let global = mounts.path().join("global.toml");
    let personal = mounts.path().join("personal.toml");
    std::fs::write(&system, "[ir]\nformat_version = 2\n").unwrap();
    std::fs::write(&global, "[ir]\nformat_version = 3\n").unwrap();
    std::fs::write(&personal, "[ir]\nstrict_mode = true\n").unwrap();
    let options = ConfigLoadOptions {
        system: SourceSelection::Explicit(system.clone()),
        global: SourceSelection::Explicit(global.clone()),
        user_override: SourceSelection::Explicit(personal.clone()),
        env: EnvSelection::Explicit(vec![("MORPHIR_IR__FORMAT_VERSION".into(), "4".into())]),
        ..ConfigLoadOptions::project_only()
    };
    let bound = WorkspaceDiscoveryRoot::from_directory(
        Dir::open_ambient_dir(root.path(), ambient_authority()).unwrap(),
        root.path(),
    )
    .unwrap();
    let captured = bound.capture(&options).unwrap();
    let exact_request = serde_json::to_vec(captured.request()).unwrap();
    for file in [system, global, personal, member.join("morphir.toml")] {
        std::fs::write(file, "[broken\n").unwrap();
    }
    assert_eq!(
        serde_json::to_vec(captured.request()).unwrap(),
        exact_request
    );
    let details = captured.into_typed_details().unwrap();
    let path = RelativePath::parse("packages/orders").unwrap();
    assert_eq!(details.snapshot.projects.len(), 1);
    assert_eq!(details.snapshot.projects[0].relative_path, path);
    let effective = &details.project_configs[&path];
    assert_eq!(effective.project.as_ref().unwrap().name, "captured/orders");
    let ir = effective.ir.as_ref().unwrap();
    assert_eq!(ir.format_version, 4);
    assert!(ir.strict_mode);
    assert_eq!(details.root_config.ir.as_ref().unwrap().format_version, 4);
    assert!(details.root_config.ir.as_ref().unwrap().strict_mode);
}

#[cfg(unix)]
#[test]
fn retargeted_host_alias_is_refused_even_when_the_original_directory_survives() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("original");
    let foreign = root.path().join("foreign");
    let label = root.path().join("alias");
    std::fs::create_dir(&original).unwrap();
    std::fs::create_dir(&foreign).unwrap();
    symlink(&original, &label).unwrap();
    let bound = WorkspaceDiscoveryRoot::from_directory(
        Dir::open_ambient_dir(&label, ambient_authority()).unwrap(),
        &label,
    )
    .unwrap();
    std::fs::remove_file(&label).unwrap();
    symlink(&foreign, &label).unwrap();
    let error = bound
        .capture(&morphir_devkit::ConfigLoadOptions::project_only())
        .unwrap_err();
    assert!(
        error.to_string().contains("workspace.path.not-confined"),
        "{error:#}"
    );
}

#[cfg(unix)]
#[test]
fn confined_absolute_member_alias_keeps_declared_membership() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::write(
        root.path().join("morphir.toml"),
        "[workspace]\nmembers = ['alias']\n",
    )
    .unwrap();
    std::fs::write(
        real.join("morphir.toml"),
        "[project]\nname = 'captured/alias'\n",
    )
    .unwrap();
    symlink(&real, root.path().join("alias")).unwrap();
    let bound = WorkspaceDiscoveryRoot::from_directory(
        Dir::open_ambient_dir(root.path(), ambient_authority()).unwrap(),
        root.path(),
    )
    .unwrap();
    let details = bound
        .capture(&morphir_devkit::ConfigLoadOptions::project_only())
        .unwrap()
        .into_typed_details()
        .unwrap();
    assert_eq!(details.snapshot.projects[0].relative_path.as_str(), "alias");
    assert_eq!(
        details
            .project_configs
            .values()
            .next()
            .unwrap()
            .project
            .as_ref()
            .unwrap()
            .name,
        "captured/alias"
    );
}

#[cfg(unix)]
#[test]
fn external_member_alias_keeps_existing_confinement_refusal() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let foreign = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("morphir.toml"),
        "[workspace]\nmembers = ['alias']\n",
    )
    .unwrap();
    std::fs::write(
        foreign.path().join("morphir.toml"),
        "[project]\nname = 'foreign/member'\n",
    )
    .unwrap();
    symlink(foreign.path(), root.path().join("alias")).unwrap();
    let bound = WorkspaceDiscoveryRoot::from_directory(
        Dir::open_ambient_dir(root.path(), ambient_authority()).unwrap(),
        root.path(),
    )
    .unwrap();
    let error = bound
        .capture(&morphir_devkit::ConfigLoadOptions::project_only())
        .unwrap_err();
    assert!(
        error.to_string().contains("workspace.path.not-confined"),
        "{error:#}"
    );
}

#[test]
fn later_capture_observes_changes_without_mutating_an_earlier_request() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("morphir.toml");
    std::fs::write(&config, "[project]\nname = 'captured/first'\n").unwrap();
    let bound = WorkspaceDiscoveryRoot::from_directory(
        Dir::open_ambient_dir(root.path(), ambient_authority()).unwrap(),
        root.path(),
    )
    .unwrap();
    let first = bound
        .capture(&morphir_devkit::ConfigLoadOptions::project_only())
        .unwrap();
    std::fs::write(&config, "[project]\nname = 'captured/second'\n").unwrap();
    let second = bound
        .capture(&morphir_devkit::ConfigLoadOptions::project_only())
        .unwrap();
    assert!(!format!("{first:?}").contains("captured/first"));
    assert_eq!(
        first.into_typed_details().unwrap().snapshot.projects[0].name,
        "captured/first"
    );
    assert_eq!(
        second.into_typed_details().unwrap().snapshot.projects[0].name,
        "captured/second"
    );
}
