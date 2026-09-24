//! The Ion document tree: the single-file annotated elements, one file per path.
//!
//! A tree file holds the same annotated elements as a single-file datagram. The path supplies the
//! package, the module, and the node name, so a file may omit them. A name that is present must
//! match the path. A tree reads as the datagram that holds the same elements in path order.

use std::collections::VecDeque;
use std::io::{Cursor, Write};

use morphir_common::ir_transport::{
    CodecOptions, DocumentTreeSink, DocumentTreeSource, EventSink, EventSource, FormatId, IonCodec,
    IrCodec, IrVersion, JsonCodec, Layout, TransportDiagnostic, discover_document_tree_format,
    read_document_tree_with_options, write_document_tree_with_options,
};
use morphir_common::vfs::{memory_root, physical_root};
use morphir_core::ir::v4::{Distribution, FormatVersion, IRFile};
use morphir_core::traversal::SemanticEvent;
use vfs::VfsPath;

// =============================================================================
// Helpers
// =============================================================================

fn tree_options(version: IrVersion) -> CodecOptions {
    CodecOptions::new(version, Layout::DocumentTree, FormatId::ion())
}

fn single_options(version: IrVersion, format: FormatId) -> CodecOptions {
    CodecOptions::new(version, Layout::SingleFile, format)
}

/// Collects the events a codec or a tree source produces.
#[derive(Default)]
struct Collect(Vec<SemanticEvent>);

impl EventSink for Collect {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        self.0.push(event);
        Ok(())
    }
}

struct Replay(VecDeque<SemanticEvent>);

impl EventSource for Replay {
    fn next_event(&mut self) -> Result<Option<SemanticEvent>, TransportDiagnostic> {
        Ok(self.0.pop_front())
    }
}

fn decode(
    codec: &dyn IrCodec,
    text: &str,
    options: &CodecOptions,
) -> Result<Vec<SemanticEvent>, TransportDiagnostic> {
    let mut sink = Collect::default();
    codec.decode(&mut Cursor::new(text.as_bytes()), options, &mut sink)?;
    Ok(sink.0)
}

fn write_events(root: &VfsPath, version: IrVersion, events: Vec<SemanticEvent>) {
    let mut sink = DocumentTreeSink::new(root.clone(), tree_options(version)).unwrap();
    let mut source = Replay(events.into());
    while let Some(event) = source.next_event().unwrap() {
        sink.accept(event).unwrap();
    }
    sink.finish().unwrap();
}

fn read_events(
    root: &VfsPath,
    version: IrVersion,
) -> Result<Vec<SemanticEvent>, TransportDiagnostic> {
    let mut source = DocumentTreeSource::open(root.clone(), tree_options(version))?;
    let mut events = Vec::new();
    while let Some(event) = source.next_event()? {
        events.push(event);
    }
    Ok(events)
}

/// Writes each `(physical path, text)` under `root`.
fn tree(files: &[(&str, &str)]) -> VfsPath {
    let root = memory_root();
    for (path, text) in files {
        let file = root.join(path).unwrap();
        file.parent().create_dir_all().unwrap();
        file.create_file()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }
    root
}

fn read_file(root: &VfsPath, path: &str) -> String {
    root.join(path).unwrap().read_to_string().unwrap()
}

fn every_file(root: &VfsPath) -> Vec<String> {
    let mut paths: Vec<String> = root
        .walk_dir()
        .unwrap()
        .filter_map(Result::ok)
        .filter(|path| path.is_file().unwrap())
        .map(|path| path.as_str().trim_start_matches('/').to_owned())
        .collect();
    paths.sort();
    paths
}

/// Ion stores `formatVersion` as the release string; JSON may store the same release as `4`.
fn release_string(mut file: IRFile) -> IRFile {
    if file.format_version == FormatVersion::Integer(4) {
        file.format_version = FormatVersion::String("4.0.0".to_owned());
    }
    file
}

fn v4_fixture(text: &str) -> IRFile {
    serde_json::from_str(text).unwrap()
}

const COMPLETE_EXAMPLE: &str =
    include_str!("../../morphir-core/tests/fixtures/ir/v4/complete-example.json");
const V4_LIBRARY: &str =
    include_str!("../../morphir-core/tests/fixtures/ir/v4/v4-library-distribution.json");
const GREETING: &str =
    include_str!("../../morphir-core/tests/fixtures/ir/classic/greeting-example.json");

// =============================================================================
// Round trips
// =============================================================================

fn assert_v4_round_trip(root: VfsPath, fixture: &str) {
    let expected = release_string(v4_fixture(fixture));
    let options = tree_options(IrVersion::V4);

    write_document_tree_with_options(&root, &expected, &options).unwrap();

    assert!(root.join("manifest.ion").unwrap().is_file().unwrap());
    assert_eq!(
        discover_document_tree_format(&root).unwrap(),
        FormatId::ion()
    );
    let read = read_document_tree_with_options(&root, &options).unwrap_or_else(|error| {
        panic!("{error:?}\n{:#?}", every_file(&root));
    });
    assert_eq!(release_string(read), expected);
}

#[test]
fn the_complete_v4_example_round_trips_through_an_ion_tree() {
    assert_v4_round_trip(memory_root(), COMPLETE_EXAMPLE);
}

#[test]
fn a_v4_library_round_trips_through_an_ion_tree_on_disk() {
    let temp = tempfile::tempdir().unwrap();
    assert_v4_round_trip(physical_root(temp.path()), V4_LIBRARY);
}

#[test]
fn the_v3_greeting_example_round_trips_through_an_ion_tree() {
    let original = decode(
        &JsonCodec::new(),
        GREETING,
        &single_options(IrVersion::V3, FormatId::json()),
    )
    .unwrap();
    let root = memory_root();

    write_events(&root, IrVersion::V3, original.clone());

    let read = read_events(&root, IrVersion::V3)
        .unwrap_or_else(|error| panic!("{error:?}\n{:#?}", every_file(&root)));
    // A tree orders the members of a module by path, as it orders modules.
    assert_eq!(sorted_v3(read), sorted_v3(original));
}

/// The v3 JSON of `events`, with each module's types and values sorted by name.
fn sorted_v3(events: Vec<SemanticEvent>) -> serde_json::Value {
    let mut json = Vec::new();
    {
        let mut sink = JsonCodec::new()
            .encoder(&mut json, &single_options(IrVersion::V3, FormatId::json()))
            .unwrap();
        for event in events {
            sink.accept(event).unwrap();
        }
        sink.finish().unwrap();
    }
    let mut value: serde_json::Value = serde_json::from_slice(&json).unwrap();
    let modules = value["distribution"][3]["modules"].as_array_mut().unwrap();
    for module in modules {
        let definition = &mut module[1]["value"];
        for members in ["types", "values"] {
            if let Some(list) = definition[members].as_array_mut() {
                list.sort_by_key(|entry| entry[0].to_string());
            }
        }
    }
    value
}

// =============================================================================
// What the writer emits
// =============================================================================

#[test]
fn a_written_v4_tree_uses_one_file_per_node_and_omits_path_names() {
    let root = memory_root();
    write_document_tree_with_options(
        &root,
        &v4_fixture(COMPLETE_EXAMPLE),
        &tree_options(IrVersion::V4),
    )
    .unwrap();

    let files = every_file(&root);
    assert!(files.contains(&"manifest.ion".to_owned()), "{files:#?}");
    assert!(
        files.contains(&"deps/morphir/_sdk/@/basics/int.type.ion".to_owned()),
        "{files:#?}"
    );
    assert!(
        files.contains(&"deps/morphir/_sdk/@/basics/add.value.ion".to_owned()),
        "{files:#?}"
    );

    let manifest = read_file(&root, "manifest.ion");
    assert!(manifest.contains("morphir::"), "{manifest}");
    assert!(manifest.contains("pathBudget"), "{manifest}");
    assert!(!manifest.contains("morphir_footer"), "{manifest}");

    let int = read_file(&root, "deps/morphir/_sdk/@/basics/int.type.ion");
    assert!(int.contains("public::spec::opaque::type::"), "{int}");
    assert!(!int.contains("name"), "{int}");

    for module in files.iter().filter(|path| path.ends_with("/module.ion")) {
        let text = read_file(&root, module);
        assert!(!text.contains("name:"), "{module}: {text}");
        assert!(!text.contains("types:"), "{module}: {text}");
        assert!(!text.contains("values:"), "{module}: {text}");
    }
}

#[test]
fn a_written_v3_tree_holds_the_module_header_and_one_file_per_node() {
    let original = decode(
        &JsonCodec::new(),
        GREETING,
        &single_options(IrVersion::V3, FormatId::json()),
    )
    .unwrap();
    let root = memory_root();

    write_events(&root, IrVersion::V3, original);

    let files = every_file(&root);
    let manifest = read_file(&root, "manifest.ion");
    assert!(manifest.contains(r#"formatVersion: "3.0.0""#), "{manifest}");
    assert!(
        files.iter().any(|path| path.ends_with(".value.ion")),
        "{files:#?}"
    );
    assert!(
        files.iter().all(|path| !path.starts_with("deps/")),
        "{files:#?}"
    );
}

#[test]
fn a_small_path_budget_keeps_the_name_of_a_truncated_stem() {
    let root = memory_root();
    let expected = release_string(v4_fixture(COMPLETE_EXAMPLE));
    let options = tree_options(IrVersion::V4).with_path_budget(64);

    write_document_tree_with_options(&root, &expected, &options).unwrap();

    let truncated: Vec<String> = every_file(&root)
        .into_iter()
        .filter(|path| path.contains("__"))
        .collect();
    assert!(!truncated.is_empty(), "{:#?}", every_file(&root));
    for path in &truncated {
        let text = read_file(&root, path);
        assert!(text.contains("name:"), "{path}: {text}");
    }
    assert_eq!(
        release_string(read_document_tree_with_options(&root, &options).unwrap()),
        expected
    );
}

// =============================================================================
// Hand-written trees read as the equivalent datagram
// =============================================================================

const V4_MANIFEST: &str = r#"
morphir::{
  ionVersion: "0.1.0-draft.1",
  formatVersion: "4.0.0",
  kind: library,
  packageName: "example/finance",
  pathBudget: 4000,
}
"#;

const V4_EQUIVALENT_DATAGRAM: &str = r#"
morphir::{
  ionVersion: "0.1.0-draft.1",
  formatVersion: "4.0.0",
  kind: library,
  packageName: "example/finance",
}
package::spec::{
  name: "morphir/SDK",
  modules: [
    module::spec::{
      name: "basics",
      types: [ public::spec::opaque::type::{ name: "int" } ],
    },
  ],
}
public::def::module::{
  name: "eligibility",
  doc: "Credit eligibility.",
  types: [
    public::def::alias::type::{ name: "decision", typeExp: "morphir/SDK:basics#int" },
    public::def::alias::type::{ name: "score", typeExp: "morphir/SDK:basics#int" },
  ],
}
morphir_footer::{}
"#;

fn read_v4(root: &VfsPath) -> Result<IRFile, TransportDiagnostic> {
    read_document_tree_with_options(root, &tree_options(IrVersion::V4))
}

fn v4_datagram(text: &str) -> IRFile {
    let events = decode(
        &IonCodec::new(),
        text,
        &single_options(IrVersion::V4, FormatId::ion()),
    )
    .unwrap();
    let mut json = Vec::new();
    {
        let mut sink = JsonCodec::new()
            .encoder(&mut json, &single_options(IrVersion::V4, FormatId::json()))
            .unwrap();
        for event in events {
            sink.accept(event).unwrap();
        }
        sink.finish().unwrap();
    }
    release_string(serde_json::from_slice(&json).unwrap())
}

#[test]
fn a_module_file_may_hold_its_children_beside_node_files() {
    let root = tree(&[
        ("manifest.ion", V4_MANIFEST),
        (
            "pkg/example/finance/eligibility/module.ion",
            r#"
public::def::module::{ doc: "Credit eligibility." }
public::def::alias::type::{ name: "decision", typeExp: "morphir/SDK:basics#int" }
"#,
        ),
        (
            "pkg/example/finance/eligibility/score.type.ion",
            r#"public::def::alias::type::{ typeExp: "morphir/SDK:basics#int" }"#,
        ),
        ("deps/morphir/_sdk/@/basics/module.ion", "module::spec::{}"),
        (
            "deps/morphir/_sdk/@/basics/int.type.ion",
            "public::spec::opaque::type::{}",
        ),
    ]);

    let read = read_v4(&root).unwrap();

    assert_eq!(read, v4_datagram(V4_EQUIVALENT_DATAGRAM));
}

#[test]
fn a_name_that_matches_its_path_is_accepted() {
    let root = tree(&[
        ("manifest.ion", V4_MANIFEST),
        (
            "pkg/example/finance/eligibility/module.ion",
            r#"public::def::module::{ name: "eligibility", package: "example/finance", doc: "Credit eligibility." }
public::def::alias::type::{ name: "decision", typeExp: "morphir/SDK:basics#int" }"#,
        ),
        (
            "pkg/example/finance/eligibility/score.type.ion",
            r#"public::def::alias::type::{ name: "score", module: "eligibility", typeExp: "morphir/SDK:basics#int" }"#,
        ),
        (
            "deps/morphir/_sdk/@/basics/module.ion",
            r#"module::spec::{ name: "basics", package: "morphir/SDK" }"#,
        ),
        (
            "deps/morphir/_sdk/@/basics/int.type.ion",
            r#"public::spec::opaque::type::{ name: "int" }"#,
        ),
    ]);

    assert_eq!(read_v4(&root).unwrap(), v4_datagram(V4_EQUIVALENT_DATAGRAM));
}

fn assert_refused(root: &VfsPath, fragment: &str) {
    let error = read_v4(root).expect_err("the tree is refused");
    let text = format!("{error:?}");
    assert!(text.contains(fragment), "expected '{fragment}' in {text}");
}

#[test]
fn a_name_that_disagrees_with_its_path_is_refused() {
    let root = tree(&[
        ("manifest.ion", V4_MANIFEST),
        (
            "pkg/example/finance/eligibility/module.ion",
            "public::def::module::{}",
        ),
        (
            "pkg/example/finance/eligibility/score.type.ion",
            r#"public::def::alias::type::{ name: "rating", typeExp: "morphir/SDK:basics#int" }"#,
        ),
    ]);

    assert_refused(&root, "score.type");
}

#[test]
fn a_module_name_that_disagrees_with_its_directory_is_refused() {
    let root = tree(&[
        ("manifest.ion", V4_MANIFEST),
        (
            "pkg/example/finance/eligibility/module.ion",
            r#"public::def::module::{ name: "pricing" }"#,
        ),
    ]);

    assert_refused(&root, "pricing");
}

#[test]
fn a_type_defined_twice_after_the_merge_is_refused() {
    let root = tree(&[
        ("manifest.ion", V4_MANIFEST),
        (
            "pkg/example/finance/eligibility/module.ion",
            r#"public::def::module::{}
public::def::alias::type::{ name: "score", typeExp: "morphir/SDK:basics#int" }"#,
        ),
        (
            "pkg/example/finance/eligibility/score.type.ion",
            r#"public::def::alias::type::{ typeExp: "morphir/SDK:basics#int" }"#,
        ),
    ]);

    assert_refused(&root, "duplicate_name");
}

#[test]
fn a_type_file_holds_one_type() {
    let root = tree(&[
        ("manifest.ion", V4_MANIFEST),
        (
            "pkg/example/finance/eligibility/module.ion",
            "public::def::module::{}",
        ),
        (
            "pkg/example/finance/eligibility/score.type.ion",
            r#"public::def::value::{ outputType: "morphir/SDK:basics#int", body: 1 }"#,
        ),
    ]);

    assert_refused(&root, "score.type");
}

#[test]
fn a_module_directory_without_a_module_file_is_refused() {
    let root = tree(&[
        ("manifest.ion", V4_MANIFEST),
        (
            "pkg/example/finance/eligibility/score.type.ion",
            r#"public::def::alias::type::{ typeExp: "morphir/SDK:basics#int" }"#,
        ),
    ]);

    assert_refused(&root, "eligibility/module");
}

#[test]
fn a_tree_file_has_no_footer() {
    let root = tree(&[(
        "manifest.ion",
        &format!("{V4_MANIFEST}\nmorphir_footer::{{}}"),
    )]);

    assert_refused(&root, "morphir_footer");
}

#[test]
fn an_ion_and_a_json_manifest_are_ambiguous() {
    let root = tree(&[("manifest.ion", V4_MANIFEST), ("manifest.json", "{}")]);

    let error = discover_document_tree_format(&root).unwrap_err();
    assert!(format!("{error:?}").contains("ambiguous_manifest"));
}

const V3_MANIFEST: &str = r#"
morphir::{
  formatVersion: "3.0.0",
  kind: library,
  packageName: "example",
}
"#;

const V3_EQUIVALENT_DATAGRAM: &str = r#"
morphir::{
  formatVersion: "3.0.0",
  kind: library,
  packageName: "example",
}
public::def::module::{
  name: "eligibility",
  types: [
    public::def::alias::type::{ name: "decision", typeExp: "morphir/SDK:basics#bool" },
    public::def::alias::type::{ name: "score", typeExp: "morphir/SDK:basics#int" },
  ],
  values: [
    public::def::value::{
      name: "approve",
      inputTypes: [ { name: "score", type: "morphir/SDK:basics#int" } ],
      outputType: "morphir/SDK:basics#bool",
      body: true,
    },
  ],
}
morphir_footer::{}
"#;

#[test]
fn a_v3_tree_reads_as_the_equivalent_datagram() {
    let root = tree(&[
        ("manifest.ion", V3_MANIFEST),
        (
            "pkg/example/eligibility/module.ion",
            r#"public::def::module::{}
public::def::alias::type::{ name: "decision", typeExp: "morphir/SDK:basics#bool" }"#,
        ),
        (
            "pkg/example/eligibility/score.type.ion",
            r#"public::def::alias::type::{ typeExp: "morphir/SDK:basics#int" }"#,
        ),
        (
            "pkg/example/eligibility/approve.value.ion",
            r#"public::def::value::{
  inputTypes: [ { name: "score", type: "morphir/SDK:basics#int" } ],
  outputType: "morphir/SDK:basics#bool",
  body: true,
}"#,
        ),
    ]);

    let expected = decode(
        &IonCodec::new(),
        V3_EQUIVALENT_DATAGRAM,
        &single_options(IrVersion::V3, FormatId::ion()),
    )
    .unwrap();
    assert_eq!(read_events(&root, IrVersion::V3).unwrap(), expected);
}

#[test]
fn a_tree_version_must_match_the_selected_version() {
    let root = tree(&[("manifest.ion", V3_MANIFEST)]);

    assert!(read_events(&root, IrVersion::V4).is_err());
}

#[test]
fn a_v4_library_keeps_every_module_of_the_fixture() {
    let root = memory_root();
    let expected = v4_fixture(COMPLETE_EXAMPLE);
    write_document_tree_with_options(&root, &expected, &tree_options(IrVersion::V4)).unwrap();
    let Distribution::Library(read) = read_v4(&root).unwrap().distribution else {
        panic!("a library");
    };
    let Distribution::Library(original) = expected.distribution else {
        panic!("a library");
    };
    assert_eq!(
        read.def.modules.keys().collect::<Vec<_>>(),
        original.def.modules.keys().collect::<Vec<_>>()
    );
}

// =============================================================================
// Names that look like tree files
// =============================================================================

/// A module, type, or value may be named `manifest` or `module`. The distribution manifest is only
/// ever the tree root's `manifest.ion`, and a module's own file is only ever the `module` leaf, so
/// neither name collides with them. `con` is a Windows device name and takes a `_` suffix.
const RESERVED_LOOKING: &str = r#"
morphir::{
  ionVersion: "0.1.0-draft.1",
  formatVersion: "4.0.0",
  kind: library,
  packageName: "example",
}
public::def::module::{
  name: "manifest",
  types: [
    public::def::alias::type::{ name: "manifest", typeExp: "morphir/SDK:basics#int" },
    public::def::alias::type::{ name: "module", typeExp: "morphir/SDK:basics#int" },
  ],
}
public::def::module::{
  name: "module",
  types: [ public::def::alias::type::{ name: "con", typeExp: "morphir/SDK:basics#int" } ],
}
public::def::module::{
  name: "manifest/module",
  types: [ public::def::alias::type::{ name: "manifest", typeExp: "morphir/SDK:basics#int" } ],
}
morphir_footer::{}
"#;

#[test]
fn modules_and_types_named_like_tree_files_round_trip() {
    let expected = v4_datagram(RESERVED_LOOKING);
    let root = memory_root();

    write_document_tree_with_options(&root, &expected, &tree_options(IrVersion::V4)).unwrap();

    let files = every_file(&root);
    for path in [
        "manifest.ion",
        "pkg/example/manifest/module.ion",
        "pkg/example/manifest/manifest.type.ion",
        "pkg/example/manifest/module.type.ion",
        "pkg/example/module/module.ion",
        "pkg/example/module/con_.type.ion",
        "pkg/example/manifest/module/module.ion",
        "pkg/example/manifest/module/manifest.type.ion",
    ] {
        assert!(files.contains(&path.to_owned()), "{path} in {files:#?}");
    }
    assert_eq!(read_v4(&root).unwrap(), expected);
}

#[test]
fn a_module_file_directly_under_the_package_is_refused() {
    let root = tree(&[
        ("manifest.ion", V4_MANIFEST),
        ("pkg/example/finance/module.ion", "public::def::module::{}"),
    ]);

    assert_refused(&root, "a module path has at least one name");
}

#[test]
fn a_v4_type_with_attributes_is_refused_rather_than_dropped() {
    let mut ir = v4_fixture(COMPLETE_EXAMPLE);
    let Distribution::Library(content) = &mut ir.distribution else {
        panic!("a library");
    };
    let module = content.def.modules.values_mut().next().unwrap();
    let value = module.value.values.values_mut().next().unwrap();
    let output = value
        .value
        .value
        .output_type
        .as_mut()
        .expect("an output type");
    let (morphir_core::ir::v4::Type::Reference(attributes, _, _)
    | morphir_core::ir::v4::Type::Variable(attributes, _)) = output
    else {
        panic!("the fixture's first output type is a reference or a variable");
    };
    attributes
        .extensions
        .insert("hint".to_owned(), serde_json::json!("kept"));

    let error = write_document_tree_with_options(&memory_root(), &ir, &tree_options(IrVersion::V4))
        .expect_err("the writer refuses attributes it cannot encode");

    assert!(format!("{error:?}").contains("attributes"), "{error:?}");
}
