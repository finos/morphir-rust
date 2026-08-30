use std::{collections::BTreeMap, fs, path::Path};

use cap_std::{ambient_authority, fs::Dir};
use morphir_workspace::{FileEntry, RelativePath};

use super::{
    aliases::{
        AliasBudgets, DirectoryAlias, materialize_directory_aliases, record_directory_alias,
    },
    mounts::selected_mount,
    traversal::{BoundaryEvent, build_tree_with},
    *,
};
use crate::config::sources::SourceSelection;

#[cfg(unix)]
use std::os::unix::fs::symlink;

#[cfg(unix)]
fn open_capability(path: &Path) -> Dir {
    Dir::open_ambient_dir(path, ambient_authority()).unwrap()
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

        let tree = selected_mount(&SourceSelection::Explicit(path), Vec::new, description)
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

    let error =
        selected_mount(&SourceSelection::Explicit(path.clone()), Vec::new, "system").unwrap_err();

    assert_eq!(
        error.to_string(),
        format!(
            "Unsupported system config serialization at {}; expected TOML, YAML, or JSON",
            path.display()
        )
    );
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
        &mut |event, path| {
            if !replaced
                && event == BoundaryEvent::BeforeReadConfig
                && path.as_str() == "morphir.toml"
            {
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
        "[workspace]\nmembers = ['packages/*']\n",
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
        &mut |event, path| {
            if !replaced
                && event == BoundaryEvent::BeforeOpenDirectory
                && path.as_str() == "packages"
            {
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

    let error =
        build_tree_with(&cap, root.path(), root.path(), budgets, &mut |_, _| {}).unwrap_err();

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

    let error =
        build_tree_with(&cap, root.path(), root.path(), budgets, &mut |_, _| {}).unwrap_err();

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

    materialize_directory_aliases(&mut aliases, &mut entries, budgets).unwrap();

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

    materialize_directory_aliases(&mut aliases, &mut entries, AliasBudgets::DEFAULT).unwrap();

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

    materialize_directory_aliases(&mut aliases, &mut entries, AliasBudgets::DEFAULT).unwrap();

    assert!(entries.contains_key(&RelativePath::parse("alias/pkg/linked/morphir.toml").unwrap()));
}
