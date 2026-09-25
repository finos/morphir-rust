//! The document-tree writer: `write_tree`, the two per-module writers, and the four refusals.
//!
//! The kit's tree cases (`spec/ir/mck/document-tree.md`, `document-tree-0003` to `0009`) are the
//! byte oracle. Each case gives a whole distribution document and the `file` fences of the tree it
//! writes to; both are embedded here verbatim, because a test in this submodule cannot read the
//! parent checkout's kit at run time. Every case asserts the full set of logical paths, in the
//! reference's emission order, and the exact bytes of every file.
//!
//! The refusals are pinned by code, cursor and message together: three of the four use a different
//! cursor convention, and only the message distinguishes a length failure from a floor failure
//! that the writer deliberately does not have.

use indexmap::IndexMap;
use morphir_core::ir::layout::{
    ManifestHeader, Profile, Root, TreePolicy, write_definition_module, write_manifest,
    write_manifest_header, write_specification_module, write_tree,
};
use morphir_core::ir::v4::distribution::EntryPoints;
use morphir_core::ir::v4::module::{Documented, ModuleDefinition};
use morphir_core::ir::v4::tree_files::DistributionKind;
use morphir_core::ir::v4::types::TypeDefinition;
use morphir_core::ir::{
    Access, AccessControlled, Diagnostic, DiagnosticCode, DiagnosticStage, FormatVersion, IRFile,
};
use morphir_core::naming::PackageName;

mod common;

use common::{
    DOCUMENT_0003, DOCUMENT_0004, DOCUMENT_0006, DOCUMENT_0007, DOCUMENT_0008, DOCUMENT_0009,
    TREE_0003, TREE_0004, TREE_0006, TREE_0007, TREE_0008, TREE_0009, json_document, yaml_document,
};

// =============================================================================
// Helpers
// =============================================================================

fn policy(profile: Profile, path_budget: u32) -> TreePolicy {
    TreePolicy {
        profile,
        path_budget,
    }
}

/// Asserts the whole tree: the same paths, in the same order, with the same bytes.
fn assert_tree(file: &IRFile, policy: &TreePolicy, expected: &[(&str, &str)]) {
    let written = write_tree(file, policy).expect("the kit's tree writes");

    let paths: Vec<&str> = written.iter().map(|(path, _)| path.as_str()).collect();
    let wanted: Vec<&str> = expected.iter().map(|(path, _)| *path).collect();
    assert_eq!(paths, wanted, "the set of logical paths, in emission order");

    for ((path, text), (_, expected_text)) in written.iter().zip(expected) {
        assert_eq!(text, expected_text, "the bytes of {path}");
    }
}

fn refusal(file: &IRFile, policy: &TreePolicy) -> Diagnostic {
    write_tree(file, policy).expect_err("this tree cannot be written")
}

#[test]
fn proposed_revision_writes_tree_files_and_4_0_refuses_document_metadata() {
    let mut file = json_document(DOCUMENT_0006);
    file.format_version = FormatVersion::String("4.1.0".to_owned());
    let files = write_tree(&file, &policy(Profile::Json, 4000)).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&files[0].1).unwrap();
    assert_eq!(manifest["formatVersion"], serde_json::json!("4.1.0"));

    file.format_version = FormatVersion::Integer(4);
    file.metadata = Some(Box::new(
        morphir_core::ir::v4::DocumentMeta::parse(&serde_json::json!({})).unwrap(),
    ));
    let error = refusal(&file, &policy(Profile::Json, 4000));
    assert_eq!(error.code, DiagnosticCode::InvalidDistributionShape);
}

fn assert_refusal(diagnostic: &Diagnostic, cursor: &str, message: &str) {
    assert_eq!(diagnostic.code, DiagnosticCode::InvalidDistributionShape);
    assert_eq!(diagnostic.stage, DiagnosticStage::Semantic);
    assert_eq!(diagnostic.cursor, cursor);
    assert_eq!(diagnostic.message, message);
}

// =============================================================================
// The kit's tree cases
// =============================================================================

#[test]
fn document_tree_0003_writes_the_escaped_stem() {
    assert_tree(
        &yaml_document(DOCUMENT_0003),
        &policy(Profile::Yaml, 4000),
        TREE_0003,
    );
}

#[test]
fn document_tree_0004_truncates_a_stem_and_records_it_in_file_names() {
    assert_tree(
        &yaml_document(DOCUMENT_0004),
        &policy(Profile::Yaml, 64),
        TREE_0004,
    );
}

#[test]
fn document_tree_0006_writes_the_same_tree_in_the_json_profile() {
    assert_tree(
        &json_document(DOCUMENT_0006),
        &policy(Profile::Json, 4000),
        TREE_0006,
    );
}

#[test]
fn document_tree_0007_writes_access_only_for_a_private_module() {
    assert_tree(
        &yaml_document(DOCUMENT_0007),
        &policy(Profile::Yaml, 4000),
        TREE_0007,
    );
}

#[test]
fn document_tree_0008_writes_a_dependency_under_the_bare_version_slot() {
    assert_tree(
        &yaml_document(DOCUMENT_0008),
        &policy(Profile::Yaml, 4000),
        TREE_0008,
    );
}

#[test]
fn document_tree_0009_writes_an_applications_dependencies_as_definitions() {
    assert_tree(
        &yaml_document(DOCUMENT_0009),
        &policy(Profile::Yaml, 4000),
        TREE_0009,
    );
}

#[test]
fn writing_the_same_distribution_twice_is_byte_identical() {
    let file = yaml_document(DOCUMENT_0009);
    let policy = policy(Profile::Yaml, 4000);
    assert_eq!(
        write_tree(&file, &policy).expect("the kit's tree writes"),
        write_tree(&file, &policy).expect("the kit's tree writes"),
    );
}

/// An application with no entry points: the manifest omits the member rather than writing `{}`.
const APPLICATION_WITHOUT_ENTRY_POINTS: &str = r#"formatVersion: 4
distribution:
  Application:
    packageName: example
    dependencies: {}
    def:
      modules:
        main:
          Public:
            types: {}
            values: {}
    entryPoints: {}
"#;

#[test]
fn an_application_manifest_omits_empty_entry_points() {
    assert_tree(
        &yaml_document(APPLICATION_WITHOUT_ENTRY_POINTS),
        &policy(Profile::Yaml, 4000),
        &[
            (
                "manifest",
                "formatVersion: 4\ndistribution: Application\npackage: example\npathBudget: 4000\n",
            ),
            (
                "pkg/example/main/module",
                "formatVersion: 4\npath: main\ntypes: []\nvalues: []\n",
            ),
        ],
    );
}

/// A `Specs` tree: the own package is written as specifications too, and a specification module
/// writes no `access` member even though its manifest could carry one.
const SPECS_DOCUMENT: &str = r#"formatVersion: 4
distribution:
  Specs:
    packageName: my-org/my-project
    dependencies: {}
    spec:
      modules:
        domain:
          types:
            user-ID:
              OpaqueTypeSpecification: {}
          values: {}
"#;

#[test]
fn a_specs_tree_writes_its_own_package_as_specifications() {
    assert_tree(
        &yaml_document(SPECS_DOCUMENT),
        &policy(Profile::Yaml, 4000),
        &[
            (
                "manifest",
                "formatVersion: 4\ndistribution: Specs\npackage: my-org/my-project\npathBudget: 4000\n",
            ),
            (
                "pkg/my-org/my-project/domain/module",
                "formatVersion: 4\npath: domain\ntypes: [user-ID]\nvalues: []\n",
            ),
            (
                "pkg/my-org/my-project/domain/user-_id.type",
                "formatVersion: 4\nname: user-ID\nspec:\n  OpaqueTypeSpecification: {}\n",
            ),
        ],
    );
}

// =============================================================================
// The four refusals
// =============================================================================

/// A module specification carrying annotations: refused with the *logical* module-manifest path.
const SPECS_WITH_ANNOTATIONS: &str = r#"formatVersion: 4
distribution:
  Specs:
    packageName: my-org/my-project
    dependencies: {}
    spec:
      modules:
        domain:
          annotations: [morphir/SDK:annotations#deprecated]
          types: {}
          values: {}
"#;

#[test]
fn module_annotations_cannot_be_written_to_a_document_tree() {
    let diagnostic = refusal(
        &yaml_document(SPECS_WITH_ANNOTATIONS),
        &policy(Profile::Yaml, 4000),
    );
    assert_refusal(
        &diagnostic,
        "pkg/my-org/my-project/domain/module",
        "module annotations cannot be written to a document tree",
    );
}

#[test]
fn annotations_are_refused_before_the_budget_when_both_apply() {
    let diagnostic = refusal(
        &yaml_document(SPECS_WITH_ANNOTATIONS),
        &policy(Profile::Yaml, 20),
    );
    assert_refusal(
        &diagnostic,
        "pkg/my-org/my-project/domain/module",
        "module annotations cannot be written to a document tree",
    );
}

#[test]
fn a_module_directory_over_the_budget_is_refused_at_its_physical_path() {
    let diagnostic = refusal(&yaml_document(DOCUMENT_0003), &policy(Profile::Yaml, 20));
    assert_refusal(
        &diagnostic,
        "pkg/my-org/my-project/domain/module.yaml",
        "path budget 20 cannot fit pkg/my-org/my-project/domain/module.yaml",
    );
}

/// The module directory `pkg/a/b/module.yaml` is 18 characters, so it fits a budget of 20 and the
/// refusal that follows is the stem's, not the module directory's.
const SHORT_DIRECTORY: &str = r#"formatVersion: 4
distribution:
  Library:
    packageName: a
    dependencies: {}
    def:
      modules:
        b:
          Public:
            types:
              thing:
                Public:
                  TypeAliasDefinition:
                    typeParams: []
                    typeExp: morphir/SDK:string#string
            values: {}
"#;

#[test]
fn a_stem_that_cannot_be_truncated_small_enough_is_refused_at_a_bare_slash() {
    let diagnostic = refusal(&yaml_document(SHORT_DIRECTORY), &policy(Profile::Yaml, 20));
    assert_refusal(
        &diagnostic,
        "/",
        "path budget 20 cannot fit pkg/a/b/thing.type.yaml",
    );
}

#[test]
fn the_writer_has_no_path_budget_floor() {
    // A budget below the reader's floor of 64 is not refused for being below it: the only refusal
    // a small budget earns is a path that does not fit, and a tree whose paths all fit writes.
    let file = yaml_document(SHORT_DIRECTORY);
    let written = write_tree(&file, &policy(Profile::Yaml, 40)).expect("every path fits 40");
    assert_eq!(
        written
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>(),
        vec!["manifest", "pkg/a/b/module", "pkg/a/b/thing.type"],
    );
}

// =============================================================================
// The per-module writers
// =============================================================================

fn alias(doc: Option<&str>) -> AccessControlled<Documented<TypeDefinition>> {
    AccessControlled {
        access: Access::Public,
        value: Documented::new(
            doc.map(Into::into),
            TypeDefinition::TypeAliasDefinition {
                type_params: Vec::new(),
                type_expr: morphir_core::ir::v4::types::Type::unit(Default::default()),
            },
        ),
    }
}

fn module_with_types(keys: &[&str]) -> AccessControlled<ModuleDefinition> {
    let mut types = IndexMap::new();
    for key in keys {
        types.insert((*key).to_owned(), alias(None));
    }
    AccessControlled {
        access: Access::Public,
        value: ModuleDefinition {
            types,
            values: IndexMap::new(),
            doc: None,
        },
    }
}

#[test]
fn direct_module_writer_refuses_4_0_node_facts() {
    let mut module = module_with_types(&["example"]);
    let TypeDefinition::TypeAliasDefinition { type_expr, .. } =
        &mut module.value.types.get_mut("example").unwrap().value.value
    else {
        unreachable!()
    };
    let morphir_core::ir::v4::Type::Unit(attributes) = type_expr else {
        unreachable!()
    };
    attributes.metadata = morphir_core::ir::v4::MetadataScope::parse(
        Some(&serde_json::json!({
            "deprecated": "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/value/deprecated"
        })),
        Some(&serde_json::json!({"deprecated": true})),
    )
    .unwrap();
    assert!(
        write_definition_module(
            Root::Pkg,
            &package("acme"),
            "example",
            &module,
            &FormatVersion::String("4.0.0".to_owned()),
            &policy(Profile::Json, 4000),
        )
        .is_err()
    );
}

fn package(name: &str) -> PackageName {
    PackageName::from_canonical_string(name).expect("a canonical package name")
}

#[test]
fn two_names_sharing_a_file_stem_are_refused_at_the_physical_node_path() {
    // `user-ID` and `user--id` are the two canonical encodings of one name, so they escape to one
    // stem and the second would silently overwrite the first's file.
    let module = module_with_types(&["user-ID", "user--id"]);
    let diagnostic = write_definition_module(
        Root::Pkg,
        &package("my-org/my-project"),
        "domain",
        &module,
        &FormatVersion::Integer(4),
        &policy(Profile::Yaml, 4000),
    )
    .expect_err("two names sharing a stem are refused");

    assert_refusal(
        &diagnostic,
        "pkg/my-org/my-project/domain/user-_id.type.yaml",
        "two type names share the file stem \"user-_id\"",
    );
}

#[test]
fn two_module_keys_escaping_to_one_directory_leave_one_entry_per_path() {
    // `user-ID` and `user--id` are the two canonical encodings of one name, and nothing validates
    // a module key on read, so both modules lay their files out under `pkg/acme/user-_id/`. The
    // reference accumulates into a map, so the shared module manifest keeps the position it first
    // took and carries the second module's bytes.
    let mut modules = IndexMap::new();
    modules.insert("user-ID".to_owned(), module_with_types(&["alpha"]));
    modules.insert("user--id".to_owned(), module_with_types(&["beta"]));

    let file = IRFile {
        format_version: FormatVersion::Integer(4),
        metadata: None,
        distribution: morphir_core::ir::Distribution::Library(morphir_core::ir::LibraryContent {
            package_name: package("acme"),
            dependencies: IndexMap::new(),
            def: morphir_core::ir::PackageDefinition { modules },
        }),
    };

    let written = write_tree(&file, &policy(Profile::Yaml, 4000)).expect("both modules write");

    assert_eq!(
        written
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "manifest",
            "pkg/acme/user-_id/module",
            "pkg/acme/user-_id/alpha.type",
            "pkg/acme/user-_id/beta.type",
        ],
    );
    assert_eq!(
        written[1].1,
        "formatVersion: 4\npath: user-ID\ntypes: [beta]\nvalues: []\n",
    );
}

#[test]
fn a_module_key_that_is_not_a_path_is_refused_at_the_module_it_would_have_written() {
    // The model keys its modules by their canonical string, which the reference's `NamedModule`
    // carried as a parsed name; a key that names nothing has no directory of its own, so the
    // cursor is the module manifest the key itself would have produced.
    let diagnostic = write_definition_module(
        Root::Pkg,
        &package("my-org/my-project"),
        "Not A Module",
        &module_with_types(&[]),
        &FormatVersion::Integer(4),
        &policy(Profile::Yaml, 4000),
    )
    .expect_err("a key that does not name a module path is refused");

    assert_eq!(diagnostic.code, DiagnosticCode::InvalidPath);
    assert_eq!(diagnostic.stage, DiagnosticStage::Semantic);
    assert_eq!(
        diagnostic.cursor,
        "pkg/my-org/my-project/Not A Module/module"
    );
}

#[test]
fn a_type_key_that_is_not_a_name_is_refused_at_the_module_manifest() {
    let diagnostic = write_definition_module(
        Root::Pkg,
        &package("my-org/my-project"),
        "domain",
        &module_with_types(&["Not A Name"]),
        &FormatVersion::Integer(4),
        &policy(Profile::Yaml, 4000),
    )
    .expect_err("a key that does not name a type is refused");

    assert_eq!(diagnostic.code, DiagnosticCode::InvalidName);
    assert_eq!(diagnostic.stage, DiagnosticStage::Semantic);
    assert_eq!(diagnostic.cursor, "pkg/my-org/my-project/domain/module");
}

#[test]
fn a_type_and_a_value_may_share_a_file_stem() {
    let mut values = IndexMap::new();
    values.insert(
        "thing".to_owned(),
        AccessControlled {
            access: Access::Public,
            value: Documented::new(
                None,
                morphir_core::ir::v4::value::ValueDefinition {
                    input_types: IndexMap::new(),
                    output_type: Some(morphir_core::ir::v4::types::Type::unit(Default::default())),
                    body: morphir_core::ir::v4::value::ValueBody::Expression(
                        morphir_core::ir::v4::value::Value::Unit(Default::default()),
                    ),
                },
            ),
        },
    );
    let mut module = module_with_types(&["thing"]);
    module.value.values = values;

    let written = write_definition_module(
        Root::Pkg,
        &package("my-org/my-project"),
        "domain",
        &module,
        &FormatVersion::Integer(4),
        &policy(Profile::Yaml, 4000),
    )
    .expect("a type and a value never collide: the suffixes differ");

    assert_eq!(
        written
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "pkg/my-org/my-project/domain/module",
            "pkg/my-org/my-project/domain/thing.type",
            "pkg/my-org/my-project/domain/thing.value",
        ],
    );
}

#[test]
fn a_specification_module_writes_no_access_and_one_module_at_a_time() {
    let file = yaml_document(DOCUMENT_0008);
    let morphir_core::ir::Distribution::Library(content) = &file.distribution else {
        panic!("document-tree-0008 is a Library");
    };
    let dependency = content
        .dependencies
        .get("morphir/SDK")
        .expect("the dependency the manifest lists");
    let (name, module) = dependency
        .modules
        .first()
        .expect("the dependency's one module");

    let written = write_specification_module(
        Root::Deps,
        &package("morphir/SDK"),
        name,
        module,
        &file.format_version,
        &policy(Profile::Yaml, 4000),
    )
    .expect("a specification module writes");

    assert_eq!(
        written
            .iter()
            .map(|(path, text)| (path.as_str(), text.as_str()))
            .collect::<Vec<_>>(),
        TREE_0008[2..]
            .iter()
            .map(|(path, text)| (*path, *text))
            .collect::<Vec<_>>(),
    );
}

#[test]
fn write_manifest_answers_the_manifest_path_and_its_bytes() {
    let file = yaml_document(DOCUMENT_0008);
    let (path, text) = write_manifest(&file, &policy(Profile::Yaml, 4000)).unwrap();
    assert_eq!(path, TREE_0008[0].0);
    assert_eq!(text, TREE_0008[0].1);
}

#[test]
fn write_manifest_header_writes_the_manifest_without_the_distribution() {
    // What a writer streaming one module at a time has when it reaches the end of the stream: the
    // header's own members and the dependency names it saw go past, and no package body at all.
    let header = ManifestHeader {
        format_version: FormatVersion::Integer(4),
        distribution: DistributionKind::Library,
        package: PackageName::parse("my-org/my-project"),
        dependencies: vec![PackageName::parse("morphir/SDK")],
        entry_points: EntryPoints::new(),
        metadata: None,
    };

    let (path, text) = write_manifest_header(&header, &policy(Profile::Yaml, 4000)).unwrap();

    assert_eq!(path, TREE_0008[0].0);
    assert_eq!(text, TREE_0008[0].1);

    let mut invalid = header;
    invalid.metadata = Some(Box::new(
        morphir_core::ir::v4::DocumentMeta::parse(&serde_json::json!({})).unwrap(),
    ));
    assert!(write_manifest_header(&invalid, &policy(Profile::Yaml, 4000)).is_err());
}

// =============================================================================
// The profile itself
// =============================================================================

#[test]
fn the_json_profile_reads_and_writes_its_own_canonical_bytes() {
    assert_eq!(Profile::Json.name(), "json");
    assert_eq!(Profile::Json.extension(), ".json");

    let text = TREE_0006[0].1;
    let value = Profile::Json.read(text).expect("the manifest is JSON");
    assert_eq!(Profile::Json.write(&value), text);
}

#[test]
fn the_yaml_profile_reads_and_writes_its_own_canonical_bytes() {
    assert_eq!(Profile::Yaml.name(), "yaml");
    assert_eq!(Profile::Yaml.extension(), ".yaml");

    let text = TREE_0008[0].1;
    let value = Profile::Yaml.read(text).expect("the manifest is YAML");
    assert_eq!(Profile::Yaml.write(&value), text);
}

#[test]
fn the_two_profiles_read_one_tree_file_to_the_same_value() {
    let from_yaml = Profile::Yaml
        .read(TREE_0003[1].1)
        .expect("the module manifest is YAML");
    let from_json = Profile::Json
        .read(TREE_0006[1].1)
        .expect("the module manifest is JSON");
    assert_eq!(from_yaml, from_json);
}

#[test]
fn a_profile_refuses_text_that_is_not_its_own() {
    let diagnostic = Profile::Json
        .read("formatVersion: 4\n")
        .expect_err("YAML is not JSON");
    assert_eq!(diagnostic.stage, DiagnosticStage::Syntax);
}
