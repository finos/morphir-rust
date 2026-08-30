use std::{collections::BTreeMap, fs};

use morphir_workspace::{FileEntry, FileTree, RelativePath};

#[cfg(unix)]
use std::path::Path;

#[cfg(unix)]
use cap_std::{ambient_authority, fs::Dir};

use super::{
    aliases::{
        AliasBudgets, DirectoryAlias, materialize_directory_aliases, record_directory_alias,
    },
    budget::{PayloadBudget, PayloadKind, entry_bytes},
    mounts::{apply_user_override_selection, selected_mount},
    traversal::{BoundaryEvent, TraversalBudgets, build_tree_with},
    *,
};
use crate::config::sources::SourceSelection;

#[cfg(unix)]
use std::os::unix::fs::symlink;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)]
fn open_capability(path: &Path) -> Dir {
    Dir::open_ambient_dir(path, ambient_authority()).unwrap()
}

fn payload_for_entries(entries: &BTreeMap<RelativePath, FileEntry>) -> PayloadBudget {
    let mut payload = PayloadBudget::new(TraversalBudgets::DEFAULT.config_bytes);
    for entry in entries.values() {
        payload.reserve(entry_bytes(entry)).unwrap();
    }
    payload
}

#[test]
fn explicit_system_and_global_mounts_accept_supported_extension_variants() {
    let directory = tempfile::tempdir().unwrap();
    let cases = [
        ("system.yml", "system", "morphir.yaml"),
        ("global.YAML", "global user", "morphir.yaml"),
        ("system.TOML", "system", "morphir.toml"),
        ("global.JSON", "global user", "morphir.json"),
    ];

    for (file_name, description, virtual_name) in cases {
        let path = directory.path().join(file_name);
        let contents = format!("contents for {file_name}");
        fs::write(&path, &contents).unwrap();

        let mut payload = PayloadBudget::new(TraversalBudgets::DEFAULT.config_bytes);
        let tree = selected_mount(
            &SourceSelection::Explicit(path),
            Vec::new,
            description,
            &mut payload,
        )
        .unwrap()
        .expect("explicit config should be mounted");

        assert_eq!(
            tree.file_text(&RelativePath::parse(virtual_name).unwrap()),
            Some(contents.as_str()),
            "explicit {description} config {file_name} should be normalized to {virtual_name}"
        );
    }
}

#[test]
fn explicit_mounts_preserve_unsupported_extension_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("morphir.txt");
    fs::write(&path, "not a supported serialization").unwrap();

    let mut payload = PayloadBudget::new(TraversalBudgets::DEFAULT.config_bytes);
    let error = selected_mount(
        &SourceSelection::Explicit(path.clone()),
        Vec::new,
        "system",
        &mut payload,
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        format!(
            "Unsupported system config serialization at {}; expected TOML, YAML, or JSON",
            path.display()
        )
    );
}

#[test]
fn explicit_user_overrides_accept_supported_extension_variants() {
    let directory = tempfile::tempdir().unwrap();
    let cases = [
        (
            "selected.yml",
            "project:\n  version: 2.0.0\n",
            "morphir.user.yaml",
        ),
        (
            "selected.YAML",
            "project:\n  version: 3.0.0\n",
            "morphir.user.yaml",
        ),
        (
            "selected.TOML",
            "[project]\nversion = \"4.0.0\"\n",
            "morphir.user.toml",
        ),
    ];

    for (file_name, contents, virtual_name) in cases {
        let source = directory.path().join(file_name);
        fs::write(&source, contents).unwrap();
        let mut tree = FileTree {
            entries: BTreeMap::from([
                (RelativePath::root(), FileEntry::Directory),
                (
                    RelativePath::parse("morphir.toml").unwrap(),
                    FileEntry::File {
                        text: "[project]\nname = \"acme/root\"\n".to_owned(),
                    },
                ),
            ]),
        };

        let mut payload = payload_for_entries(&tree.entries);
        apply_user_override_selection(&mut tree, &SourceSelection::Explicit(source), &mut payload)
            .unwrap();

        assert_eq!(
            tree.file_text(&RelativePath::parse(virtual_name).unwrap()),
            Some(contents),
            "explicit user override {file_name} should be normalized to {virtual_name}"
        );
    }
}

#[cfg(unix)]
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

#[cfg(unix)]
#[test]
fn alias_budget_is_fixed_and_reports_a_stable_code() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("morphir.toml"),
        "[workspace]\nmembers = ['alias']\n",
    )
    .unwrap();
    let real = root.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("morphir.toml"), "[project]\nname = 'acme/real'\n").unwrap();
    symlink(&real, root.path().join("alias")).unwrap();
    let cap = open_capability(root.path());
    let budgets = AliasBudgets {
        alias_edges: 0,
        ..AliasBudgets::DEFAULT
    };

    let error = build_tree_with(
        &cap,
        root.path(),
        root.path(),
        budgets,
        TraversalBudgets::DEFAULT,
        &mut |_, _| {},
    )
    .unwrap_err();

    assert!(error.to_string().contains("workspace.alias.resource-limit"));
    assert!(error.to_string().contains("alias edges budget 0"));
}

#[test]
fn traversal_rejects_the_first_alias_edge_over_budget_before_storing_it() {
    let mut aliases = Vec::new();
    let mut allocated = 0;
    let budgets = AliasBudgets {
        alias_edges: 1,
        ..AliasBudgets::DEFAULT
    };
    record_directory_alias(&mut aliases, budgets, || {
        allocated += 1;
        DirectoryAlias {
            lexical_path: RelativePath::parse("alias-a").unwrap(),
            canonical_target: RelativePath::parse("real-a").unwrap(),
        }
    })
    .unwrap();

    let error = record_directory_alias(&mut aliases, budgets, || {
        allocated += 1;
        DirectoryAlias {
            lexical_path: RelativePath::parse("alias-b").unwrap(),
            canonical_target: RelativePath::parse("real-b").unwrap(),
        }
    })
    .unwrap_err();

    assert_eq!(aliases.len(), 1);
    assert_eq!(allocated, 1);
    assert!(error.to_string().contains("workspace.alias.resource-limit"));
    assert!(error.to_string().contains("alias edges budget 1"));
}

#[cfg(unix)]
#[test]
fn alias_work_budget_is_checked_before_snapshot_cloning() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("morphir.toml"),
        "[workspace]\nmembers = ['alias']\n",
    )
    .unwrap();
    fs::create_dir(root.path().join("real")).unwrap();
    symlink(root.path().join("real"), root.path().join("alias")).unwrap();
    let cap = open_capability(root.path());
    let budgets = AliasBudgets {
        total_work: 0,
        ..AliasBudgets::DEFAULT
    };

    let error = build_tree_with(
        &cap,
        root.path(),
        root.path(),
        budgets,
        TraversalBudgets::DEFAULT,
        &mut |_, _| {},
    )
    .unwrap_err();

    assert!(error.to_string().contains("workspace.alias.resource-limit"));
    assert!(error.to_string().contains("total work budget 0"));
}

#[test]
fn many_distinct_shallow_alias_targets_use_bounded_subtree_index_work() {
    let mut entries = BTreeMap::from([
        (RelativePath::root(), FileEntry::Directory),
        (RelativePath::parse("real").unwrap(), FileEntry::Directory),
    ]);
    let mut aliases = Vec::new();
    for index in 0..128 {
        let target = RelativePath::parse(format!("real/{index:03}")).unwrap();
        entries.insert(target.clone(), FileEntry::Directory);
        entries.insert(
            target.join("morphir.toml").unwrap(),
            FileEntry::File {
                text: format!("[project]\nname = 'acme/project-{index:03}'\n"),
            },
        );
        aliases.push(DirectoryAlias {
            lexical_path: RelativePath::parse(format!("alias/{index:03}")).unwrap(),
            canonical_target: target,
        });
    }
    let budgets = AliasBudgets {
        total_work: 10_000,
        ..AliasBudgets::DEFAULT
    };

    let mut payload = payload_for_entries(&entries);
    materialize_directory_aliases(&mut aliases, &mut entries, budgets, &mut payload, &|_| {
        PayloadKind::Final
    })
    .unwrap();

    assert_eq!(
        entries
            .keys()
            .filter(|path| path.as_str().starts_with("alias/")
                && path.as_str().ends_with("morphir.toml"))
            .count(),
        128
    );
}

#[test]
fn punctuation_sibling_does_not_hide_direct_alias_descendants() {
    let target = RelativePath::parse("real/pkg").unwrap();
    let punctuation_sibling = RelativePath::parse("real/pkg!shadow").unwrap();
    let config = target.join("morphir.toml").unwrap();
    assert!(target < punctuation_sibling);
    assert!(punctuation_sibling < config);
    let mut entries = BTreeMap::from([
        (RelativePath::root(), FileEntry::Directory),
        (target.clone(), FileEntry::Directory),
        (punctuation_sibling, FileEntry::Directory),
        (
            config,
            FileEntry::File {
                text: "[project]\nname = 'acme/pkg'\n".to_owned(),
            },
        ),
    ]);
    let mut aliases = [DirectoryAlias {
        lexical_path: RelativePath::parse("alias/pkg").unwrap(),
        canonical_target: target,
    }];

    let mut payload = payload_for_entries(&entries);
    materialize_directory_aliases(
        &mut aliases,
        &mut entries,
        AliasBudgets::DEFAULT,
        &mut payload,
        &|_| PayloadKind::Final,
    )
    .unwrap();

    assert!(entries.contains_key(&RelativePath::parse("alias/pkg/morphir.toml").unwrap()));
}

#[test]
fn punctuation_sibling_does_not_hide_nested_alias_edges() {
    let target = RelativePath::parse("real/pkg").unwrap();
    let punctuation_alias = RelativePath::parse("real/pkg!shadow").unwrap();
    let nested_alias = RelativePath::parse("real/pkg/linked").unwrap();
    assert!(target < punctuation_alias);
    assert!(punctuation_alias < nested_alias);
    let orders = RelativePath::parse("projects/orders").unwrap();
    let shadow = RelativePath::parse("projects/shadow").unwrap();
    let mut entries = BTreeMap::from([
        (RelativePath::root(), FileEntry::Directory),
        (target.clone(), FileEntry::Directory),
        (orders.clone(), FileEntry::Directory),
        (shadow.clone(), FileEntry::Directory),
        (
            orders.join("morphir.toml").unwrap(),
            FileEntry::File {
                text: "[project]\nname = 'acme/orders'\n".to_owned(),
            },
        ),
    ]);
    let mut aliases = [
        DirectoryAlias {
            lexical_path: RelativePath::parse("alias/pkg").unwrap(),
            canonical_target: target,
        },
        DirectoryAlias {
            lexical_path: punctuation_alias,
            canonical_target: shadow,
        },
        DirectoryAlias {
            lexical_path: nested_alias,
            canonical_target: orders,
        },
    ];

    let mut payload = payload_for_entries(&entries);
    materialize_directory_aliases(
        &mut aliases,
        &mut entries,
        AliasBudgets::DEFAULT,
        &mut payload,
        &|_| PayloadKind::Final,
    )
    .unwrap();

    assert!(entries.contains_key(&RelativePath::parse("alias/pkg/linked/morphir.toml").unwrap()));
}
