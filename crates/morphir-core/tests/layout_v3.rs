//! The v3 document tree: a classic `Library` or `Specs` distribution laid out with the v4 tree's
//! layout, every file saying `formatVersion: "3.1.0"`.

use morphir_core::ir::DiagnosticCode;
use morphir_core::ir::classic::{Access, Distribution, DistributionBody, Name};
use morphir_core::ir::layout::{
    AnyTree, Profile, Tree, TreePolicy, read_any_tree, read_tree, read_tree_v3, write_tree_v3,
};

fn fixture(name: &str) -> Distribution {
    let text = match name {
        "greeting" => include_str!("fixtures/ir/classic/greeting-example.json"),
        _ => include_str!("fixtures/ir/classic/v3-with-dependencies.json"),
    };
    serde_json::from_str(text).unwrap()
}

fn policy(profile: Profile, budget: u32) -> TreePolicy {
    TreePolicy {
        profile,
        path_budget: budget,
    }
}

fn tree(files: Vec<(String, String)>) -> Tree {
    files.into_iter().collect()
}

/// A file of the tree, parsed through the profile's own reader.
fn parsed(profile: Profile, text: &str) -> serde_json::Value {
    profile.read(text).unwrap()
}

/// Module members come back in path order, so compare with members sorted by name.
fn sorted(mut file: Distribution) -> Distribution {
    let sort_def = |modules: &mut Vec<morphir_core::ir::classic::ModuleEntry<_, _>>| {
        modules.sort_by(|a, b| format!("{:?}", a.path).cmp(&format!("{:?}", b.path)));
        for module in modules.iter_mut() {
            module
                .definition
                .value
                .types
                .sort_by(|a, b| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)));
            module
                .definition
                .value
                .values
                .sort_by(|a, b| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)));
        }
    };
    if let DistributionBody::Library(_, deps, def) = &mut file.distribution {
        sort_def(&mut def.modules);
        for (_, spec) in deps.iter_mut() {
            for module in spec.modules.iter_mut() {
                module
                    .specification
                    .types
                    .sort_by(|a, b| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)));
                module
                    .specification
                    .values
                    .sort_by(|a, b| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)));
            }
        }
    }
    file
}

const SPECS: &str = r#"{"formatVersion":"3.1.0","distribution":["Specs",[["my"],["pkg"]],[],{"modules":[[[["basics"]],{"types":[[["int"],{"doc":"","value":["OpaqueTypeSpecification",[]]}]],"values":[],"doc":"Basics."}]]}]}"#;

#[test]
fn a_v3_library_round_trips_through_json_and_yaml_trees() {
    for name in ["greeting", "deps"] {
        for profile in [Profile::Json, Profile::Yaml] {
            let original = fixture(name);
            let files = write_tree_v3(&original, &policy(profile, 4000)).unwrap();
            let (read, warnings) = read_tree_v3(&tree(files), profile).unwrap();
            assert!(warnings.is_empty());
            assert_eq!(sorted(read), sorted(original), "{name} {profile:?}");
        }
    }
}

#[test]
fn every_v3_tree_file_says_3_1_0() {
    for profile in [Profile::Json, Profile::Yaml] {
        let files = write_tree_v3(&fixture("deps"), &policy(profile, 4000)).unwrap();
        for (path, text) in &files {
            assert_eq!(
                parsed(profile, text)["formatVersion"],
                serde_json::json!("3.1.0"),
                "{path}: {text}"
            );
        }
        assert!(
            files
                .iter()
                .any(|(path, _)| path == "deps/morphir/_sdk/@/basics/money.type")
        );
    }
}

#[test]
fn a_cut_stem_is_recorded_and_read_back() {
    // No name in the fixture is long enough to be cut at the smallest budget, so one is added.
    let mut original = fixture("deps");
    if let DistributionBody::Library(_, deps, _) = &mut original.distribution {
        let basics = &mut deps[0].1.modules[0].specification;
        let int = basics.types[0].1.clone();
        let long = Name::new([
            "a", "type", "whose", "name", "is", "far", "too", "long", "for", "the", "budget",
        ]);
        basics.types.push((long, int));
    }
    let files = write_tree_v3(&original, &policy(Profile::Json, 64)).unwrap();
    assert!(
        files
            .iter()
            .any(|(_, text)| parsed(Profile::Json, text).get("fileNames").is_some()),
        "{files:#?}"
    );
    let (read, _) = read_tree_v3(&tree(files), Profile::Json).unwrap();
    assert_eq!(sorted(read), sorted(original));
}

#[test]
fn a_node_file_whose_version_differs_from_the_manifest_is_refused() {
    let mut files =
        tree(write_tree_v3(&fixture("greeting"), &policy(Profile::Json, 4000)).unwrap());
    let (path, text) = files
        .iter()
        .find(|(path, _)| path.ends_with(".type"))
        .map(|(p, t)| (p.clone(), t.clone()))
        .unwrap();
    let mut node = parsed(Profile::Json, &text);
    node["formatVersion"] = serde_json::json!(3);
    files.insert(path.clone(), Profile::Json.write(&node));
    let error = read_tree_v3(&files, Profile::Json).unwrap_err();
    assert_eq!(error.code, DiagnosticCode::VersionMismatch, "{error:?}");
    assert_eq!(error.cursor, format!("{path}#/formatVersion"), "{error:?}");
}

/// A v4 `Library` tree with one empty module.
fn v4_tree() -> Tree {
    [
        (
            "manifest",
            r#"{ "formatVersion": 4, "distribution": "Library", "package": "example", "pathBudget": 4000 }"#,
        ),
        (
            "pkg/example/main/module",
            r#"{ "formatVersion": 4, "path": "main", "types": [], "values": [] }"#,
        ),
    ]
    .into_iter()
    .map(|(path, text)| (path.to_owned(), text.to_owned()))
    .collect()
}

#[test]
fn a_v4_reader_refuses_a_v3_tree_and_the_reverse() {
    let v3 = tree(write_tree_v3(&fixture("greeting"), &policy(Profile::Json, 4000)).unwrap());
    assert!(read_tree(&v3, Profile::Json).is_err());

    let v4 = v4_tree();
    read_tree(&v4, Profile::Json).expect("the v4 tree is a v4 tree");
    let error = read_tree_v3(&v4, Profile::Json).unwrap_err();
    assert_eq!(error.code, DiagnosticCode::VersionMismatch, "{error:?}");
    assert_eq!(error.cursor, "manifest#/formatVersion", "{error:?}");
}

#[test]
fn any_tree_reads_either_version() {
    let original = fixture("deps");
    let v3 = tree(write_tree_v3(&original, &policy(Profile::Yaml, 4000)).unwrap());
    match read_any_tree(&v3, Profile::Yaml).unwrap().0 {
        AnyTree::V3(read) => assert_eq!(sorted(read), sorted(original)),
        AnyTree::V4(_) => panic!("a v3 tree read as v4"),
    }
    match read_any_tree(&v4_tree(), Profile::Json).unwrap().0 {
        AnyTree::V4(_) => {}
        AnyTree::V3(_) => panic!("a v4 tree read as v3"),
    }
}

#[test]
fn a_dependency_named_like_the_distribution_is_refused() {
    let mut files = tree(write_tree_v3(&fixture("deps"), &policy(Profile::Json, 4000)).unwrap());
    let mut manifest = parsed(Profile::Json, &files["manifest"]);
    assert_eq!(manifest["dependencies"], serde_json::json!(["morphir/SDK"]));
    manifest["dependencies"] = serde_json::json!(["example"]);
    files.insert("manifest".into(), Profile::Json.write(&manifest));
    let moved: Vec<(String, String)> = files
        .iter()
        .filter(|(path, _)| path.starts_with("deps/morphir/_sdk/@/"))
        .map(|(path, text)| {
            (
                path.replace("deps/morphir/_sdk/@/", "deps/example/@/"),
                text.clone(),
            )
        })
        .collect();
    files.retain(|path, _| !path.starts_with("deps/"));
    files.extend(moved);
    let error = read_tree_v3(&files, Profile::Json).unwrap_err();
    assert!(
        error
            .message
            .contains("a v3 dependency cannot name the distribution package"),
        "{error:?}"
    );
}

#[test]
fn a_specs_tree_with_a_definition_file_is_refused() {
    let specs: Distribution = serde_json::from_str(r#"{"formatVersion":"3.1.0","distribution":["Specs",[["my"],["pkg"]],[],{"modules":[[[["basics"]],{"types":[[["int"],{"doc":"","value":["OpaqueTypeSpecification",[]]}]],"values":[],"doc":null}]]}]}"#).unwrap();
    let mut files = tree(write_tree_v3(&specs, &policy(Profile::Json, 4000)).unwrap());
    files.insert(
        "pkg/my/pkg/basics/int.type".into(),
        r#"{"formatVersion":"3.1.0","name":"int","def":{"access":"Public","value":{"doc":"","value":["TypeAliasDefinition",[],["Unit",{}]]}}}"#.into(),
    );
    let error = read_tree_v3(&files, Profile::Json).unwrap_err();
    assert!(
        format!("{error:?}").contains("expected a specification file"),
        "{error:?}"
    );
}

#[test]
fn a_specification_payload_with_a_member_it_does_not_hold_is_refused() {
    let mut files = tree(
        write_tree_v3(
            &serde_json::from_str(SPECS).unwrap(),
            &policy(Profile::Json, 4000),
        )
        .unwrap(),
    );
    let path = "pkg/my/pkg/basics/int.type";
    let mut node = parsed(Profile::Json, &files[path]);
    node["spec"]["annotations"] = serde_json::json!([]);
    files.insert(path.into(), Profile::Json.write(&node));
    let error = read_tree_v3(&files, Profile::Json).unwrap_err();
    assert_eq!(error.code, DiagnosticCode::UnknownMember, "{error:?}");
    assert_eq!(
        error.cursor,
        format!("{path}#/spec/annotations"),
        "{error:?}"
    );
}

#[test]
fn a_v3_specs_distribution_round_trips_through_a_tree() {
    let specs: Distribution = serde_json::from_str(SPECS).unwrap();
    let files = write_tree_v3(&specs, &policy(Profile::Yaml, 4000)).unwrap();
    assert_eq!(read_tree_v3(&tree(files), Profile::Yaml).unwrap().0, specs);
}

#[test]
fn a_private_module_keeps_its_access() {
    let mut original = fixture("deps");
    if let DistributionBody::Library(_, _, def) = &mut original.distribution {
        def.modules[0].definition.access = Access::Private;
    }
    let files = write_tree_v3(&original, &policy(Profile::Json, 4000)).unwrap();
    let module = files
        .iter()
        .find(|(path, _)| path == "pkg/example/eligibility/module")
        .map(|(_, text)| parsed(Profile::Json, text))
        .unwrap();
    assert_eq!(module["access"], serde_json::json!("Private"));
    let (read, _) = read_tree_v3(&tree(files), Profile::Json).unwrap();
    assert_eq!(sorted(read), sorted(original));
}

#[test]
fn a_module_file_whose_version_differs_from_the_manifest_is_refused() {
    let mut files = tree(write_tree_v3(&fixture("deps"), &policy(Profile::Yaml, 4000)).unwrap());
    let path = "deps/morphir/_sdk/@/basics/module";
    let mut module = parsed(Profile::Yaml, &files[path]);
    module["formatVersion"] = serde_json::json!("4.0.0");
    files.insert(path.into(), Profile::Yaml.write(&module));
    let error = read_tree_v3(&files, Profile::Yaml).unwrap_err();
    assert_eq!(error.code, DiagnosticCode::VersionMismatch, "{error:?}");
    assert_eq!(error.cursor, format!("{path}#/formatVersion"), "{error:?}");
}
