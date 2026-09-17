use serde_json::json;

const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

pub fn root_with_missing_dependency_catalog() -> String {
    json!({
        "formatVersion": "0.1.0-draft.2",
        "capability": "flat-library",
        "mode": "initial",
        "root": {
            "release": { "packagePath": "example.com/app/root", "version": "1.0.0" },
            "irPackageName": "example/app",
            "manifestDigest": DIGEST,
            "contentDigest": DIGEST,
            "dependencies": [{
                "irPackageName": "example/eligibility",
                "packagePath": "example.com/finance/eligibility",
                "versionRange": {
                    "minimumInclusive": "1.0.0",
                    "maximumExclusive": "2.0.0"
                }
            }]
        },
        "catalogs": []
    })
    .to_string()
}

pub fn independent_outer_shape_faults() -> String {
    json!({
        "formatVersion": "wrong",
        "capability": "flat-library",
        "mode": "initial",
        "root": release("example.com/app/root", "1.0.0", "example/app", vec![]),
        "catalogs": null,
        "extra": true
    })
    .to_string()
}

pub fn invalid_release_identity_siblings() -> String {
    let mut value: serde_json::Value = serde_json::from_str(&empty_initial()).unwrap();
    value["root"]["release"] = json!({ "packagePath": 0, "version": 0 });
    value.to_string()
}

pub fn invalid_requirement_siblings() -> String {
    let mut value: serde_json::Value = serde_json::from_str(&empty_initial()).unwrap();
    value["root"]["dependencies"] = json!([{
        "irPackageName": 0,
        "packagePath": 0,
        "versionRange": { "minimumInclusive": 0, "maximumExclusive": 0 }
    }]);
    value.to_string()
}

pub fn initial_with_lock() -> String {
    let mut input: serde_json::Value = serde_json::from_str(&empty_initial()).unwrap();
    input["lock"] = json!({});
    input.to_string()
}

pub fn duplicate_catalog_with_invalid_subtree() -> String {
    json!({
        "formatVersion": "0.1.0-draft.2",
        "capability": "flat-library",
        "mode": "initial",
        "root": release("example.com/app/root", "1.0.0", "example/app", vec![]),
        "catalogs": [
            { "packagePath": "example.com/pkg/a", "releases": [] },
            {
                "packagePath": "example.com/pkg/a",
                "releases": [release(
                    "example.com/pkg/a",
                    "1.0.0",
                    "example/a",
                    vec![requirement("example/b", "example.com/pkg/b", "2.0.0", "1.0.0")]
                )]
            }
        ]
    })
    .to_string()
}

pub fn empty_interval() -> String {
    json!({
        "formatVersion": "0.1.0-draft.2",
        "capability": "flat-library",
        "mode": "initial",
        "root": release(
            "example.com/app/root",
            "1.0.0",
            "example/app",
            vec![requirement("example/a", "example.com/pkg/a", "1.0.0", "1.0.0")]
        ),
        "catalogs": []
    })
    .to_string()
}

pub fn empty_initial() -> String {
    json!({
        "formatVersion": "0.1.0-draft.2",
        "capability": "flat-library",
        "mode": "initial",
        "root": release("example.com/app/root", "1.0.0", "example/app", vec![]),
        "catalogs": []
    })
    .to_string()
}

pub fn replay_with_newer_release() -> String {
    let root = release(
        "example.com/app/root",
        "1.0.0",
        "example/app",
        vec![requirement(
            "example/eligibility",
            "example.com/finance/eligibility",
            "1.0.0",
            "2.0.0",
        )],
    );
    let older = release(
        "example.com/finance/eligibility",
        "1.2.0",
        "example/eligibility",
        vec![],
    );
    let newer = release(
        "example.com/finance/eligibility",
        "1.3.0",
        "example/eligibility",
        vec![],
    );
    json!({
        "formatVersion": "0.1.0-draft.2",
        "capability": "flat-library",
        "mode": "replay",
        "root": root,
        "releases": [older, newer],
        "lock": lock("1.2.0", DIGEST)
    })
    .to_string()
}

pub fn replay_missing_lock() -> String {
    let mut value: serde_json::Value = serde_json::from_str(&replay_with_newer_release()).unwrap();
    value.as_object_mut().unwrap().remove("lock");
    value.to_string()
}

pub fn replay_with_digest_mismatch() -> String {
    let mut value: serde_json::Value = serde_json::from_str(&replay_with_newer_release()).unwrap();
    value["lock"]["nodes"][1]["contentDigest"] =
        json!("sha256:1111111111111111111111111111111111111111111111111111111111111111");
    value.to_string()
}

pub fn replay_with_duplicate_node() -> String {
    let mut value: serde_json::Value = serde_json::from_str(&replay_with_newer_release()).unwrap();
    let duplicate = value["lock"]["nodes"][1].clone();
    value["lock"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    value.to_string()
}

pub fn replay_with_root_mismatch() -> String {
    let mut value: serde_json::Value = serde_json::from_str(&replay_with_newer_release()).unwrap();
    value["lock"]["root"]["version"] = json!("9.0.0");
    value.to_string()
}

pub fn replay_missing_selected_metadata() -> String {
    let mut value: serde_json::Value = serde_json::from_str(&replay_with_newer_release()).unwrap();
    value["releases"].as_array_mut().unwrap().remove(0);
    value.to_string()
}

pub fn replay_with_disconnected_cycles() -> String {
    let root = release("example.com/app/root", "1.0.0", "example/app", vec![]);
    let a = release("example.com/pkg/a", "1.0.0", "example/a", vec![]);
    let b = release("example.com/pkg/b", "1.0.0", "example/b", vec![]);
    let c = release("example.com/pkg/c", "1.0.0", "example/c", vec![]);
    let d = release("example.com/pkg/d", "1.0.0", "example/d", vec![]);
    let e = release("example.com/pkg/e", "1.0.0", "example/e", vec![]);
    let missing = json!({ "packagePath": "example.com/pkg/missing", "version": "1.0.0" });
    json!({
        "formatVersion": "0.1.0-draft.2",
        "capability": "flat-library",
        "mode": "replay",
        "root": root,
        "releases": [],
        "lock": {
            "root": root["release"],
            "nodes": [
                node(&root, vec![]),
                node(&a, vec![json!({ "irPackageName": "example/b", "target": b["release"] })]),
                node(&b, vec![
                    json!({ "irPackageName": "example/a", "target": a["release"] }),
                    json!({ "irPackageName": "example/c", "target": c["release"] })
                ]),
                node(&c, vec![json!({ "irPackageName": "example/b", "target": b["release"] })]),
                node(&d, vec![json!({ "irPackageName": "example/d", "target": d["release"] })]),
                node(&e, vec![json!({ "irPackageName": "example/missing", "target": missing })])
            ]
        }
    })
    .to_string()
}

pub fn dense_replay_topology() -> String {
    const RELEASES: usize = 80;
    let root_dependencies = (0..RELEASES)
        .map(|index| {
            requirement(
                &format!("example/p{index}"),
                &format!("example.com/pkg/p{index}"),
                "1.0.0",
                "2.0.0",
            )
        })
        .collect();
    let root = release(
        "example.com/app/root",
        "1.0.0",
        "example/app",
        root_dependencies,
    );
    let releases: Vec<_> = (0..RELEASES)
        .map(|index| {
            let dependencies = (index + 1..RELEASES)
                .map(|target| {
                    requirement(
                        &format!("example/p{target}"),
                        &format!("example.com/pkg/p{target}"),
                        "1.0.0",
                        "2.0.0",
                    )
                })
                .collect();
            release(
                &format!("example.com/pkg/p{index}"),
                "1.0.0",
                &format!("example/p{index}"),
                dependencies,
            )
        })
        .collect();
    let root_bindings = releases
        .iter()
        .map(|record| {
            json!({
                "irPackageName": record["irPackageName"],
                "target": record["release"]
            })
        })
        .collect();
    let mut locked_nodes = vec![node(&root, root_bindings)];
    locked_nodes.extend(releases.iter().enumerate().map(|(index, record)| {
        let bindings = releases[index + 1..]
            .iter()
            .map(|target| {
                json!({
                    "irPackageName": target["irPackageName"],
                    "target": target["release"]
                })
            })
            .collect();
        node(record, bindings)
    }));
    json!({
        "formatVersion": "0.1.0-draft.2",
        "capability": "flat-library",
        "mode": "replay",
        "root": root,
        "releases": releases,
        "lock": { "root": root["release"], "nodes": locked_nodes }
    })
    .to_string()
}

pub fn replay_with_selected_release_count(selected_releases: usize) -> String {
    assert!(selected_releases > 0);
    let leaves: Vec<_> = (0..selected_releases - 1)
        .map(|index| {
            release(
                &format!("example.com/pkg/p{index}"),
                "1.0.0",
                &format!("example/p{index}"),
                vec![],
            )
        })
        .collect();
    let root = release(
        "example.com/app/root",
        "1.0.0",
        "example/app",
        leaves
            .iter()
            .map(|record| {
                requirement(
                    record["irPackageName"].as_str().unwrap(),
                    record["release"]["packagePath"].as_str().unwrap(),
                    "1.0.0",
                    "2.0.0",
                )
            })
            .collect(),
    );
    let root_bindings = leaves
        .iter()
        .map(|record| {
            json!({
                "irPackageName": record["irPackageName"],
                "target": record["release"]
            })
        })
        .collect();
    let mut nodes = vec![node(&root, root_bindings)];
    nodes.extend(leaves.iter().map(|record| node(record, vec![])));
    json!({
        "formatVersion": "0.1.0-draft.2",
        "capability": "flat-library",
        "mode": "replay",
        "root": root,
        "releases": leaves,
        "lock": { "root": root["release"], "nodes": nodes }
    })
    .to_string()
}

pub fn update_with_oversized_baseline() -> String {
    const LEAVES: usize = 511;
    let leaves: Vec<_> = (0..LEAVES)
        .map(|index| {
            release(
                &format!("example.com/pkg/p{index}"),
                "1.0.0",
                &format!("example/p{index}"),
                vec![],
            )
        })
        .collect();
    let hub_v1 = release(
        "example.com/pkg/hub",
        "1.0.0",
        "example/hub",
        leaves
            .iter()
            .map(|record| {
                requirement(
                    record["irPackageName"].as_str().unwrap(),
                    record["release"]["packagePath"].as_str().unwrap(),
                    "1.0.0",
                    "2.0.0",
                )
            })
            .collect(),
    );
    let hub_v2 = release("example.com/pkg/hub", "2.0.0", "example/hub", vec![]);
    let root = release(
        "example.com/app/root",
        "1.0.0",
        "example/app",
        vec![requirement(
            "example/hub",
            "example.com/pkg/hub",
            "1.0.0",
            "3.0.0",
        )],
    );
    let hub_bindings = leaves
        .iter()
        .map(|record| {
            json!({
                "irPackageName": record["irPackageName"],
                "target": record["release"]
            })
        })
        .collect();
    let mut selected = vec![(hub_v1.clone(), vec![])];
    selected.extend(leaves.iter().cloned().map(|record| (record, vec![])));
    let mut baseline = graph(&root, selected);
    baseline["nodes"][1]["bindings"] = serde_json::Value::Array(hub_bindings);
    let mut catalogs = vec![catalog("example.com/pkg/hub", vec![hub_v1, hub_v2])];
    catalogs.extend(leaves.iter().map(|record| {
        catalog(
            record["release"]["packagePath"].as_str().unwrap(),
            vec![record.clone()],
        )
    }));
    update(
        root,
        catalogs,
        baseline,
        vec![json!({
            "kind": "exact",
            "packagePath": "example.com/pkg/hub",
            "version": "2.0.0"
        })],
    )
}

pub fn replay_node_with_independent_shape_faults() -> String {
    let mut value: serde_json::Value = serde_json::from_str(&replay_with_newer_release()).unwrap();
    value["lock"]["nodes"] = json!([{
        "release": { "packagePath": 0, "version": 0 },
        "bindings": []
    }]);
    value.to_string()
}

pub fn replay_duplicate_path_suppresses_node_semantics() -> String {
    let root = release("example.com/app/root", "1.0.0", "example/app", vec![]);
    let duplicate_path = json!({
        "release": { "packagePath": "example.com/app/root", "version": "2.0.0" },
        "irPackageName": "example/app",
        "manifestDigest": DIGEST,
        "contentDigest": DIGEST,
        "bindings": [
            { "irPackageName": "example/a", "target": root["release"] },
            { "irPackageName": "example/a", "target": root["release"] }
        ]
    });
    json!({
        "formatVersion": "0.1.0-draft.2",
        "capability": "flat-library",
        "mode": "replay",
        "root": root,
        "releases": [],
        "lock": {
            "root": root["release"],
            "nodes": [node(&root, vec![]), duplicate_path]
        }
    })
    .to_string()
}

pub fn update_targeting_root() -> String {
    let mut value: serde_json::Value = serde_json::from_str(&preservation()).unwrap();
    value["targets"] = json!([{
        "kind": "exact",
        "packagePath": "example.com/app/root",
        "version": "1.0.0"
    }]);
    value.to_string()
}

pub fn update_with_invalid_target_kind(kind: Option<serde_json::Value>) -> String {
    let mut value: serde_json::Value = serde_json::from_str(&preservation()).unwrap();
    let mut target = serde_json::Map::from_iter([
        ("packagePath".into(), json!(0)),
        ("extra".into(), json!(true)),
    ]);
    if let Some(kind) = kind {
        target.insert("kind".into(), kind);
    }
    value["targets"] = serde_json::Value::Array(vec![serde_json::Value::Object(target)]);
    value.to_string()
}

pub fn overlapping_ranges() -> String {
    initial(
        release(
            "example.com/app/root",
            "1.0.0",
            "example/app",
            vec![
                requirement("example/a", "example.com/pkg/a", "1.0.0", "2.0.0"),
                requirement("example/b", "example.com/pkg/b", "1.0.0", "2.0.0"),
            ],
        ),
        vec![
            catalog(
                "example.com/pkg/a",
                vec![release(
                    "example.com/pkg/a",
                    "1.0.0",
                    "example/a",
                    vec![requirement(
                        "example/shared",
                        "example.com/pkg/shared",
                        "1.0.0",
                        "3.0.0",
                    )],
                )],
            ),
            catalog(
                "example.com/pkg/b",
                vec![release(
                    "example.com/pkg/b",
                    "1.0.0",
                    "example/b",
                    vec![requirement(
                        "example/shared",
                        "example.com/pkg/shared",
                        "2.0.0",
                        "4.0.0",
                    )],
                )],
            ),
            catalog(
                "example.com/pkg/shared",
                vec![
                    release("example.com/pkg/shared", "1.0.0", "example/shared", vec![]),
                    release("example.com/pkg/shared", "2.0.0", "example/shared", vec![]),
                    release("example.com/pkg/shared", "3.0.0", "example/shared", vec![]),
                ],
            ),
        ],
    )
}

pub fn backtracking() -> String {
    initial(
        release(
            "example.com/app/root",
            "1.0.0",
            "example/app",
            vec![requirement(
                "example/a",
                "example.com/pkg/a",
                "1.0.0",
                "3.0.0",
            )],
        ),
        vec![
            catalog(
                "example.com/pkg/a",
                vec![
                    release(
                        "example.com/pkg/a",
                        "2.0.0",
                        "example/a",
                        vec![requirement(
                            "example/b",
                            "example.com/pkg/b",
                            "2.0.0",
                            "3.0.0",
                        )],
                    ),
                    release(
                        "example.com/pkg/a",
                        "1.0.0",
                        "example/a",
                        vec![requirement(
                            "example/b",
                            "example.com/pkg/b",
                            "1.0.0",
                            "2.0.0",
                        )],
                    ),
                ],
            ),
            catalog(
                "example.com/pkg/b",
                vec![release("example.com/pkg/b", "1.0.0", "example/b", vec![])],
            ),
        ],
    )
}

pub fn unbounded_versions() -> String {
    initial(
        release(
            "example.com/app/root",
            "1.0.0",
            "example/app",
            vec![requirement(
                "example/a",
                "example.com/pkg/a",
                "9007199254740992.0.0",
                "9007199254740995.0.0",
            )],
        ),
        vec![catalog(
            "example.com/pkg/a",
            vec![
                release(
                    "example.com/pkg/a",
                    "9007199254740993.0.0",
                    "example/a",
                    vec![],
                ),
                release(
                    "example.com/pkg/a",
                    "9007199254740994.0.0",
                    "example/a",
                    vec![],
                ),
            ],
        )],
    )
}

pub fn target_freshness() -> String {
    let root = release(
        "example.com/app/root",
        "1.0.0",
        "example/app",
        vec![requirement(
            "example/a",
            "example.com/pkg/a",
            "1.0.0",
            "3.0.0",
        )],
    );
    let a1 = release(
        "example.com/pkg/a",
        "1.0.0",
        "example/a",
        vec![requirement(
            "example/shared",
            "example.com/pkg/shared",
            "1.0.0",
            "2.0.0",
        )],
    );
    let a2 = release(
        "example.com/pkg/a",
        "2.0.0",
        "example/a",
        vec![requirement(
            "example/shared",
            "example.com/pkg/shared",
            "2.0.0",
            "3.0.0",
        )],
    );
    let shared1 = release("example.com/pkg/shared", "1.0.0", "example/shared", vec![]);
    let shared2 = release("example.com/pkg/shared", "2.0.0", "example/shared", vec![]);
    update(
        root.clone(),
        vec![
            catalog("example.com/pkg/a", vec![a1.clone(), a2]),
            catalog("example.com/pkg/shared", vec![shared1.clone(), shared2]),
        ],
        graph(
            &root,
            vec![
                (a1.clone(), vec![("example/shared", &shared1)]),
                (shared1.clone(), vec![]),
            ],
        ),
        vec![json!({ "kind": "eligible", "packagePath": "example.com/pkg/a" })],
    )
}

pub fn preservation() -> String {
    let root = release(
        "example.com/app/root",
        "1.0.0",
        "example/app",
        vec![requirement(
            "example/a",
            "example.com/pkg/a",
            "1.0.0",
            "2.0.0",
        )],
    );
    let a1 = release(
        "example.com/pkg/a",
        "1.0.0",
        "example/a",
        vec![requirement(
            "example/shared",
            "example.com/pkg/shared",
            "1.0.0",
            "3.0.0",
        )],
    );
    let shared1 = release("example.com/pkg/shared", "1.0.0", "example/shared", vec![]);
    let shared2 = release("example.com/pkg/shared", "2.0.0", "example/shared", vec![]);
    update(
        root.clone(),
        vec![
            catalog("example.com/pkg/a", vec![a1.clone()]),
            catalog("example.com/pkg/shared", vec![shared1.clone(), shared2]),
        ],
        graph(
            &root,
            vec![
                (a1.clone(), vec![("example/shared", &shared1)]),
                (shared1.clone(), vec![]),
            ],
        ),
        vec![json!({
            "kind": "exact",
            "packagePath": "example.com/pkg/a",
            "version": "1.0.0"
        })],
    )
}

pub fn capability_failure() -> String {
    initial(
        release(
            "example.com/app/root",
            "1.0.0",
            "example/app",
            vec![
                requirement("example/a", "example.com/pkg/a", "1.0.0", "2.0.0"),
                requirement("example/b", "example.com/pkg/b", "1.0.0", "2.0.0"),
            ],
        ),
        vec![
            catalog(
                "example.com/pkg/a",
                vec![release(
                    "example.com/pkg/a",
                    "1.0.0",
                    "example/a",
                    vec![requirement(
                        "example/shared",
                        "example.com/pkg/shared",
                        "1.0.0",
                        "2.0.0",
                    )],
                )],
            ),
            catalog(
                "example.com/pkg/b",
                vec![release(
                    "example.com/pkg/b",
                    "1.0.0",
                    "example/b",
                    vec![requirement(
                        "example/shared",
                        "example.com/pkg/shared",
                        "2.0.0",
                        "3.0.0",
                    )],
                )],
            ),
            catalog(
                "example.com/pkg/shared",
                vec![
                    release("example.com/pkg/shared", "1.0.0", "example/shared", vec![]),
                    release("example.com/pkg/shared", "2.0.0", "example/shared", vec![]),
                ],
            ),
        ],
    )
}

pub fn scope_conflict() -> String {
    let root = release(
        "example.com/app/root",
        "1.0.0",
        "example/app",
        vec![
            requirement("example/a", "example.com/pkg/a", "1.0.0", "3.0.0"),
            requirement(
                "example/reporting",
                "example.com/pkg/reporting",
                "1.0.0",
                "3.0.0",
            ),
        ],
    );
    let a1 = release(
        "example.com/pkg/a",
        "1.0.0",
        "example/a",
        vec![requirement(
            "example/shared",
            "example.com/pkg/shared",
            "1.0.0",
            "2.0.0",
        )],
    );
    let a2 = release(
        "example.com/pkg/a",
        "2.0.0",
        "example/a",
        vec![requirement(
            "example/shared",
            "example.com/pkg/shared",
            "2.0.0",
            "3.0.0",
        )],
    );
    let reporting1 = release(
        "example.com/pkg/reporting",
        "1.0.0",
        "example/reporting",
        vec![requirement(
            "example/shared",
            "example.com/pkg/shared",
            "1.0.0",
            "2.0.0",
        )],
    );
    let reporting2 = release(
        "example.com/pkg/reporting",
        "2.0.0",
        "example/reporting",
        vec![requirement(
            "example/shared",
            "example.com/pkg/shared",
            "2.0.0",
            "3.0.0",
        )],
    );
    let shared1 = release("example.com/pkg/shared", "1.0.0", "example/shared", vec![]);
    let shared2 = release("example.com/pkg/shared", "2.0.0", "example/shared", vec![]);
    update(
        root.clone(),
        vec![
            catalog("example.com/pkg/a", vec![a1.clone(), a2]),
            catalog(
                "example.com/pkg/reporting",
                vec![reporting1.clone(), reporting2],
            ),
            catalog("example.com/pkg/shared", vec![shared1.clone(), shared2]),
        ],
        graph(
            &root,
            vec![
                (a1, vec![("example/shared", &shared1)]),
                (reporting1, vec![("example/shared", &shared1)]),
                (shared1.clone(), vec![]),
            ],
        ),
        vec![json!({
            "kind": "exact",
            "packagePath": "example.com/pkg/a",
            "version": "2.0.0"
        })],
    )
}

pub fn search_resource_exhaustion() -> String {
    const RELEASES: usize = 513;
    let root = release(
        "example.com/app/root",
        "1.0.0",
        "example/app",
        vec![requirement(
            "example/p0",
            "example.com/pkg/p0",
            "1.0.0",
            "2.0.0",
        )],
    );
    let catalogs = (0..RELEASES)
        .map(|index| {
            let dependencies = (index + 1 < RELEASES)
                .then(|| {
                    requirement(
                        &format!("example/p{}", index + 1),
                        &format!("example.com/pkg/p{}", index + 1),
                        "1.0.0",
                        "2.0.0",
                    )
                })
                .into_iter()
                .collect();
            catalog(
                &format!("example.com/pkg/p{index}"),
                vec![release(
                    &format!("example.com/pkg/p{index}"),
                    "1.0.0",
                    &format!("example/p{index}"),
                    dependencies,
                )],
            )
        })
        .collect();
    initial(root, catalogs)
}

pub fn dense_acyclic_graph() -> String {
    dense_acyclic_graph_with_releases(20)
}

pub fn dense_search_work_exhaustion() -> String {
    dense_acyclic_graph_with_releases(24)
}

fn dense_acyclic_graph_with_releases(releases: usize) -> String {
    let root_dependencies = (0..releases)
        .map(|index| {
            requirement(
                &format!("example/p{index}"),
                &format!("example.com/pkg/p{index}"),
                "1.0.0",
                "2.0.0",
            )
        })
        .collect();
    let root = release(
        "example.com/app/root",
        "1.0.0",
        "example/app",
        root_dependencies,
    );
    let catalogs = (0..releases)
        .map(|index| {
            let dependencies = (0..index)
                .map(|dependency| {
                    requirement(
                        &format!("example/p{dependency}"),
                        &format!("example.com/pkg/p{dependency}"),
                        "1.0.0",
                        "2.0.0",
                    )
                })
                .collect();
            catalog(
                &format!("example.com/pkg/p{index}"),
                vec![release(
                    &format!("example.com/pkg/p{index}"),
                    "1.0.0",
                    &format!("example/p{index}"),
                    dependencies,
                )],
            )
        })
        .collect();
    initial(root, catalogs)
}

fn update(
    root: serde_json::Value,
    catalogs: Vec<serde_json::Value>,
    lock: serde_json::Value,
    targets: Vec<serde_json::Value>,
) -> String {
    json!({
        "formatVersion": "0.1.0-draft.2",
        "capability": "flat-library",
        "mode": "update",
        "root": root,
        "catalogs": catalogs,
        "lock": lock,
        "targets": targets
    })
    .to_string()
}

fn graph(
    root: &serde_json::Value,
    selected: Vec<(serde_json::Value, Vec<(&str, &serde_json::Value)>)>,
) -> serde_json::Value {
    let root_bindings = root["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|dependency| {
            let target = selected
                .iter()
                .find(|(record, _)| record["release"]["packagePath"] == dependency["packagePath"])
                .unwrap();
            json!({
                "irPackageName": dependency["irPackageName"],
                "target": target.0["release"]
            })
        })
        .collect::<Vec<_>>();
    let mut nodes = vec![node(root, root_bindings)];
    nodes.extend(selected.iter().map(|(record, bindings)| {
        node(
            record,
            bindings
                .iter()
                .map(|(name, target)| json!({ "irPackageName": name, "target": target["release"] }))
                .collect(),
        )
    }));
    json!({ "root": root["release"], "nodes": nodes })
}

fn node(record: &serde_json::Value, bindings: Vec<serde_json::Value>) -> serde_json::Value {
    json!({
        "release": record["release"],
        "irPackageName": record["irPackageName"],
        "manifestDigest": record["manifestDigest"],
        "contentDigest": record["contentDigest"],
        "bindings": bindings
    })
}

fn initial(root: serde_json::Value, catalogs: Vec<serde_json::Value>) -> String {
    json!({
        "formatVersion": "0.1.0-draft.2",
        "capability": "flat-library",
        "mode": "initial",
        "root": root,
        "catalogs": catalogs
    })
    .to_string()
}

fn catalog(package_path: &str, releases: Vec<serde_json::Value>) -> serde_json::Value {
    json!({ "packagePath": package_path, "releases": releases })
}

fn lock(eligibility_version: &str, eligibility_digest: &str) -> serde_json::Value {
    json!({
        "root": { "packagePath": "example.com/app/root", "version": "1.0.0" },
        "nodes": [
            {
                "release": { "packagePath": "example.com/app/root", "version": "1.0.0" },
                "irPackageName": "example/app",
                "manifestDigest": DIGEST,
                "contentDigest": DIGEST,
                "bindings": [{
                    "irPackageName": "example/eligibility",
                    "target": {
                        "packagePath": "example.com/finance/eligibility",
                        "version": eligibility_version
                    }
                }]
            },
            {
                "release": {
                    "packagePath": "example.com/finance/eligibility",
                    "version": eligibility_version
                },
                "irPackageName": "example/eligibility",
                "manifestDigest": DIGEST,
                "contentDigest": eligibility_digest,
                "bindings": []
            }
        ]
    })
}

fn release(
    package_path: &str,
    version: &str,
    ir_package_name: &str,
    dependencies: Vec<serde_json::Value>,
) -> serde_json::Value {
    json!({
        "release": { "packagePath": package_path, "version": version },
        "irPackageName": ir_package_name,
        "manifestDigest": DIGEST,
        "contentDigest": DIGEST,
        "dependencies": dependencies
    })
}

fn requirement(
    ir_package_name: &str,
    package_path: &str,
    minimum: &str,
    maximum: &str,
) -> serde_json::Value {
    json!({
        "irPackageName": ir_package_name,
        "packagePath": package_path,
        "versionRange": {
            "minimumInclusive": minimum,
            "maximumExclusive": maximum
        }
    })
}
