use super::*;
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
