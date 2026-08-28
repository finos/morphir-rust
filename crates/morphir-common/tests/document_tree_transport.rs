use std::io::Write;

use morphir_common::ir_transport::{
    CodecOptions, FormatId, IrVersion, Layout, discover_document_tree_format, read_document_tree,
    read_document_tree_with_options, write_document_tree, write_document_tree_with_options,
};
use morphir_common::vfs::{memory_root, physical_root};
use morphir_core::ir::classic;
use morphir_core::ir::v4::Distribution;
use morphir_core::migration::{MigrationOptions, migrate_distribution};
use morphir_core::naming::PackageName;

fn fixture() -> morphir_core::ir::v4::IRFile {
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ClassicFile {
        format_version: u32,
        distribution: serde_json::Value,
    }

    let source = serde_json::to_value(ClassicFile {
        format_version: 3,
        distribution: serde_json::json!([
            "Library",
            [["document", "tree", "fixture"]],
            [],
            {
                "modules": [
                    [
                        [["first", "module"]],
                        {
                            "access": "Public",
                            "value": { "types": [], "values": [], "doc": "first" }
                        }
                    ],
                    [
                        [["second", "module"]],
                        {
                            "access": "Public",
                            "value": { "types": [], "values": [], "doc": "second" }
                        }
                    ]
                ]
            }
        ]),
    })
    .unwrap();
    let classic: classic::Distribution = serde_json::from_value(source).unwrap();
    let migrated = migrate_distribution(&classic, MigrationOptions::default())
        .unwrap()
        .value;
    serde_json::from_value(serde_json::to_value(migrated).unwrap()).unwrap()
}

fn granular_fixture() -> morphir_core::ir::v4::IRFile {
    serde_json::from_str(include_str!(
        "../../morphir-core/tests/fixtures/ir/v4/complete-example.json"
    ))
    .unwrap()
}

fn assert_round_trip(root: vfs::VfsPath) {
    let expected = fixture();
    write_document_tree(&root, &expected).unwrap();

    assert!(root.join("manifest.json").unwrap().is_file().unwrap());
    assert!(
        root.walk_dir()
            .unwrap()
            .filter_map(Result::ok)
            .any(|path| path.filename() == "module.json")
    );
    assert_eq!(read_document_tree(&root).unwrap(), expected);
}

fn retain_first_module(ir: &mut morphir_core::ir::v4::IRFile) {
    let Distribution::Library(content) = &mut ir.distribution else {
        panic!("test fixture must be a library");
    };
    let first = content
        .def
        .modules
        .first()
        .map(|(name, module)| (name.clone(), module.clone()))
        .unwrap();
    content.def.modules.clear();
    content.def.modules.insert(first.0, first.1);
}

fn rename_package(ir: &mut morphir_core::ir::v4::IRFile, package: &str) {
    let Distribution::Library(content) = &mut ir.distribution else {
        panic!("test fixture must be a library");
    };
    content.package_name = PackageName::parse(package);
}

#[test]
fn v4_document_tree_round_trips_on_memory_vfs() {
    assert_round_trip(memory_root());
}

#[test]
fn v4_document_tree_round_trips_on_physical_vfs() {
    let temp = tempfile::tempdir().unwrap();
    assert_round_trip(physical_root(temp.path()));
}

#[test]
fn rewriting_a_package_removes_stale_modules() {
    let root = memory_root();
    write_document_tree(&root, &fixture()).unwrap();
    let mut replacement = fixture();
    retain_first_module(&mut replacement);

    write_document_tree(&root, &replacement).unwrap();

    assert_eq!(read_document_tree(&root).unwrap(), replacement);
}

#[test]
fn reading_a_tree_ignores_modules_from_other_packages() {
    let root = memory_root();
    write_document_tree(&root, &fixture()).unwrap();
    let mut current = fixture();
    retain_first_module(&mut current);
    rename_package(&mut current, "another/package");

    write_document_tree(&root, &current).unwrap();

    assert_eq!(read_document_tree(&root).unwrap(), current);
}

#[test]
fn yaml_tree_uses_only_yaml_physical_names() {
    let root = memory_root();
    let expected = granular_fixture();
    let options = CodecOptions::new(IrVersion::V4, Layout::DocumentTree, FormatId::yaml());

    write_document_tree_with_options(&root, &expected, &options).unwrap();

    assert!(root.join("manifest.yaml").unwrap().is_file().unwrap());
    let module = root
        .join("pkg/regulation/u-s/f-r-2052-a/data-tables")
        .unwrap();
    assert!(module.join("module.yaml").unwrap().is_file().unwrap());
    assert!(
        module
            .join("data-tables.type.yaml")
            .unwrap()
            .is_file()
            .unwrap()
    );
    assert!(
        module
            .join("calculate-total.value.yaml")
            .unwrap()
            .is_file()
            .unwrap()
    );
    assert!(
        root.walk_dir()
            .unwrap()
            .filter_map(Result::ok)
            .all(|path| !path.filename().ends_with(".json"))
    );
    assert_eq!(
        read_document_tree_with_options(&root, &options).unwrap(),
        expected
    );
}

#[test]
fn discovery_rejects_ambiguous_tree_manifests() {
    let root = memory_root();
    root.create_dir_all().unwrap();
    for name in ["manifest.json", "manifest.yaml"] {
        let mut writer = root.join(name).unwrap().create_file().unwrap();
        writer.write_all(b"{}").unwrap();
    }

    let diagnostic = discover_document_tree_format(&root).unwrap_err();

    assert_eq!(
        diagnostic.code(),
        "morphir::ir::detection::ambiguous_manifest"
    );
}
