//! The document-tree transport as a filesystem adapter over `morphir_core::ir::layout`.
//!
//! The layout itself is pinned in the kit, against the Morphir Compatibility Kit's own tree cases;
//! what is pinned here is what the adapter adds — where a logical path lands on a filesystem, how a
//! tree root is discovered, what a rewrite removes, and which trees are refused. Every expected
//! path is computed through `layout::paths` rather than spelled out, so a test asserts that the
//! adapter uses the kit's grammar rather than re-stating a guess at what the grammar says.

use std::io::Write;

use morphir_common::ir_transport::{
    CodecOptions, DocumentTreeSink, EventSink, FormatId, IrCodec, IrVersion, JsonCodec, Layout,
    TransportDiagnostic, discover_document_tree_format, read_document_tree,
    read_document_tree_with_options, write_document_tree, write_document_tree_with_options,
};
use morphir_common::vfs::{memory_root, physical_root};
use morphir_core::ir::classic;
use morphir_core::ir::layout::{
    NodeFileKind, Profile, Root, module_dir, module_manifest_path, node_file_path, to_physical,
};
use morphir_core::ir::v4::{Distribution, IRFile};
use morphir_core::migration::{MigrationOptions, migrate_distribution};
use morphir_core::naming::{ModuleName, Name, PackageName};
use morphir_core::traversal::{DependencyEvent, SemanticEvent, SemanticEventKind};
use vfs::VfsPath;

// =============================================================================
// Fixtures and helpers
// =============================================================================

/// A two-module library with no dependencies, built from a classic v3 document and migrated.
fn fixture() -> IRFile {
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

/// The kit's complete v4 example: package `regulation`, one module with types and values, and a
/// `morphir/SDK` specification dependency.
fn granular_fixture() -> IRFile {
    serde_json::from_str(include_str!(
        "../../morphir-core/tests/fixtures/ir/v4/complete-example.json"
    ))
    .unwrap()
}

/// The canonical document of MCK distributions-0010: an application whose dependency is a package
/// definition, which a tree writes under `deps/` as definitions.
const APPLICATION_WITH_DEFINITION_DEPENDENCY: &str = r#"{"formatVersion":4,"distribution":{"Application":{"packageName":"example","dependencies":{"my-org/shared":{"modules":{"util":{"Public":{"types":{},"values":{"identity":{"Public":{"ExpressionBody":{"inputTypes":{"x":"morphir/SDK:basics#int"},"outputType":"morphir/SDK:basics#int","body":{"Variable":"x"}}}}}}}}}},"def":{"modules":{"main":{"Public":{"types":{},"values":{"run":{"Public":{"ExpressionBody":{"inputTypes":{},"outputType":"morphir/SDK:basics#unit","body":{"Unit":{}}}}}}}}}},"entryPoints":{"start":{"target":"example:main#run","kind":"main"}}}}}"#;

fn options(format: FormatId) -> CodecOptions {
    CodecOptions::new(IrVersion::V4, Layout::DocumentTree, format)
}

fn profile_of(format: &FormatId) -> Profile {
    if *format == FormatId::json() {
        Profile::Json
    } else {
        Profile::Yaml
    }
}

/// The physical file a logical path lands on under `root`.
fn at(root: &VfsPath, logical: &str, profile: Profile) -> VfsPath {
    root.join(to_physical(logical, profile)).unwrap()
}

fn assert_file(root: &VfsPath, logical: &str, profile: Profile) {
    let path = at(root, logical, profile);
    assert!(
        path.is_file().unwrap(),
        "expected {} to exist",
        path.as_str()
    );
}

fn manifest_value(root: &VfsPath, profile: Profile) -> serde_json::Value {
    let path = at(root, "manifest", profile);
    let text = path.read_to_string().unwrap();
    match profile {
        Profile::Json => serde_json::from_str(&text).unwrap(),
        Profile::Yaml => morphir_core::ir::yaml::read(&text).unwrap(),
    }
}

fn package(name: &str) -> PackageName {
    PackageName::from_canonical_string(name).unwrap()
}

fn module(name: &str) -> ModuleName {
    ModuleName::from_canonical_string(name).unwrap()
}

/// The package a distribution belongs to, and the modules its own package holds, as the model
/// keys them. The fixtures are read for their own names rather than having them spelled out: what
/// is under test is where the adapter puts a module, not what the fixture calls it.
fn own_package(ir: &IRFile) -> PackageName {
    ir.distribution.package_name().clone()
}

fn own_modules(ir: &IRFile) -> Vec<String> {
    match &ir.distribution {
        Distribution::Library(content) => content.def.modules.keys().cloned().collect(),
        Distribution::Application(content) => content.def.modules.keys().cloned().collect(),
        Distribution::Specs(content) => content.spec.modules.keys().cloned().collect(),
    }
}

/// Where one of a distribution's own modules lands.
fn own_module_dir(ir: &IRFile, index: usize) -> String {
    let modules = own_modules(ir);
    let name = modules.get(index).expect("the fixture has this module");
    module_dir(Root::Pkg, &own_package(ir), module(name).as_path())
}

fn every_physical_path(root: &VfsPath) -> Vec<String> {
    root.walk_dir()
        .unwrap()
        .filter_map(Result::ok)
        .map(|path| path.as_str().to_owned())
        .collect()
}

// =============================================================================
// Round trips
// =============================================================================

fn assert_round_trip(root: VfsPath, format: FormatId) {
    let profile = profile_of(&format);
    let expected = granular_fixture();
    let options = options(format);

    write_document_tree_with_options(&root, &expected, &options).unwrap();

    assert_file(&root, "manifest", profile);
    assert_eq!(
        read_document_tree_with_options(&root, &options).unwrap(),
        expected
    );
}

#[test]
fn a_json_tree_round_trips_on_a_memory_vfs() {
    assert_round_trip(memory_root(), FormatId::json());
}

#[test]
fn a_json_tree_round_trips_on_a_physical_vfs() {
    let temp = tempfile::tempdir().unwrap();
    assert_round_trip(physical_root(temp.path()), FormatId::json());
}

#[test]
fn a_yaml_tree_round_trips_on_a_memory_vfs() {
    assert_round_trip(memory_root(), FormatId::yaml());
}

#[test]
fn a_yaml_tree_round_trips_on_a_physical_vfs() {
    let temp = tempfile::tempdir().unwrap();
    assert_round_trip(physical_root(temp.path()), FormatId::yaml());
}

#[test]
fn an_application_with_a_definition_dependency_round_trips_through_a_tree() {
    let root = memory_root();
    let expected: IRFile = serde_json::from_str(APPLICATION_WITH_DEFINITION_DEPENDENCY).unwrap();

    write_document_tree(&root, &expected).unwrap();

    // The dependency's modules are definitions under `deps/`, not a specification in the manifest.
    let dir = module_dir(
        Root::Deps,
        &package("my-org/shared"),
        module("util").as_path(),
    );
    assert_file(
        &root,
        &module_manifest_path(Root::Deps, &dir),
        Profile::Json,
    );
    assert_eq!(read_document_tree(&root).unwrap(), expected);
}

// =============================================================================
// Where the files land
// =============================================================================

#[test]
fn the_complete_example_writes_escaped_stems_and_a_dependency_under_the_version_slot() {
    let root = memory_root();
    let options = options(FormatId::yaml());

    write_document_tree_with_options(&root, &granular_fixture(), &options).unwrap();

    let fixture = granular_fixture();
    let own = own_module_dir(&fixture, 0);
    assert_file(&root, &module_manifest_path(Root::Pkg, &own), Profile::Yaml);
    assert_file(
        &root,
        &node_file_path(
            Root::Pkg,
            &own,
            &morphir_core::naming::file_stem(&Name::from_canonical_string("data-tables").unwrap()),
            NodeFileKind::Type,
        ),
        Profile::Yaml,
    );

    // `SDK` is an initialism, so its stem is `_sdk`, and a dependency's package path ends in the
    // bare version slot the v4 model has nothing to put a version in.
    let dependency = module_dir(
        Root::Deps,
        &package("morphir/SDK"),
        module("basics").as_path(),
    );
    assert_eq!(dependency, "morphir/_sdk/@/basics");
    assert_file(
        &root,
        &module_manifest_path(Root::Deps, &dependency),
        Profile::Yaml,
    );

    // A YAML tree is spelled in YAML throughout.
    assert!(
        every_physical_path(&root)
            .iter()
            .all(|path| !path.ends_with(".json"))
    );
}

#[test]
fn the_written_manifest_carries_the_path_budget_and_its_dependencies_by_name() {
    let root = memory_root();

    write_document_tree(&root, &granular_fixture()).unwrap();

    let manifest = manifest_value(&root, Profile::Json);
    assert_eq!(manifest["pathBudget"], serde_json::json!(4000));
    assert_eq!(
        manifest["dependencies"],
        serde_json::json!(["morphir/SDK"]),
        "a dependency is a name in the manifest; its body lives under deps/"
    );
}

#[test]
fn a_small_path_budget_truncates_a_stem_and_records_it_in_file_names() {
    let root = memory_root();
    let expected = granular_fixture();
    let options = options(FormatId::json()).with_path_budget(64);

    write_document_tree_with_options(&root, &expected, &options).unwrap();

    assert_eq!(
        manifest_value(&root, Profile::Json)["pathBudget"],
        serde_json::json!(64)
    );
    let own = own_module_dir(&expected, 0);
    let module_manifest: serde_json::Value = serde_json::from_str(
        &at(&root, &module_manifest_path(Root::Pkg, &own), Profile::Json)
            .read_to_string()
            .unwrap(),
    )
    .unwrap();
    let file_names = module_manifest["fileNames"]
        .as_object()
        .expect("a budget this small cuts at least one stem");
    assert!(!file_names.is_empty());
    for (name, stem) in file_names {
        assert_ne!(
            stem.as_str().unwrap(),
            name,
            "a recorded file name is the cut stem, not the name itself"
        );
    }

    assert_eq!(
        read_document_tree_with_options(&root, &options).unwrap(),
        expected
    );
}

// =============================================================================
// Rewriting a tree
// =============================================================================

fn retain_first_module(ir: &mut IRFile) {
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

#[test]
fn rewriting_a_package_removes_stale_modules_under_pkg() {
    let root = memory_root();
    write_document_tree(&root, &fixture()).unwrap();
    let stale = own_module_dir(&fixture(), 1);
    assert_file(
        &root,
        &module_manifest_path(Root::Pkg, &stale),
        Profile::Json,
    );

    let mut replacement = fixture();
    retain_first_module(&mut replacement);
    write_document_tree(&root, &replacement).unwrap();

    assert!(
        !at(
            &root,
            &module_manifest_path(Root::Pkg, &stale),
            Profile::Json
        )
        .exists()
        .unwrap()
    );
    assert_eq!(read_document_tree(&root).unwrap(), replacement);
}

#[test]
fn rewriting_a_distribution_removes_stale_packages_under_deps() {
    let root = memory_root();
    write_document_tree(&root, &granular_fixture()).unwrap();
    assert!(root.join("deps").unwrap().exists().unwrap());

    // The plain fixture depends on nothing, so nothing belongs under `deps/` afterwards.
    let replacement = fixture();
    write_document_tree(&root, &replacement).unwrap();

    assert!(!root.join("deps").unwrap().exists().unwrap());
    assert_eq!(read_document_tree(&root).unwrap(), replacement);
}

// =============================================================================
// Discovery
// =============================================================================

#[test]
fn a_yml_manifest_is_discovered_and_read() {
    let root = memory_root();
    let expected = granular_fixture();
    let options = options(FormatId::yaml());
    write_document_tree_with_options(&root, &expected, &options).unwrap();

    let manifest = root.join("manifest.yaml").unwrap();
    let text = manifest.read_to_string().unwrap();
    manifest.remove_file().unwrap();
    root.join("manifest.yml")
        .unwrap()
        .create_file()
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();

    assert_eq!(
        discover_document_tree_format(&root).unwrap(),
        FormatId::yaml()
    );
    assert_eq!(read_document_tree(&root).unwrap(), expected);
}

#[test]
fn a_yml_node_file_is_read_in_a_yaml_tree() {
    let root = memory_root();
    let expected = granular_fixture();
    let options = options(FormatId::yaml());
    write_document_tree_with_options(&root, &expected, &options).unwrap();

    let logical = module_manifest_path(Root::Pkg, &own_module_dir(&expected, 0));
    let yaml = at(&root, &logical, Profile::Yaml);
    let text = yaml.read_to_string().unwrap();
    yaml.remove_file().unwrap();
    root.join(format!("{logical}.yml"))
        .unwrap()
        .create_file()
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();

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

// =============================================================================
// Containment: a link is never followed
// =============================================================================

/// Creates a directory link, however the platform spells one.
#[cfg(unix)]
fn link_dir(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn link_dir(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

/// A temp directory holding a tree, and a second one outside it holding a sentinel file, with a
/// directory link from `<root>/pkg/linked-away` to the second.
///
/// `None` when the platform refused to create the link — creating one on Windows needs Developer
/// Mode or the symlink privilege — with the reason printed, so a run without the privilege reports
/// a skipped guarantee rather than a passing one.
struct LinkFixture {
    root_dir: tempfile::TempDir,
    outside: tempfile::TempDir,
    root: VfsPath,
}

impl LinkFixture {
    fn build(format: FormatId, outside_file: &str) -> Option<Self> {
        let root_dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join(outside_file), b"{}").unwrap();

        let root = physical_root(root_dir.path());
        write_document_tree_with_options(&root, &granular_fixture(), &options(format)).unwrap();

        let link = root_dir.path().join("pkg").join("linked-away");
        if let Err(error) = link_dir(outside.path(), &link) {
            println!(
                "SKIPPED: this platform would not create a directory link ({error}); the \
                 containment guarantee was not exercised"
            );
            return None;
        }
        Some(Self {
            root_dir,
            outside,
            root,
        })
    }

    fn sentinel(&self, name: &str) -> std::path::PathBuf {
        self.outside.path().join(name)
    }

    fn link(&self) -> std::path::PathBuf {
        self.root_dir.path().join("pkg").join("linked-away")
    }
}

#[test]
fn rewriting_a_tree_unlinks_a_link_under_pkg_instead_of_emptying_its_target() {
    let Some(fixture) = LinkFixture::build(FormatId::json(), "sentinel.txt") else {
        return;
    };

    // The second write prunes `pkg/`. If the prune followed the link, this would delete the
    // sentinel in a directory the caller never named.
    write_document_tree(&fixture.root, &granular_fixture()).unwrap();

    assert!(
        fixture.sentinel("sentinel.txt").exists(),
        "a file outside the tree root was deleted by a rewrite"
    );
    assert!(
        !fixture.link().exists(),
        "the link itself should have been removed with the package root"
    );
}

#[test]
fn reading_a_tree_does_not_see_files_through_a_link() {
    // A stray `.json` under `pkg/` that belongs to no module is refused by the tree reader, so a
    // successful read is proof the linked directory's contents were never reached.
    let Some(fixture) = LinkFixture::build(FormatId::json(), "stray.json") else {
        return;
    };

    assert_eq!(
        read_document_tree(&fixture.root).unwrap(),
        granular_fixture()
    );
    assert!(fixture.sentinel("stray.json").exists());
}

// =============================================================================
// Refusals
// =============================================================================

#[test]
fn a_json_node_file_in_a_yaml_tree_is_refused() {
    let root = memory_root();
    let options = options(FormatId::yaml());
    write_document_tree_with_options(&root, &granular_fixture(), &options).unwrap();

    let stray = "pkg/not-a-package/not-a-module/module.json";
    let path = root.join(stray).unwrap();
    path.parent().create_dir_all().unwrap();
    path.create_file().unwrap().write_all(b"{}").unwrap();

    let diagnostic = read_document_tree_with_options(&root, &options).unwrap_err();

    assert_eq!(
        diagnostic.code(),
        "morphir::ir::document_tree::invalid_distribution_shape"
    );
    assert!(
        diagnostic
            .message()
            .contains(&format!("{stray} is not a {} file", Profile::Yaml.name())),
        "unexpected message: {}",
        diagnostic.message()
    );
}

#[test]
fn a_tree_whose_manifest_has_no_path_budget_is_refused_with_migration_guidance() {
    let root = memory_root();
    write_document_tree(&root, &fixture()).unwrap();

    let path = root.join("manifest.json").unwrap();
    let mut manifest: serde_json::Value =
        serde_json::from_str(&path.read_to_string().unwrap()).unwrap();
    manifest.as_object_mut().unwrap().remove("pathBudget");
    path.create_file()
        .unwrap()
        .write_all(serde_json::to_vec(&manifest).unwrap().as_slice())
        .unwrap();

    let diagnostic = read_document_tree(&root).unwrap_err();

    assert_eq!(
        diagnostic.code(),
        "morphir::ir::document_tree::missing_member"
    );
    assert_eq!(
        diagnostic.guidance(),
        Some("this tree predates 0.4.0-alpha.7; regenerate it with morphir migrate")
    );
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
    let mut sink = DocumentTreeSink::new(root, options(FormatId::json())).unwrap();
    sink.accept(application_begin).unwrap();
    let diagnostic = sink.accept(specification_dependency).unwrap_err();

    assert_eq!(
        diagnostic.code(),
        "morphir::ir::document_tree::unsupported_dependencies"
    );
}

// =============================================================================
// The module manifest's documentation
// =============================================================================

#[test]
fn a_module_manifest_accepts_an_array_of_lines_for_doc_and_writes_one_string() {
    let root = memory_root();
    write_document_tree(&root, &fixture()).unwrap();

    let dir = own_module_dir(&fixture(), 0);
    let module_manifest = at(&root, &module_manifest_path(Root::Pkg, &dir), Profile::Json);
    let mut raw: serde_json::Value =
        serde_json::from_str(&module_manifest.read_to_string().unwrap()).unwrap();
    let module_path = raw["path"].as_str().unwrap().to_owned();
    raw["doc"] = serde_json::json!(["line one", "line two"]);
    module_manifest
        .create_file()
        .unwrap()
        .write_all(serde_json::to_vec(&raw).unwrap().as_slice())
        .unwrap();

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
        serde_json::from_str(&module_manifest.read_to_string().unwrap()).unwrap();
    assert_eq!(rewritten["doc"], serde_json::json!("line one\nline two"));
}
