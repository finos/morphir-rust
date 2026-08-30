use super::*;
#[cfg(unix)]
#[test]
fn alias_amplified_configuration_bytes_are_request_bounded() {
    let root = tempfile::tempdir().unwrap();
    let workspace = "[workspace]\nmembers = ['aliases/*']\n";
    let project = "[project]\nname = 'acme/real'\n";
    fs::write(root.path().join("morphir.toml"), workspace).unwrap();
    let real = root.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("morphir.toml"), project).unwrap();
    let aliases = root.path().join("aliases");
    fs::create_dir(&aliases).unwrap();
    symlink(&real, aliases.join("one")).unwrap();
    symlink(&real, aliases.join("two")).unwrap();
    let cap = open_capability(root.path());
    let budgets = TraversalBudgets {
        config_bytes: workspace.len() + project.len() * 2,
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
    assert!(error.to_string().contains("configuration bytes budget"));
}

#[cfg(unix)]
#[test]
fn alias_amplified_payload_accepts_the_exact_byte_limit() {
    let root = tempfile::tempdir().unwrap();
    let workspace = "[workspace]\nmembers = ['alias']\n";
    let project = "[project]\nname = 'acme/real'\n";
    fs::write(root.path().join("morphir.toml"), workspace).unwrap();
    let real = root.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("morphir.toml"), project).unwrap();
    symlink(&real, root.path().join("alias")).unwrap();
    let cap = open_capability(root.path());

    let tree = build_tree_with(
        &cap,
        root.path(),
        root.path(),
        AliasBudgets::DEFAULT,
        TraversalBudgets {
            config_bytes: workspace.len() + project.len() * 2,
            ..TraversalBudgets::DEFAULT
        },
        &mut |_, _| {},
    )
    .unwrap();

    let payload_bytes = tree.entries.values().map(entry_bytes).sum::<usize>();
    assert_eq!(payload_bytes, workspace.len() + project.len() * 2);
}

#[test]
fn root_and_global_mount_share_the_request_byte_budget() {
    let root = tempfile::tempdir().unwrap();
    let workspace = "[project]\nname = 'acme/root'\n";
    let global = "[ir]\nstrict_mode = true\n";
    fs::write(root.path().join("morphir.toml"), workspace).unwrap();
    let global_path = root.path().join("global.toml");
    fs::write(&global_path, global).unwrap();
    let options = ConfigLoadOptions {
        global: SourceSelection::Explicit(global_path),
        ..ConfigLoadOptions::project_only()
    };

    let (_, exact) = bind_workspace_discovery_request_with_budgets(
        root.path(),
        &options,
        AliasBudgets::DEFAULT,
        TraversalBudgets {
            config_bytes: workspace.len() + global.len(),
            ..TraversalBudgets::DEFAULT
        },
    )
    .unwrap();
    assert!(exact.morphir_home.is_some());

    let error = bind_workspace_discovery_request_with_budgets(
        root.path(),
        &options,
        AliasBudgets::DEFAULT,
        TraversalBudgets {
            config_bytes: workspace.len() + global.len() - 1,
            ..TraversalBudgets::DEFAULT
        },
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("workspace.traversal.resource-limit")
    );
}

#[test]
fn explicit_user_override_shares_the_request_byte_budget() {
    let root = tempfile::tempdir().unwrap();
    let workspace = "[project]\nname = 'acme/root'\n";
    let user = "[project]\nversion = '2.0.0'\n";
    fs::write(root.path().join("morphir.toml"), workspace).unwrap();
    let user_path = root.path().join("selected.toml");
    fs::write(&user_path, user).unwrap();
    let options = ConfigLoadOptions {
        user_override: SourceSelection::Explicit(user_path),
        ..ConfigLoadOptions::project_only()
    };

    bind_workspace_discovery_request_with_budgets(
        root.path(),
        &options,
        AliasBudgets::DEFAULT,
        TraversalBudgets {
            config_bytes: workspace.len() + user.len(),
            ..TraversalBudgets::DEFAULT
        },
    )
    .unwrap();

    let error = bind_workspace_discovery_request_with_budgets(
        root.path(),
        &options,
        AliasBudgets::DEFAULT,
        TraversalBudgets {
            config_bytes: workspace.len() + user.len() - 1,
            ..TraversalBudgets::DEFAULT
        },
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("workspace.traversal.resource-limit")
    );
}

#[test]
fn replaced_natural_override_does_not_count_toward_final_payload() {
    let root = tempfile::tempdir().unwrap();
    let workspace = "[project]\nname = 'acme/root'\n";
    let natural = "[project]\nversion = '1.0.0'\ndescription = 'gone'\n";
    let explicit = "[project]\nversion = '2.0.0'\n";
    fs::write(root.path().join("morphir.toml"), workspace).unwrap();
    fs::write(root.path().join("morphir.user.toml"), natural).unwrap();
    let explicit_path = root.path().join("selected.toml");
    fs::write(&explicit_path, explicit).unwrap();
    let options = ConfigLoadOptions {
        user_override: SourceSelection::Explicit(explicit_path),
        ..ConfigLoadOptions::project_only()
    };

    let (_, request) = bind_workspace_discovery_request_with_budgets(
        root.path(),
        &options,
        AliasBudgets::DEFAULT,
        TraversalBudgets {
            config_bytes: workspace.len() + explicit.len(),
            ..TraversalBudgets::DEFAULT
        },
    )
    .unwrap();

    assert_eq!(
        request
            .development_root
            .file_text(&RelativePath::parse("morphir.user.toml").unwrap()),
        Some(explicit)
    );
}

#[test]
fn explicit_root_override_removes_stray_dot_config_override() {
    let root = tempfile::tempdir().unwrap();
    let workspace = "[project]\nname = 'acme/root'\n";
    let stray = "[project]\nversion = '1'\n";
    let explicit = "[project]\nversion = '2'\n";
    fs::write(root.path().join("morphir.toml"), workspace).unwrap();
    let dot_config = root.path().join(".config/morphir");
    fs::create_dir_all(&dot_config).unwrap();
    fs::write(dot_config.join("config.user.yaml"), stray).unwrap();
    let explicit_path = root.path().join("selected.toml");
    fs::write(&explicit_path, explicit).unwrap();
    let options = ConfigLoadOptions {
        user_override: SourceSelection::Explicit(explicit_path),
        ..ConfigLoadOptions::project_only()
    };

    let (_, request) = bind_workspace_discovery_request_with_budgets(
        root.path(),
        &options,
        AliasBudgets::DEFAULT,
        TraversalBudgets {
            config_bytes: workspace.len() + explicit.len(),
            ..TraversalBudgets::DEFAULT
        },
    )
    .unwrap();

    assert!(
        !request
            .development_root
            .entries
            .contains_key(&RelativePath::parse(".config/morphir/config.user.yaml").unwrap())
    );
    assert_eq!(
        request
            .development_root
            .file_text(&RelativePath::parse("morphir.user.toml").unwrap()),
        Some(explicit)
    );
}

#[test]
fn explicit_dot_config_override_removes_stray_root_override() {
    let root = tempfile::tempdir().unwrap();
    let workspace = "project:\n  name: acme/root\n";
    let stray = "[project]\nversion = '1'\n";
    let explicit = "project:\n  version: '2'\n";
    let dot_config = root.path().join(".config/morphir");
    fs::create_dir_all(&dot_config).unwrap();
    fs::write(dot_config.join("config.yaml"), workspace).unwrap();
    fs::write(root.path().join("morphir.user.toml"), stray).unwrap();
    let explicit_path = root.path().join("selected.yaml");
    fs::write(&explicit_path, explicit).unwrap();
    let options = ConfigLoadOptions {
        user_override: SourceSelection::Explicit(explicit_path),
        ..ConfigLoadOptions::project_only()
    };

    let (_, request) = bind_workspace_discovery_request_with_budgets(
        root.path(),
        &options,
        AliasBudgets::DEFAULT,
        TraversalBudgets {
            config_bytes: workspace.len() + explicit.len(),
            ..TraversalBudgets::DEFAULT
        },
    )
    .unwrap();

    assert!(
        !request
            .development_root
            .entries
            .contains_key(&RelativePath::parse("morphir.user.toml").unwrap())
    );
    assert_eq!(
        request
            .development_root
            .file_text(&RelativePath::parse(".config/morphir/config.user.yaml").unwrap()),
        Some(explicit)
    );
}

#[test]
fn replaced_override_bodies_have_a_separate_resident_cap() {
    let root = tempfile::tempdir().unwrap();
    let workspace = "[project]\nname = 'a'\n";
    let natural_toml = "x = '12345678901234567890'\n";
    let natural_yaml = "x: 123456789012345678901234\n";
    let explicit = "[project]\nversion = '2'\n";
    fs::write(root.path().join("morphir.toml"), workspace).unwrap();
    fs::write(root.path().join("morphir.user.toml"), natural_toml).unwrap();
    fs::write(root.path().join("morphir.user.yaml"), natural_yaml).unwrap();
    let explicit_path = root.path().join("selected.toml");
    fs::write(&explicit_path, explicit).unwrap();
    let options = ConfigLoadOptions {
        user_override: SourceSelection::Explicit(explicit_path),
        ..ConfigLoadOptions::project_only()
    };
    let limit = workspace.len() + explicit.len();
    assert!(natural_toml.len() <= limit);
    assert!(natural_yaml.len() <= limit);
    assert!(natural_toml.len() + natural_yaml.len() > limit);

    let error = bind_workspace_discovery_request_with_budgets(
        root.path(),
        &options,
        AliasBudgets::DEFAULT,
        TraversalBudgets {
            config_bytes: limit,
            ..TraversalBudgets::DEFAULT
        },
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("workspace.traversal.resource-limit")
    );
}

#[test]
fn skipped_override_bodies_are_not_transiently_materialized() {
    let root = tempfile::tempdir().unwrap();
    let workspace = "[workspace]\nmembers = ['packages/*']\n";
    fs::write(root.path().join("morphir.toml"), workspace).unwrap();
    let packages = root.path().join("packages");
    fs::create_dir(&packages).unwrap();
    for index in 0..32 {
        let package = packages.join(format!("package-{index:02}"));
        fs::create_dir(&package).unwrap();
        fs::write(
            package.join("morphir.user.toml"),
            "[project]\ndescription = 'this skipped body must never be resident'\n",
        )
        .unwrap();
    }
    let options = ConfigLoadOptions {
        user_override: SourceSelection::Skip,
        ..ConfigLoadOptions::project_only()
    };

    let (_, request) = bind_workspace_discovery_request_with_budgets(
        root.path(),
        &options,
        AliasBudgets::DEFAULT,
        TraversalBudgets {
            config_bytes: workspace.len(),
            ..TraversalBudgets::DEFAULT
        },
    )
    .unwrap();

    assert_eq!(
        request
            .development_root
            .entries
            .values()
            .map(entry_bytes)
            .sum::<usize>(),
        workspace.len()
    );
}

#[cfg(unix)]
#[test]
fn aliases_do_not_materialize_skipped_override_bodies() {
    let root = tempfile::tempdir().unwrap();
    let workspace = "[workspace]\nmembers = ['alias']\n";
    fs::write(root.path().join("morphir.toml"), workspace).unwrap();
    let real = root.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::write(
        real.join("morphir.user.toml"),
        "[project]\ndescription = 'large skipped alias body'\n",
    )
    .unwrap();
    symlink(&real, root.path().join("alias")).unwrap();
    let options = ConfigLoadOptions {
        user_override: SourceSelection::Skip,
        ..ConfigLoadOptions::project_only()
    };

    let (_, request) = bind_workspace_discovery_request_with_budgets(
        root.path(),
        &options,
        AliasBudgets::DEFAULT,
        TraversalBudgets {
            config_bytes: workspace.len(),
            ..TraversalBudgets::DEFAULT
        },
    )
    .unwrap();

    assert!(
        !request
            .development_root
            .entries
            .keys()
            .any(|path| { path.as_str().ends_with("morphir.user.toml") })
    );
}

#[test]
fn payload_budget_accepts_exact_limit_and_rejects_overflow() {
    let mut exact = PayloadBudget::new(usize::MAX);
    exact.reserve(usize::MAX).unwrap();

    let error = exact.reserve(1).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("workspace.traversal.resource-limit")
    );
}
