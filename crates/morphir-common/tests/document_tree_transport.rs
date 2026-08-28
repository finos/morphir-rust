use morphir_common::ir_transport::{read_document_tree, write_document_tree};
use morphir_common::vfs::{memory_root, physical_root};
use morphir_core::ir::classic;
use morphir_core::migration::{MigrationOptions, migrate_distribution};

fn fixture() -> morphir_core::ir::v4::IRFile {
    let classic: classic::Distribution = serde_json::from_str(include_str!(
        "../../../../../website/static/ir/examples/v3/greeting-example.json"
    ))
    .unwrap();
    let migrated = migrate_distribution(&classic, MigrationOptions::default())
        .unwrap()
        .value;
    serde_json::from_value(serde_json::to_value(migrated).unwrap()).unwrap()
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

#[test]
fn v4_document_tree_round_trips_on_memory_vfs() {
    assert_round_trip(memory_root());
}

#[test]
fn v4_document_tree_round_trips_on_physical_vfs() {
    let temp = tempfile::tempdir().unwrap();
    assert_round_trip(physical_root(temp.path()));
}
