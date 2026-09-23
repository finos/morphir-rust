//! Pins `morphir_core::ir::layout::paths` against the reference `IR/src/layout/paths.ts`, per
//! `.dev/docs/superpowers/maps/2026-09-17-reference-tree-layout-map.md` section 1.

use morphir_core::ir::layout::{
    PathKind, Profile, Root, classify, from_physical, module_dir, module_manifest_path,
    package_dir, to_physical,
};
use morphir_core::naming::{ModuleName, PackageName};
use serde::Deserialize;

#[test]
fn classify_reads_the_manifest_path() {
    assert_eq!(classify("manifest"), PathKind::Manifest);
}

#[test]
fn classify_reads_a_module_path() {
    assert_eq!(
        classify("pkg/a/module"),
        PathKind::Module {
            root: Root::Pkg,
            dir: "a".to_string(),
        }
    );
}

#[test]
fn classify_reads_a_type_path() {
    assert_eq!(
        classify("pkg/a/b/x.type"),
        PathKind::Type {
            root: Root::Pkg,
            dir: "a/b".to_string(),
            stem: "x".to_string(),
        }
    );
}

#[test]
fn classify_reads_a_value_path() {
    assert_eq!(
        classify("deps/a/@/b/x.value"),
        PathKind::Value {
            root: Root::Deps,
            dir: "a/@/b".to_string(),
            stem: "x".to_string(),
        }
    );
}

#[test]
fn classify_is_greedy_on_the_definition_leaf() {
    // `a.type.value` -> stem `a.type`, kind `value` (the reference's `DEFINITION_LEAF` is greedy).
    assert_eq!(
        classify("pkg/a/a.type.value"),
        PathKind::Value {
            root: Root::Pkg,
            dir: "a".to_string(),
            stem: "a.type".to_string(),
        }
    );
}

#[test]
fn classify_pinned_other_cases() {
    for path in [
        "",
        "README",
        "pkg",
        "pkg/a",
        "other/thing",
        "pkg/a/b/x.unknown",
    ] {
        assert_eq!(classify(path), PathKind::Other, "{path}");
    }
}

#[test]
fn from_physical_recognizes_every_known_extension() {
    assert_eq!(
        from_physical("pkg/a/module.json").as_deref(),
        Some("pkg/a/module")
    );
    assert_eq!(
        from_physical("pkg/a/module.yaml").as_deref(),
        Some("pkg/a/module")
    );
    assert_eq!(
        from_physical("pkg/a/module.yml").as_deref(),
        Some("pkg/a/module")
    );
    assert_eq!(
        from_physical("pkg/a/name.type.ion").as_deref(),
        Some("pkg/a/name.type")
    );
}

#[test]
fn from_physical_normalizes_backslashes() {
    assert_eq!(
        from_physical(r"pkg\a\module.json").as_deref(),
        Some("pkg/a/module")
    );
}

#[test]
fn from_physical_ignores_unknown_extensions() {
    assert_eq!(from_physical("notes.md"), None);
    assert_eq!(from_physical("session.jsonl"), None);
}

#[test]
fn to_physical_appends_the_profile_extension() {
    assert_eq!(
        to_physical("pkg/a/module", Profile::Json),
        "pkg/a/module.json"
    );
    assert_eq!(
        to_physical("pkg/a/module", Profile::Yaml),
        "pkg/a/module.yaml"
    );
    assert_eq!(
        to_physical("pkg/a/module", Profile::Ion),
        "pkg/a/module.ion"
    );
}

#[test]
fn package_and_module_dir_carry_the_deps_version_slot_after_the_package_path() {
    // package `a`, module `b/c` -> `deps/a/@/b/c`
    let pkg_a = PackageName::parse("a");
    let module_bc = ModuleName::parse("b/c");
    assert_eq!(package_dir(Root::Deps, &pkg_a), "a/@");
    assert_eq!(module_dir(Root::Deps, &pkg_a, module_bc.path()), "a/@/b/c");

    // package `a/b`, module `c` -> `a/b/@/c`
    let pkg_ab = PackageName::parse("a/b");
    let module_c = ModuleName::parse("c");
    assert_eq!(package_dir(Root::Deps, &pkg_ab), "a/b/@");
    assert_eq!(module_dir(Root::Deps, &pkg_ab, module_c.path()), "a/b/@/c");

    // `pkg/` never carries a version slot.
    assert_eq!(package_dir(Root::Pkg, &pkg_a), "a");
    assert_eq!(module_dir(Root::Pkg, &pkg_a, module_bc.path()), "a/b/c");
}

// =============================================================================
// Tied to the shared naming-conformance corpus, per the risk the reference map's section 1
// flags: `to_physical(module_manifest_path(...))` must equal the corpus's own
// `documentTreePath`/`dependencyTreePath` for every case.
// =============================================================================

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Corpus {
    qualified_module_name_cases: Vec<QualifiedModuleNameCase>,
}

#[derive(Debug, Deserialize)]
struct Canonical {
    uppercase: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QualifiedModuleNameCase {
    canonical: Canonical,
    document_tree_path: String,
    dependency_tree_path: String,
}

fn corpus() -> Corpus {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/naming-conformance.json"
    ))
    .expect("naming conformance fixture")
}

#[test]
fn layout_paths_agree_with_the_naming_corpus() {
    for case in corpus().qualified_module_name_cases {
        let (pkg, module) = case.canonical.uppercase.split_once(':').unwrap();
        let pkg = PackageName::from_canonical_string(pkg).unwrap();
        let module = ModuleName::from_canonical_string(module).unwrap();

        let pkg_dir = module_dir_for(Root::Pkg, &pkg, &module);
        let deps_dir = module_dir_for(Root::Deps, &pkg, &module);

        assert_eq!(
            to_physical(&module_manifest_path(Root::Pkg, &pkg_dir), Profile::Json),
            case.document_tree_path,
            "{}",
            case.canonical.uppercase
        );
        assert_eq!(
            to_physical(&module_manifest_path(Root::Deps, &deps_dir), Profile::Json),
            case.dependency_tree_path,
            "{}",
            case.canonical.uppercase
        );
    }
}

fn module_dir_for(root: Root, pkg: &PackageName, module: &ModuleName) -> String {
    module_dir(root, pkg, module.path())
}
