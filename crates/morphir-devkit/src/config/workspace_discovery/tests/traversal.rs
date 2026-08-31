use super::*;
#[test]
fn root_replaced_after_capability_open_is_rejected_by_identity() {
    let parent = tempfile::tempdir().unwrap();
    let grant = parent.path().join("granted-root");
    fs::create_dir(&grant).unwrap();
    fs::write(
        grant.join("morphir.toml"),
        "[project]\nname = 'inside/project'\n",
    )
    .unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(
        outside.path().join("morphir.toml"),
        "[project]\nname = 'external/project'\n",
    )
    .unwrap();
    let held = parent.path().join("held-original");

    let error = bind_workspace_discovery_request_with_hook(
        &grant,
        &ConfigLoadOptions::project_only(),
        &mut |_| {
            fs::rename(&grant, &held).unwrap();
            symlink(outside.path(), &grant).unwrap();
        },
    )
    .unwrap_err();

    assert!(error.to_string().contains("workspace.path.not-confined"));
    assert!(error.to_string().contains("development root changed"));
    assert!(error.to_string().contains(&grant.display().to_string()));
    assert!(!error.to_string().contains("external/project"));
}

#[cfg(unix)]
#[test]
fn config_replaced_by_external_symlink_before_read_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let config = root.path().join("morphir.toml");
    fs::write(&config, "[project]\nname = 'inside/project'\n").unwrap();
    let external = outside.path().join("external.toml");
    fs::write(&external, "[project]\nname = 'external/project'\n").unwrap();
    let cap = open_capability(root.path());
    let mut replaced = false;

    let error = build_tree_with(
        &cap,
        root.path(),
        root.path(),
        AliasBudgets::DEFAULT,
        TraversalBudgets::DEFAULT,
        &mut |event, path| {
            if !replaced && event == BoundaryEvent::ReadConfig && path.as_str() == "morphir.toml" {
                fs::rename(&config, root.path().join("original.toml")).unwrap();
                symlink(&external, &config).unwrap();
                replaced = true;
            }
        },
    )
    .unwrap_err();

    assert!(replaced);
    assert!(error.to_string().contains("Failed to read confined"));
    assert!(!error.to_string().contains("external/project"));
}

#[cfg(unix)]
#[test]
fn absolute_internal_config_symlink_is_read_through_the_capability() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("actual-config.toml");
    fs::write(&target, "[project]\nname = 'inside/project'\n").unwrap();
    symlink(&target, root.path().join("morphir.toml")).unwrap();
    let cap = open_capability(root.path());

    let tree = build_tree_with(
        &cap,
        root.path(),
        root.path(),
        AliasBudgets::DEFAULT,
        TraversalBudgets::DEFAULT,
        &mut |_, _| {},
    )
    .unwrap();

    assert_eq!(
        tree.file_text(&RelativePath::parse("morphir.toml").unwrap()),
        Some("[project]\nname = 'inside/project'\n")
    );
    assert!(
        !tree
            .entries
            .contains_key(&RelativePath::parse("actual-config.toml").unwrap())
    );
}

#[cfg(unix)]
#[test]
fn directory_replaced_by_external_symlink_before_open_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("morphir.toml"),
        "[workspace]\nmembers = ['packages/*']\nexclude = ['private/**']\n",
    )
    .unwrap();
    let packages = root.path().join("packages");
    fs::create_dir(&packages).unwrap();
    let external_packages = outside.path().join("packages");
    fs::create_dir(&external_packages).unwrap();
    fs::write(
        external_packages.join("morphir.toml"),
        "[project]\nname = 'external/project'\n",
    )
    .unwrap();
    let cap = open_capability(root.path());
    let mut replaced = false;

    let error = build_tree_with(
        &cap,
        root.path(),
        root.path(),
        AliasBudgets::DEFAULT,
        TraversalBudgets::DEFAULT,
        &mut |event, path| {
            if !replaced && event == BoundaryEvent::OpenDirectory && path.as_str() == "packages" {
                fs::rename(&packages, root.path().join("original-packages")).unwrap();
                symlink(&external_packages, &packages).unwrap();
                replaced = true;
            }
        },
    )
    .unwrap_err();

    assert!(replaced);
    assert!(
        error
            .to_string()
            .contains("Failed to open confined directory")
    );
    assert!(!error.to_string().contains("external/project"));
}

#[cfg(unix)]
#[test]
fn unreadable_granted_directory_is_an_explicit_discovery_error() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("morphir.toml"),
        "[workspace]\nmembers = ['packages/*']\n",
    )
    .unwrap();
    let private = root.path().join("private");
    fs::create_dir(&private).unwrap();
    let original_permissions = fs::metadata(&private).unwrap().permissions();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o000)).unwrap();
    let cap = open_capability(root.path());

    let result = build_tree_with(
        &cap,
        root.path(),
        root.path(),
        AliasBudgets::DEFAULT,
        TraversalBudgets::DEFAULT,
        &mut |_, _| {},
    );
    fs::set_permissions(&private, original_permissions).unwrap();

    let error = result.expect_err("unreadable granted directories must not be skipped");
    assert!(error.to_string().contains("workspace.traversal.unreadable"));
    assert!(error.to_string().contains("private"));
}

#[cfg(unix)]
#[test]
fn traversal_entry_budget_fires_before_buffered_children_are_processed() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("morphir.toml"),
        "[workspace]\nmembers = ['packages/*']\n",
    )
    .unwrap();
    fs::create_dir(root.path().join("packages")).unwrap();
    for index in 0..4 {
        fs::write(root.path().join(format!("unrelated-{index}")), "ignored").unwrap();
    }
    let cap = open_capability(root.path());
    let budgets = TraversalBudgets {
        real_entries: 2,
        ..TraversalBudgets::DEFAULT
    };
    let mut boundary_events = 0;

    let error = build_tree_with(
        &cap,
        root.path(),
        root.path(),
        AliasBudgets::DEFAULT,
        budgets,
        &mut |_, _| boundary_events += 1,
    )
    .unwrap_err();

    assert_eq!(boundary_events, 0);
    assert!(
        error
            .to_string()
            .contains("workspace.traversal.resource-limit")
    );
    assert!(error.to_string().contains("real entries budget 2"));
}

#[cfg(unix)]
#[test]
fn excluded_subtree_is_still_subject_to_traversal_limits() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("morphir.toml"),
        "[workspace]\nmembers = ['packages/*']\nexclude = ['target/**']\n",
    )
    .unwrap();
    fs::create_dir(root.path().join("packages")).unwrap();
    let target = root.path().join("target");
    fs::create_dir(&target).unwrap();
    for index in 0..4 {
        fs::write(target.join(format!("cache-{index}")), "ignored").unwrap();
    }
    let cap = open_capability(root.path());
    let budgets = TraversalBudgets {
        real_entries: 6,
        ..TraversalBudgets::DEFAULT
    };

    let error = build_tree_with(
        &cap,
        root.path(),
        root.path(),
        AliasBudgets::DEFAULT,
        budgets,
        &mut |_, _| {},
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("workspace.traversal.resource-limit")
    );
    assert!(error.to_string().contains("real entries budget 6"));
}

#[cfg(unix)]
#[test]
fn unreadable_and_invalid_utf8_configs_have_stable_diagnostics() {
    let unreadable_root = tempfile::tempdir().unwrap();
    let unreadable = unreadable_root.path().join("morphir.toml");
    fs::write(&unreadable, "[project]\nname = 'acme/root'\n").unwrap();
    let original_permissions = fs::metadata(&unreadable).unwrap().permissions();
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
    let unreadable_result = build_workspace_discovery_request(
        unreadable_root.path(),
        &ConfigLoadOptions::project_only(),
    );
    fs::set_permissions(&unreadable, original_permissions).unwrap();
    let unreadable_error = unreadable_result.unwrap_err().to_string();
    assert!(unreadable_error.contains("workspace.traversal.unreadable"));
    assert!(unreadable_error.contains("morphir.toml"));

    let invalid_root = tempfile::tempdir().unwrap();
    fs::write(invalid_root.path().join("morphir.toml"), [0xff, 0xfe]).unwrap();
    let invalid_error =
        build_workspace_discovery_request(invalid_root.path(), &ConfigLoadOptions::project_only())
            .unwrap_err()
            .to_string();
    assert!(invalid_error.contains("workspace.traversal.unreadable"));
    assert!(invalid_error.contains("morphir.toml"));
    assert!(!invalid_error.contains('�'));
}

#[cfg(unix)]
#[test]
fn confined_metadata_failures_have_stable_unreadable_diagnostics() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("morphir.toml");
    fs::write(&config, "[project]\nname = 'acme/root'\n").unwrap();
    let cap = open_capability(root.path());
    let mut removed = false;

    let error = build_tree_with(
        &cap,
        root.path(),
        root.path(),
        AliasBudgets::DEFAULT,
        TraversalBudgets::DEFAULT,
        &mut |event, path| {
            if !removed && event == BoundaryEvent::InspectEntry && path.as_str() == "morphir.toml" {
                fs::remove_file(&config).unwrap();
                removed = true;
            }
        },
    )
    .unwrap_err();

    assert!(removed);
    assert!(error.to_string().contains("workspace.traversal.unreadable"));
    assert!(error.to_string().contains("morphir.toml"));
}

#[cfg(unix)]
#[test]
fn established_symlink_target_metadata_failure_is_unreadable() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("actual.toml");
    fs::write(&target, "[project]\nname = 'acme/root'\n").unwrap();
    symlink(&target, root.path().join("morphir.toml")).unwrap();
    let cap = open_capability(root.path());
    let mut removed = false;

    let error = build_tree_with(
        &cap,
        root.path(),
        root.path(),
        AliasBudgets::DEFAULT,
        TraversalBudgets::DEFAULT,
        &mut |event, path| {
            if !removed
                && event == BoundaryEvent::InspectSymlinkTarget
                && path.as_str() == "morphir.toml"
            {
                fs::remove_file(&target).unwrap();
                removed = true;
            }
        },
    )
    .unwrap_err();

    assert!(removed);
    assert!(error.to_string().contains("workspace.traversal.unreadable"));
    assert!(error.to_string().contains("morphir.toml"));
}

#[cfg(unix)]
#[test]
fn unresolved_symlink_target_remains_a_confinement_error() {
    let root = tempfile::tempdir().unwrap();
    symlink(
        root.path().join("missing.toml"),
        root.path().join("morphir.toml"),
    )
    .unwrap();

    let error = build_workspace_discovery_request(root.path(), &ConfigLoadOptions::project_only())
        .unwrap_err()
        .to_string();

    assert!(error.contains("workspace.path.not-confined"));
    assert!(!error.contains("workspace.traversal.unreadable"));
    assert!(error.contains("morphir.toml"));
}

#[cfg(unix)]
#[test]
fn traversal_directory_depth_and_config_byte_budgets_are_enforced() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("morphir.toml"),
        "[workspace]\nmembers = ['packages/*']\n",
    )
    .unwrap();
    fs::create_dir_all(root.path().join("packages/orders/nested")).unwrap();
    let cap = open_capability(root.path());

    for (budgets, resource) in [
        (
            TraversalBudgets {
                real_directories: 1,
                ..TraversalBudgets::DEFAULT
            },
            "real directories budget 1",
        ),
        (
            TraversalBudgets {
                max_depth: 1,
                ..TraversalBudgets::DEFAULT
            },
            "depth budget 1",
        ),
        (
            TraversalBudgets {
                config_bytes: 1,
                ..TraversalBudgets::DEFAULT
            },
            "configuration bytes budget 1",
        ),
    ] {
        let error = build_tree_with(
            &cap,
            root.path(),
            root.path(),
            AliasBudgets::DEFAULT,
            budgets,
            &mut |_, _| {},
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("workspace.traversal.resource-limit")
        );
        assert!(
            error.to_string().contains(resource),
            "unexpected diagnostic for {resource}: {error}"
        );
    }
}

#[test]
fn narrow_workspace_with_unrelated_subtree_remains_correct() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("morphir.toml"),
        "[workspace]\nmembers = ['packages/*']\nexclude = ['target/**']\n",
    )
    .unwrap();
    let member = root.path().join("packages/orders");
    fs::create_dir_all(&member).unwrap();
    fs::write(
        member.join("morphir.toml"),
        "[project]\nname = 'acme/orders'\n",
    )
    .unwrap();
    for index in 0..32 {
        fs::create_dir_all(root.path().join(format!("target/cache-{index}/nested"))).unwrap();
    }

    let snapshot = discover_workspace(root.path(), &ConfigLoadOptions::project_only()).unwrap();

    assert_eq!(snapshot.projects.len(), 1);
    assert_eq!(
        snapshot.projects[0].relative_path.as_str(),
        "packages/orders"
    );
}
