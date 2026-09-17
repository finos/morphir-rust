use std::io::Write;

use morphir_common::ir_transport::{
    CodecOptions, DocumentTreeSink, EventSink, FormatId, IrCodec, IrVersion, JsonCodec, Layout,
    TransportDiagnostic, discover_document_tree_format, read_document_tree,
    read_document_tree_with_options, write_document_tree, write_document_tree_with_options,
};
use morphir_common::vfs::{memory_root, physical_root};
use morphir_core::ir::classic;
use morphir_core::ir::v4::Distribution;
use morphir_core::migration::{MigrationOptions, migrate_distribution};
use morphir_core::naming::PackageName;
use morphir_core::traversal::{DependencyEvent, SemanticEvent, SemanticEventKind};

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
fn a_module_manifest_accepts_an_array_of_lines_for_doc_and_writes_one_string() {
    let root = memory_root();
    write_document_tree(&root, &fixture()).unwrap();

    let module_manifest = root
        .walk_dir()
        .unwrap()
        .filter_map(Result::ok)
        .find(|path| path.filename() == "module.json")
        .unwrap();
    let mut raw: serde_json::Value =
        serde_json::from_reader(module_manifest.open_file().unwrap()).unwrap();
    let module_path = raw["path"].as_str().unwrap().to_owned();
    raw["doc"] = serde_json::json!(["line one", "line two"]);
    let mut writer = module_manifest.create_file().unwrap();
    writer
        .write_all(serde_json::to_vec(&raw).unwrap().as_slice())
        .unwrap();
    drop(writer);

    let read = read_document_tree(&root).unwrap();
    let Distribution::Library(content) = &read.distribution else {
        panic!("test fixture must be a library");
    };
    let module = content.def.modules.get(&module_path).unwrap();
    assert_eq!(
        module.value.doc.as_ref().unwrap().text(),
        "line one\nline two"
    );

    write_document_tree(&root, &read).unwrap();
    let rewritten: serde_json::Value =
        serde_json::from_reader(module_manifest.open_file().unwrap()).unwrap();
    assert_eq!(rewritten["doc"], serde_json::json!("line one\nline two"));
}

#[derive(Default)]
struct CollectingSink {
    events: Vec<SemanticEvent>,
}

impl EventSink for CollectingSink {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        self.events.push(event);
        Ok(())
    }
}

fn decode_json(input: &str) -> Vec<SemanticEvent> {
    let codec = JsonCodec::new();
    let options = CodecOptions::new(IrVersion::V4, Layout::SingleFile, FormatId::json());
    let mut reader = std::io::Cursor::new(input.as_bytes());
    let mut sink = CollectingSink::default();
    codec.decode(&mut reader, &options, &mut sink).unwrap();
    sink.events
}

/// An application header with no dependencies of its own; used to isolate the header event.
const APPLICATION_HEADER_ONLY: &str = r#"{"formatVersion":4,"distribution":{"Application":{"packageName":"example","dependencies":{},"def":{"modules":{}},"entryPoints":{}}}}"#;

/// A library distribution whose dependency is a package specification, as libraries require.
const LIBRARY_WITH_SPECIFICATION_DEPENDENCY: &str = r#"{"formatVersion":4,"distribution":{"Library":{"packageName":"example","dependencies":{"my-org/shared":{"modules":{}}},"def":{"modules":{}}}}}"#;

#[test]
fn document_tree_sink_refuses_a_specification_dependency_under_an_application_header() {
    let application_begin = decode_json(APPLICATION_HEADER_ONLY)
        .into_iter()
        .find(|event| matches!(event.kind(), SemanticEventKind::Begin(_)))
        .unwrap();
    let specification_dependency = decode_json(LIBRARY_WITH_SPECIFICATION_DEPENDENCY)
        .into_iter()
        .find(|event| {
            matches!(
                event.kind(),
                SemanticEventKind::Dependency(DependencyEvent::V4 { .. })
            )
        })
        .unwrap();

    let root = memory_root();
    let options = CodecOptions::new(IrVersion::V4, Layout::DocumentTree, FormatId::json());
    let mut sink = DocumentTreeSink::new(root, options).unwrap();
    sink.accept(application_begin).unwrap();
    let diagnostic = sink.accept(specification_dependency).unwrap_err();

    assert_eq!(
        diagnostic.code(),
        "morphir::ir::document_tree::unsupported_dependencies"
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
