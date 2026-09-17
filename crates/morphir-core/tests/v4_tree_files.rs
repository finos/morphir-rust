//! The four document-tree file models, read and written as ordinary v4 nodes.
//!
//! A document tree is a set of files — one distribution manifest, one manifest per module and one
//! file per type or value — and each of them is a v4 node in its own right: it repeats the format
//! version at its root, it is checked against the same support table a whole document is checked
//! against, and it has exactly one canonical spelling. These tests pin the two kit cases the
//! models make runnable (`document-tree-0001` and `document-tree-0002`, byte for byte) and one
//! assertion per rule the reference readers and writers state — each on the code, the cursor and
//! the message together, because a diagnostic with the right code at the wrong place is a
//! different answer.
//!
//! Every fixture outside the two kit fences is written with the package `acme/shop`, so a fixture
//! that drifted from the kit's own names is obvious.

use morphir_core::ir::json::write_canonical;
use morphir_core::ir::v4::{
    Access, DistributionKind, DistributionManifestFile, FILE_STEM_PATTERN, FormatVersion,
    ModuleEntries, ModuleManifestFile, NodeFileBody, SpellingMode, TypeDefinitionFile,
    TypeEncoding, ValueDefinitionFile, with_spelling_mode, with_type_encoding,
};
use morphir_core::ir::{Diagnostic, DiagnosticCode, Warning};

// =============================================================================
// Helpers
// =============================================================================

fn decode<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, Diagnostic> {
    serde_json::from_str::<T>(text).map_err(|error| {
        Diagnostic::from_serde_error(&error)
            .unwrap_or_else(|| panic!("a tree-file decoder carries a diagnostic, got: {error}"))
    })
}

fn decode_with_warnings<T: serde::de::DeserializeOwned>(
    text: &str,
) -> (Result<T, Diagnostic>, Vec<Warning>) {
    with_spelling_mode(SpellingMode::Current, || decode::<T>(text))
}

/// The canonical text of a tree file: the value tree built under [`TypeEncoding::Compact`], the
/// canonical spelling of a type expression, then the JSON profile's canonical writer.
fn write<T: serde::Serialize>(node: &T) -> String {
    let value = with_type_encoding(TypeEncoding::Compact, || serde_json::to_value(node))
        .expect("a tree file serialises");
    write_canonical(&value)
}

#[track_caller]
fn refusal<T: serde::de::DeserializeOwned>(text: &str) -> Diagnostic {
    match decode::<T>(text) {
        Ok(_) => panic!("expected a refusal, got a decoded file"),
        Err(diagnostic) => diagnostic,
    }
}

/// The diagnostic a whole single-document read answers with, for the assertions that hold a tree
/// file's answer against the document reader's.
#[track_caller]
fn refusal_of_ir_file(text: &str) -> Diagnostic {
    match morphir_core::ir::json::read_ir_file(text) {
        Ok(_) => panic!("expected a refusal, got a decoded document"),
        Err(error) => error.0,
    }
}

#[track_caller]
fn assert_refused(diagnostic: &Diagnostic, code: DiagnosticCode, cursor: &str, message: &str) {
    assert_eq!(diagnostic.code, code, "code");
    assert_eq!(diagnostic.cursor, cursor, "cursor");
    assert_eq!(diagnostic.message, message, "message");
}

// =============================================================================
// The kit's two node cases, byte for byte
// =============================================================================

/// `spec/ir/mck/document-tree.md`, document-tree-0001, the `json canonical` fence.
const KIT_0001: &str = r#"{ "formatVersion": 4, "distribution": "Library", "package": "my-org/my-project", "pathBudget": 4000 }"#;

/// document-tree-0002, the `json canonical` fence.
const KIT_0002: &str = r#"{ "formatVersion": 4, "path": "my-org/domain", "types": ["user", "user-ID"], "values": ["get-user"] }"#;

/// document-tree-0002, the `json accepted` fence: `module` is an accepted spelling of `path`.
const KIT_0002_ACCEPTED: &str = r#"{ "formatVersion": 4, "module": "my-org/domain", "types": ["user", "user-ID"], "values": ["get-user"] }"#;

#[test]
fn the_kits_distribution_manifest_round_trips_byte_for_byte() {
    let file: DistributionManifestFile = decode(KIT_0001).expect("document-tree-0001 decodes");
    assert_eq!(file.distribution, DistributionKind::Library);
    assert_eq!(file.package.to_canonical_string(), "my-org/my-project");
    assert_eq!(file.path_budget, 4000);
    assert!(file.dependencies.is_empty());
    assert!(file.entry_points.is_empty());
    assert_eq!(write(&file), KIT_0001);
}

#[test]
fn the_kits_module_manifest_round_trips_byte_for_byte() {
    let file: ModuleManifestFile = decode(KIT_0002).expect("document-tree-0002 decodes");
    assert_eq!(file.path.to_canonical_string(), "my-org/domain");
    assert_eq!(file.access, Access::Public);
    assert_eq!(file.doc, None);
    assert!(file.file_names.is_empty());
    assert_eq!(write(&file), KIT_0002);
}

#[test]
fn the_accepted_module_spelling_writes_path_and_warns_about_nothing() {
    let (decoded, warnings) = decode_with_warnings::<ModuleManifestFile>(KIT_0002_ACCEPTED);
    let file = decoded.expect("the accepted spelling decodes");
    assert_eq!(
        warnings,
        Vec::new(),
        "module is an accepted spelling of path, not a windowed legacy one"
    );
    assert_eq!(write(&file), KIT_0002);
}

// =============================================================================
// $meta
// =============================================================================

#[test]
fn a_top_level_meta_member_is_stripped_from_a_distribution_manifest() {
    let text = r#"{ "formatVersion": 4, "distribution": "Library", "package": "acme/shop", "pathBudget": 4000, "$meta": { "x": 1 } }"#;
    let file: DistributionManifestFile = decode(text).expect("$meta is reserved and ignored");
    assert_eq!(
        write(&file),
        r#"{ "formatVersion": 4, "distribution": "Library", "package": "acme/shop", "pathBudget": 4000 }"#
    );
}

#[test]
fn a_top_level_meta_member_is_stripped_from_a_module_manifest() {
    let text = r#"{ "formatVersion": 4, "path": "orders", "types": [], "values": [], "$meta": { "x": 1 } }"#;
    let file: ModuleManifestFile = decode(text).expect("$meta is reserved and ignored");
    assert_eq!(
        write(&file),
        r#"{ "formatVersion": 4, "path": "orders", "types": [], "values": [] }"#
    );
}

#[test]
fn a_top_level_meta_member_is_stripped_from_a_type_definition_file() {
    let text = r#"{ "formatVersion": 4, "name": "sku", "spec": { "OpaqueTypeSpecification": {} }, "$meta": { "x": 1 } }"#;
    let file: TypeDefinitionFile = decode(text).expect("$meta is reserved and ignored");
    assert_eq!(
        write(&file),
        r#"{ "formatVersion": 4, "name": "sku", "spec": { "OpaqueTypeSpecification": {} } }"#
    );
}

#[test]
fn a_top_level_meta_member_is_stripped_from_a_value_definition_file() {
    let text = r#"{ "formatVersion": 4, "name": "total", "spec": { "inputs": {}, "output": "morphir/SDK:basics#int" }, "$meta": { "x": 1 } }"#;
    let file: ValueDefinitionFile = decode(text).expect("$meta is reserved and ignored");
    assert_eq!(
        write(&file),
        // A value specification with no inputs writes none: the canonical spelling of a
        // specification is the specification writer's, not this file's.
        r#"{ "formatVersion": 4, "name": "total", "spec": { "output": "morphir/SDK:basics#int" } }"#
    );
}

#[test]
fn a_nested_meta_member_is_not_stripped() {
    // Only the *top-level* member is reserved; one anywhere else is an unknown member like any
    // other.
    let text = r#"{ "formatVersion": 4, "distribution": "Application", "package": "acme/shop", "pathBudget": 4000, "entryPoints": { "start": { "target": "acme/shop:main#run", "kind": "main", "$meta": { "x": 1 } } } }"#;
    let diagnostic = refusal::<DistributionManifestFile>(text);
    assert_eq!(diagnostic.code, DiagnosticCode::UnknownMember);
    assert_eq!(diagnostic.cursor, "/entryPoints/start/$meta");
}

// =============================================================================
// The per-file format version
// =============================================================================

#[test]
fn a_file_with_no_format_version_is_refused_at_its_root() {
    let text = r#"{ "distribution": "Library", "package": "acme/shop", "pathBudget": 4000 }"#;
    assert_refused(
        &refusal::<DistributionManifestFile>(text),
        DiagnosticCode::MissingFormatVersion,
        "",
        "the root has no formatVersion member",
    );
}

#[test]
fn a_module_manifest_with_no_format_version_is_refused_at_its_root() {
    let text = r#"{ "path": "orders", "types": [], "values": [] }"#;
    assert_refused(
        &refusal::<ModuleManifestFile>(text),
        DiagnosticCode::MissingFormatVersion,
        "",
        "the root has no formatVersion member",
    );
}

#[test]
fn a_node_file_with_no_format_version_is_refused_at_its_root() {
    let text = r#"{ "name": "sku", "spec": { "OpaqueTypeSpecification": {} } }"#;
    assert_refused(
        &refusal::<TypeDefinitionFile>(text),
        DiagnosticCode::MissingFormatVersion,
        "",
        "the root has no formatVersion member",
    );
}

#[test]
fn an_unsupported_format_version_is_refused_at_the_member() {
    let text = r#"{ "formatVersion": 5, "distribution": "Library", "package": "acme/shop", "pathBudget": 4000 }"#;
    // The support table a whole document is checked against is the table a tree file is checked
    // against, down to the wording it refuses with.
    let whole_document =
        r#"{ "formatVersion": 5, "distribution": { "Library": { "packageName": "acme/shop" } } }"#;
    let expected = refusal_of_ir_file(whole_document);
    let diagnostic = refusal::<DistributionManifestFile>(text);
    assert_eq!(
        diagnostic.code,
        DiagnosticCode::UnsupportedFormatVersionMajor
    );
    assert_eq!(diagnostic.cursor, "/formatVersion");
    assert_eq!(diagnostic.message, expected.message);
}

#[test]
fn an_unknown_member_in_a_tree_file_is_refused_where_it_was_written() {
    let text =
        r#"{ "formatVersion": 4, "path": "orders", "types": [], "values": [], "extras": {} }"#;
    assert_refused(
        &refusal::<ModuleManifestFile>(text),
        DiagnosticCode::UnknownMember,
        "/extras",
        // The wording is this binding's own, shared with every other v4 wrapper; the reference
        // spells the same refusal `unknown member "extras"`.
        "unexpected member extras",
    );
}

#[test]
fn a_file_carries_its_own_format_version_spelling() {
    let text = r#"{ "formatVersion": "4.0.1", "path": "orders", "types": [], "values": [] }"#;
    let file: ModuleManifestFile = decode(text).expect("4.0.1 is supported");
    assert_eq!(file.format_version, FormatVersion::String("4.0.1".into()));
    assert_eq!(
        write(&file),
        r#"{ "formatVersion": "4.0.1", "path": "orders", "types": [], "values": [] }"#
    );
}

// =============================================================================
// The distribution manifest
// =============================================================================

#[test]
fn a_path_budget_below_the_floor_is_refused() {
    let text = r#"{ "formatVersion": 4, "distribution": "Library", "package": "acme/shop", "pathBudget": 63 }"#;
    assert_refused(
        &refusal::<DistributionManifestFile>(text),
        DiagnosticCode::InvalidType,
        "/pathBudget",
        "pathBudget must be an integer of at least 64",
    );
}

#[test]
fn a_path_budget_that_is_not_a_number_is_refused_as_a_type_error_first() {
    // A budget that is not a number at all is a mistake about the member's type, not about the
    // floor: only a number is measured against 64.
    let text = r#"{ "formatVersion": 4, "distribution": "Library", "package": "acme/shop", "pathBudget": "4000" }"#;
    assert_refused(
        &refusal::<DistributionManifestFile>(text),
        DiagnosticCode::InvalidType,
        "/pathBudget",
        "expected a number, found string",
    );
}

#[test]
fn a_path_budget_that_is_not_an_integer_is_refused() {
    let text = r#"{ "formatVersion": 4, "distribution": "Library", "package": "acme/shop", "pathBudget": 4000.5 }"#;
    assert_refused(
        &refusal::<DistributionManifestFile>(text),
        DiagnosticCode::InvalidType,
        "/pathBudget",
        "pathBudget must be an integer of at least 64",
    );
}

#[test]
fn an_unknown_distribution_kind_is_refused() {
    let text = r#"{ "formatVersion": 4, "distribution": "Widget", "package": "acme/shop", "pathBudget": 4000 }"#;
    assert_refused(
        &refusal::<DistributionManifestFile>(text),
        DiagnosticCode::InvalidDistributionShape,
        "/distribution",
        "unknown distribution \"Widget\"",
    );
}

#[test]
fn a_duplicate_dependency_is_refused_at_its_second_occurrence() {
    let text = r#"{ "formatVersion": 4, "distribution": "Library", "package": "acme/shop", "pathBudget": 4000, "dependencies": ["morphir/SDK", "morphir/SDK"] }"#;
    assert_refused(
        &refusal::<DistributionManifestFile>(text),
        DiagnosticCode::DuplicateMember,
        "/dependencies/1",
        "duplicate dependency \"morphir/SDK\"",
    );
}

#[test]
fn dependencies_are_written_only_when_there_are_any() {
    let text = r#"{ "formatVersion": 4, "distribution": "Library", "package": "acme/shop", "pathBudget": 4000, "dependencies": ["morphir/SDK"] }"#;
    let file: DistributionManifestFile = decode(text).expect("a listed dependency decodes");
    assert_eq!(write(&file), text);

    let empty = r#"{ "formatVersion": 4, "distribution": "Library", "package": "acme/shop", "pathBudget": 4000, "dependencies": [] }"#;
    let file: DistributionManifestFile = decode(empty).expect("an empty list decodes");
    assert_eq!(
        write(&file),
        r#"{ "formatVersion": 4, "distribution": "Library", "package": "acme/shop", "pathBudget": 4000 }"#
    );
}

#[test]
fn entry_points_on_a_non_application_manifest_are_unknown() {
    let text = r#"{ "formatVersion": 4, "distribution": "Library", "package": "acme/shop", "pathBudget": 4000, "entryPoints": { "start": { "target": "acme/shop:main#run", "kind": "main" } } }"#;
    assert_refused(
        &refusal::<DistributionManifestFile>(text),
        DiagnosticCode::UnknownMember,
        "/entryPoints",
        "unknown member \"entryPoints\" on a Library manifest",
    );
}

#[test]
fn entry_points_on_a_specs_manifest_name_the_specs_kind() {
    let text = r#"{ "formatVersion": 4, "distribution": "Specs", "package": "acme/shop", "pathBudget": 4000, "entryPoints": {} }"#;
    assert_refused(
        &refusal::<DistributionManifestFile>(text),
        DiagnosticCode::UnknownMember,
        "/entryPoints",
        "unknown member \"entryPoints\" on a Specs manifest",
    );
}

#[test]
fn an_applications_entry_points_round_trip_after_the_dependencies() {
    let text = r#"{ "formatVersion": 4, "distribution": "Application", "package": "acme/shop", "pathBudget": 4000, "dependencies": ["morphir/SDK"], "entryPoints": { "start": { "target": "acme/shop:main#run", "kind": "main" } } }"#;
    let file: DistributionManifestFile = decode(text).expect("an application manifest decodes");
    assert_eq!(file.distribution, DistributionKind::Application);
    assert_eq!(write(&file), text);
}

#[test]
fn an_applications_empty_entry_points_are_not_written() {
    let text = r#"{ "formatVersion": 4, "distribution": "Application", "package": "acme/shop", "pathBudget": 4000, "entryPoints": {} }"#;
    let file: DistributionManifestFile = decode(text).expect("an empty entry point map decodes");
    assert_eq!(
        write(&file),
        r#"{ "formatVersion": 4, "distribution": "Application", "package": "acme/shop", "pathBudget": 4000 }"#
    );
}

#[test]
fn version_created_and_layout_are_read_type_checked_and_discarded() {
    let text = r#"{ "formatVersion": 4, "distribution": "Library", "package": "acme/shop", "pathBudget": 4000, "version": "1.2.3", "created": "2026-09-17", "layout": "tree" }"#;
    let file: DistributionManifestFile = decode(text).expect("the recorded strings are accepted");
    assert_eq!(
        write(&file),
        r#"{ "formatVersion": 4, "distribution": "Library", "package": "acme/shop", "pathBudget": 4000 }"#
    );

    let mistyped = r#"{ "formatVersion": 4, "distribution": "Library", "package": "acme/shop", "pathBudget": 4000, "layout": 4 }"#;
    let diagnostic = refusal::<DistributionManifestFile>(mistyped);
    assert_eq!(diagnostic.code, DiagnosticCode::InvalidType);
    assert_eq!(diagnostic.cursor, "/layout");
}

#[test]
fn a_manifest_missing_a_required_member_is_refused() {
    let text = r#"{ "formatVersion": 4, "distribution": "Library", "package": "acme/shop" }"#;
    assert_refused(
        &refusal::<DistributionManifestFile>(text),
        DiagnosticCode::MissingMember,
        "",
        "missing member pathBudget",
    );
}

// =============================================================================
// Required members are settled before any value is read
// =============================================================================

#[test]
fn a_manifest_missing_a_required_member_says_so_before_reading_a_bad_one() {
    // `distribution` names no kind and `pathBudget` is absent. What is missing is settled first,
    // so the answer is about the member that is not there.
    let text = r#"{ "formatVersion": 4, "distribution": "Widget", "package": "acme/shop" }"#;
    assert_refused(
        &refusal::<DistributionManifestFile>(text),
        DiagnosticCode::MissingMember,
        "",
        "missing member pathBudget",
    );
}

#[test]
fn a_module_manifest_missing_its_path_says_so_before_reading_a_bad_listing() {
    let text = r#"{ "formatVersion": 4, "types": "sku", "values": [] }"#;
    assert_refused(
        &refusal::<ModuleManifestFile>(text),
        DiagnosticCode::MissingMember,
        "",
        "missing member \"path\"",
    );
}

#[test]
fn a_type_definition_file_missing_its_name_says_so_before_reading_its_body() {
    // Both `def` and `spec`, and no `name`: the missing member is the answer.
    let text = r#"{ "formatVersion": 4, "def": { "Public": { "TypeAliasDefinition": { "typeParams": [], "typeExp": "morphir/SDK:string#string" } } }, "spec": { "OpaqueTypeSpecification": {} } }"#;
    assert_refused(
        &refusal::<TypeDefinitionFile>(text),
        DiagnosticCode::MissingMember,
        "",
        "missing member name",
    );
}

#[test]
fn a_value_definition_file_missing_its_name_says_so_before_reading_its_body() {
    let text = r#"{ "formatVersion": 4, "def": "not a definition" }"#;
    assert_refused(
        &refusal::<ValueDefinitionFile>(text),
        DiagnosticCode::MissingMember,
        "",
        "missing member name",
    );
}

// =============================================================================
// The module manifest
// =============================================================================

#[test]
fn both_path_and_module_are_refused_at_the_legacy_spelling() {
    let text = r#"{ "formatVersion": 4, "path": "orders", "module": "orders", "types": [], "values": [] }"#;
    assert_refused(
        &refusal::<ModuleManifestFile>(text),
        DiagnosticCode::UnknownMember,
        "/module",
        "module is the legacy spelling of path; write only one",
    );
}

#[test]
fn neither_path_nor_module_is_refused_at_the_root() {
    let text = r#"{ "formatVersion": 4, "types": [], "values": [] }"#;
    assert_refused(
        &refusal::<ModuleManifestFile>(text),
        DiagnosticCode::MissingMember,
        "",
        "missing member \"path\"",
    );
}

#[test]
fn a_private_module_writes_its_access_right_after_the_path() {
    let text = r#"{ "formatVersion": 4, "path": "orders", "access": "Private", "types": [], "values": [] }"#;
    let file: ModuleManifestFile = decode(text).expect("a private module decodes");
    assert_eq!(file.access, Access::Private);
    assert_eq!(
        serde_json::to_string(&file).expect("a module manifest serialises"),
        r#"{"formatVersion":4,"path":"orders","access":"Private","types":[],"values":[]}"#
    );
}

#[test]
fn a_public_module_writes_no_access_member() {
    let text =
        r#"{ "formatVersion": 4, "path": "orders", "access": "pub", "types": [], "values": [] }"#;
    let file: ModuleManifestFile = decode(text).expect("the pub shorthand decodes");
    assert_eq!(file.access, Access::Public);
    assert_eq!(
        write(&file),
        r#"{ "formatVersion": 4, "path": "orders", "types": [], "values": [] }"#
    );
}

#[test]
fn an_unknown_access_spelling_is_refused() {
    let text = r#"{ "formatVersion": 4, "path": "orders", "access": "Internal", "types": [], "values": [] }"#;
    let diagnostic = refusal::<ModuleManifestFile>(text);
    assert_eq!(diagnostic.code, DiagnosticCode::InvalidAccess);
    assert_eq!(diagnostic.cursor, "/access");
}

#[test]
fn a_doc_given_as_lines_is_joined_and_written_as_one_string() {
    let text = r#"{ "formatVersion": 4, "path": "orders", "doc": ["first", "second"], "types": [], "values": [] }"#;
    let file: ModuleManifestFile = decode(text).expect("an array of lines decodes");
    assert_eq!(
        write(&file),
        r#"{ "formatVersion": 4, "path": "orders", "doc": "first\nsecond", "types": [], "values": [] }"#
    );
}

#[test]
fn a_doc_line_that_is_not_a_string_is_refused_at_its_index() {
    let text = r#"{ "formatVersion": 4, "path": "orders", "doc": ["first", 2], "types": [], "values": [] }"#;
    assert_refused(
        &refusal::<ModuleManifestFile>(text),
        DiagnosticCode::InvalidType,
        "/doc/1",
        "expected a string, found number",
    );
}

#[test]
fn a_null_doc_is_refused_rather_than_read_as_no_documentation() {
    // Only an absent `doc` is no documentation. A manifest that writes `doc: null` is saying
    // something the model has no way to keep, so it is refused where it was written.
    let text =
        r#"{ "formatVersion": 4, "path": "orders", "doc": null, "types": [], "values": [] }"#;
    assert_refused(
        &refusal::<ModuleManifestFile>(text),
        DiagnosticCode::InvalidType,
        "/doc",
        "expected a string, found null",
    );
}

#[test]
fn absent_types_and_values_are_the_empty_names_listing() {
    let text = r#"{ "formatVersion": 4, "path": "orders" }"#;
    let file: ModuleManifestFile = decode(text).expect("both listings default");
    assert!(matches!(&file.types, ModuleEntries::Names(names) if names.is_empty()));
    assert!(matches!(&file.values, ModuleEntries::Names(names) if names.is_empty()));
    assert_eq!(
        write(&file),
        r#"{ "formatVersion": 4, "path": "orders", "types": [], "values": [] }"#
    );
}

#[test]
fn an_inline_object_listing_is_read_as_definitions_for_a_bare_node() {
    let text = r#"{ "formatVersion": 4, "path": "orders", "types": { "sku": { "Public": { "TypeAliasDefinition": { "typeParams": [], "typeExp": "morphir/SDK:string#string" } } } }, "values": [] }"#;
    let file: ModuleManifestFile = decode(text).expect("an inline definition decodes");
    assert!(matches!(&file.types, ModuleEntries::Definitions(items) if items.len() == 1));
    assert_eq!(write(&file), text);
}

#[test]
fn a_types_member_that_is_neither_an_array_nor_an_object_is_refused() {
    let text = r#"{ "formatVersion": 4, "path": "orders", "types": "sku", "values": [] }"#;
    assert_refused(
        &refusal::<ModuleManifestFile>(text),
        DiagnosticCode::InvalidType,
        "/types",
        "expected an array of names or an object of entries, found string",
    );
}

#[test]
fn an_inline_listing_key_that_is_not_a_name_is_refused_at_the_key() {
    let text = r#"{ "formatVersion": 4, "path": "orders", "types": { "Not A Name": { "Public": { "TypeAliasDefinition": { "typeParams": [], "typeExp": "morphir/SDK:string#string" } } } }, "values": [] }"#;
    let diagnostic = refusal::<ModuleManifestFile>(text);
    assert_eq!(diagnostic.code, DiagnosticCode::InvalidName);
    assert_eq!(diagnostic.cursor, "/types/Not A Name");
}

/// The same rule in a single document: a module's `types` and `values` are keyed by name there
/// too, and the same decoder settles both.
#[test]
fn a_single_documents_module_listing_key_that_is_not_a_name_is_refused_at_the_key() {
    let text = r#"{ "formatVersion": 4, "distribution": { "Library": { "packageName": "acme/shop", "def": { "modules": { "orders": { "Public": { "types": { "Not A Name": { "Public": { "TypeAliasDefinition": { "typeParams": [], "typeExp": "morphir/SDK:string#string" } } } }, "values": {} } } } } } } }"#;
    let diagnostic = refusal_of_ir_file(text);
    assert_eq!(diagnostic.code, DiagnosticCode::InvalidName);
    assert_eq!(
        diagnostic.cursor,
        "/distribution/Library/def/modules/orders/Public/types/Not A Name"
    );
}

#[test]
fn a_listed_name_that_is_not_canonical_is_refused_at_its_index() {
    let text = r#"{ "formatVersion": 4, "path": "orders", "types": ["Not A Name"], "values": [] }"#;
    let diagnostic = refusal::<ModuleManifestFile>(text);
    assert_eq!(diagnostic.code, DiagnosticCode::InvalidName);
    assert_eq!(diagnostic.cursor, "/types/0");
}

#[test]
fn a_hybrid_module_lists_names_and_inlines_entries_in_the_same_file() {
    let text = r#"{ "formatVersion": 4, "path": "orders", "types": ["sku"], "values": { "total": { "Public": { "ExpressionBody": { "inputTypes": {}, "outputType": "morphir/SDK:basics#int", "body": { "Unit": {} } } } } } }"#;
    let file: ModuleManifestFile = decode(text).expect("a hybrid module decodes");
    assert!(matches!(&file.types, ModuleEntries::Names(names) if names.len() == 1));
    assert!(matches!(&file.values, ModuleEntries::Definitions(items) if items.len() == 1));
    assert_eq!(write(&file), text);
}

// =============================================================================
// fileNames
// =============================================================================

#[test]
fn file_names_are_written_after_the_listings_and_only_when_present() {
    let text = r#"{ "formatVersion": 4, "path": "orders", "types": ["customer-relationship-management-record"], "values": [], "fileNames": { "customer-relationship-management-record": "customer-relati__44a101f8" } }"#;
    let file: ModuleManifestFile = decode(text).expect("a recorded stem decodes");
    assert_eq!(file.file_names.len(), 1);
    assert_eq!(write(&file), text);
}

#[test]
fn a_file_names_key_not_listed_in_types_or_values_is_refused() {
    let text = r#"{ "formatVersion": 4, "path": "orders", "types": [], "values": [], "fileNames": { "sku": "sku" } }"#;
    assert_refused(
        &refusal::<ModuleManifestFile>(text),
        DiagnosticCode::InvalidDistributionShape,
        "/fileNames/sku",
        "fileNames key not listed in types or values",
    );
}

#[test]
fn a_file_names_key_that_is_not_a_name_is_refused() {
    let text = r#"{ "formatVersion": 4, "path": "orders", "types": [], "values": [], "fileNames": { "Not A Name": "sku" } }"#;
    let diagnostic = refusal::<ModuleManifestFile>(text);
    assert_eq!(diagnostic.code, DiagnosticCode::InvalidName);
    assert_eq!(diagnostic.cursor, "/fileNames/Not A Name");
}

#[test]
fn a_file_names_value_that_is_not_an_escaped_stem_is_refused() {
    let text = r#"{ "formatVersion": 4, "path": "orders", "types": ["sku"], "values": [], "fileNames": { "sku": "Not A Stem" } }"#;
    assert_refused(
        &refusal::<ModuleManifestFile>(text),
        DiagnosticCode::InvalidName,
        "/fileNames/sku",
        "\"Not A Stem\" is not an escaped stem",
    );
}

#[test]
fn a_file_names_value_that_is_not_a_string_is_refused() {
    let text = r#"{ "formatVersion": 4, "path": "orders", "types": ["sku"], "values": [], "fileNames": { "sku": 4 } }"#;
    let diagnostic = refusal::<ModuleManifestFile>(text);
    assert_eq!(diagnostic.code, DiagnosticCode::InvalidType);
    assert_eq!(diagnostic.cursor, "/fileNames/sku");
}

#[test]
fn the_file_stem_pattern_is_the_one_decision_0012_fixes() {
    assert_eq!(
        FILE_STEM_PATTERN,
        r"^_?[a-z0-9]+(-_?[a-z0-9]+)*(__[0-9a-f]{8})?_?$"
    );
}

// =============================================================================
// The node files
// =============================================================================

#[test]
fn a_type_definition_file_round_trips_a_def() {
    let text = r#"{ "formatVersion": 4, "name": "user-ID", "def": { "Public": { "doc": "The user's identifier", "TypeAliasDefinition": { "typeParams": [], "typeExp": "morphir/SDK:string#string" } } } }"#;
    let file: TypeDefinitionFile = decode(text).expect("a type definition file decodes");
    assert_eq!(file.name.to_canonical_string(), "user-ID");
    assert!(matches!(file.body, NodeFileBody::Def(_)));
    assert_eq!(write(&file), text);
}

#[test]
fn a_type_definition_file_round_trips_a_spec() {
    let text =
        r#"{ "formatVersion": 4, "name": "int", "spec": { "OpaqueTypeSpecification": {} } }"#;
    let file: TypeDefinitionFile = decode(text).expect("a type specification file decodes");
    assert!(matches!(file.body, NodeFileBody::Spec(_)));
    assert_eq!(write(&file), text);
}

#[test]
fn a_value_definition_file_round_trips_a_def() {
    let text = r#"{ "formatVersion": 4, "name": "run", "def": { "Public": { "ExpressionBody": { "inputTypes": {}, "outputType": "morphir/SDK:basics#unit", "body": { "Unit": {} } } } } }"#;
    let file: ValueDefinitionFile = decode(text).expect("a value definition file decodes");
    assert!(matches!(file.body, NodeFileBody::Def(_)));
    assert_eq!(write(&file), text);
}

#[test]
fn a_node_file_with_both_def_and_spec_is_refused_at_its_root() {
    let text = r#"{ "formatVersion": 4, "name": "sku", "def": { "Public": { "TypeAliasDefinition": { "typeParams": [], "typeExp": "morphir/SDK:string#string" } } }, "spec": { "OpaqueTypeSpecification": {} } }"#;
    assert_refused(
        &refusal::<TypeDefinitionFile>(text),
        DiagnosticCode::InvalidDistributionShape,
        "",
        "exactly one of def or spec",
    );
}

#[test]
fn a_node_file_with_neither_def_nor_spec_is_refused_at_its_root() {
    let text = r#"{ "formatVersion": 4, "name": "sku" }"#;
    assert_refused(
        &refusal::<ValueDefinitionFile>(text),
        DiagnosticCode::InvalidDistributionShape,
        "",
        "exactly one of def or spec",
    );
}

#[test]
fn a_node_file_needs_a_name() {
    let text = r#"{ "formatVersion": 4, "spec": { "OpaqueTypeSpecification": {} } }"#;
    assert_refused(
        &refusal::<TypeDefinitionFile>(text),
        DiagnosticCode::MissingMember,
        "",
        "missing member name",
    );
}
