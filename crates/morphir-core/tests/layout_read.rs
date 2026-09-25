//! The document-tree reader: `read_tree`, its order of checks, and every diagnostic a tree read
//! can produce.
//!
//! The kit's tree cases (`spec/ir/mck/document-tree.md`, `document-tree-0003` to `0009`) are the
//! oracle for the model: each set must read to exactly the `IRFile` its canonical document reads
//! to, `document-tree-0005` included, whose `$meta` members are stripped rather than reported.
//! They are embedded in `tests/common/mod.rs`, shared with the writer's tests so the two halves
//! cannot drift.
//!
//! Everything else here is a row of the diagnostic catalogue
//! (`.dev/docs/superpowers/maps/2026-09-17-reference-tree-layout-map.md` section 4.11), pinned by
//! code, cursor *and* message together: three cursor conventions meet in this module — a bare
//! logical path for a missing file, `<logical path>#<pointer>` for everything inside one — and only
//! the message distinguishes the three wordings a stray file earns.

mod common;

use common::{CASES, DOCUMENT_0003, TREE_0003, TREE_0008, TREE_0009, read_document, yaml_document};
use morphir_core::ir::layout::{Profile, Tree, read_tree};
use morphir_core::ir::{
    Diagnostic, DiagnosticCode, DiagnosticStage, Distribution, IRFile, Warning, json,
};

const MODULE: &str = "pkg/my-org/my-project/domain/module";
const TYPE_FILE: &str = "pkg/my-org/my-project/domain/user-_id.type";

// =============================================================================
// Helpers
// =============================================================================

/// `document-tree-0003`'s tree with one edit: the fixture every catalogue row starts from.
fn escape_tree(edit: impl FnOnce(&mut Tree)) -> Tree {
    let mut files = common::tree(TREE_0003);
    edit(&mut files);
    files
}

fn set(files: &mut Tree, path: &str, text: &str) {
    files.insert(path.to_owned(), text.to_owned());
}

/// Replaces `from` with `to` inside the file at `path`, which has to be there to edit.
fn edit(files: &mut Tree, path: &str, from: &str, to: &str) {
    let text = files
        .get(path)
        .unwrap_or_else(|| panic!("the fixture has {path}"))
        .replace(from, to);
    files.insert(path.to_owned(), text);
}

fn read_yaml(files: &Tree) -> (IRFile, Vec<Warning>) {
    read_tree(files, Profile::Yaml).unwrap_or_else(|error| panic!("this tree reads: {error:?}"))
}

fn refusal(files: &Tree) -> Diagnostic {
    read_tree(files, Profile::Yaml).expect_err("this tree cannot be read")
}

fn assert_diagnostic(diagnostic: &Diagnostic, code: DiagnosticCode, cursor: &str, message: &str) {
    assert_eq!(diagnostic.code, code, "code");
    assert_eq!(diagnostic.cursor, cursor, "cursor");
    assert_eq!(diagnostic.message, message, "message");
}

#[test]
fn tree_reader_rejects_linked_node_carriers_and_the_proposed_revision() {
    let files = escape_tree(|files| {
        let mut node = morphir_core::ir::yaml::read(files.get(TYPE_FILE).unwrap()).unwrap();
        node["def"]["Public"]["TypeAliasDefinition"]["typeExp"] = serde_json::json!({
            "Unit": {"attributes": {"@context": {"broken": {"@id": 123}}}}
        });
        files.insert(
            TYPE_FILE.to_owned(),
            morphir_core::ir::yaml::write_canonical(&node),
        );
    });
    assert!(read_tree(&files, Profile::Yaml).is_err());

    let mut files = common::tree(TREE_0003);
    for text in files.values_mut() {
        *text = text.replace("formatVersion: 4", "formatVersion: 4.1.0");
    }
    assert!(read_tree(&files, Profile::Yaml).is_err());
}

/// The document a tree read to, as canonical JSON: the text form the kit compares.
fn canonical(file: &IRFile) -> String {
    json::write_ir_file(file)
}

// =============================================================================
// The kit's tree cases
// =============================================================================

#[test]
fn every_kit_set_reads_to_its_canonical_document() {
    for case in CASES {
        let (file, warnings) = read_tree(&case.tree(), case.profile)
            .unwrap_or_else(|error| panic!("{} reads: {error:?}", case.id));
        assert_eq!(
            canonical(&file),
            canonical(&case.ir_file()),
            "{} reads to its canonical document",
            case.id
        );
        assert_eq!(warnings, Vec::new(), "{} warns about nothing", case.id);
    }
}

#[test]
fn document_tree_0005_ignores_a_top_level_meta_member() {
    // Decision 0014: `$meta` is stripped before the member check, so it is never `unknown_member`,
    // never enters the model, and never comes back out.
    let case = CASES
        .iter()
        .find(|case| case.id == "document-tree-0005")
        .expect("the kit's meta case");
    let (file, warnings) = read_tree(&case.tree(), case.profile).expect("the meta set reads");
    assert_eq!(canonical(&file), canonical(&case.ir_file()));
    assert!(warnings.is_empty());
    assert!(!canonical(&file).contains("$meta"));
}

// =============================================================================
// Row 1: no manifest
// =============================================================================

#[test]
fn a_tree_with_no_manifest_is_refused_at_the_bare_manifest_path() {
    let diagnostic = refusal(&escape_tree(|files| {
        files.remove("manifest");
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::MissingMember,
        "manifest",
        "missing member \"manifest\"",
    );
    assert_eq!(diagnostic.stage, DiagnosticStage::Semantic);
}

// =============================================================================
// Row 3: a listed name with no node file
// =============================================================================

#[test]
fn a_listed_name_with_no_node_file_is_refused_at_the_bare_node_path() {
    let diagnostic = refusal(&escape_tree(|files| {
        files.remove(TYPE_FILE);
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::MissingMember,
        TYPE_FILE,
        &format!("{MODULE} lists \"user-ID\" but there is no {TYPE_FILE}"),
    );
}

// =============================================================================
// Row 4: a node file's name is not the listed one
// =============================================================================

#[test]
fn a_node_file_whose_name_is_not_the_listed_one_is_refused() {
    let diagnostic = refusal(&escape_tree(|files| {
        edit(files, TYPE_FILE, "name: user-ID", "name: other");
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::InvalidDistributionShape,
        &format!("{TYPE_FILE}#/name"),
        &format!("expected \"user-ID\", the name {MODULE} listed, found \"other\""),
    );
}

// =============================================================================
// Row 5: a module manifest's path is not its directory
// =============================================================================

#[test]
fn a_module_path_that_is_not_its_directory_is_refused() {
    let diagnostic = refusal(&escape_tree(|files| {
        edit(files, MODULE, "path: domain", "path: elsewhere");
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::InvalidDistributionShape,
        &format!("{MODULE}#/path"),
        "module path \"elsewhere\" does not match its directory \"domain\"",
    );
}

// =============================================================================
// Rows 6 and 7: a def where a spec belongs, and a spec where a def belongs
// =============================================================================

#[test]
fn a_definition_file_where_a_specs_tree_wants_a_specification_is_refused() {
    let diagnostic = refusal(&escape_tree(|files| {
        edit(
            files,
            "manifest",
            "distribution: Library",
            "distribution: Specs",
        );
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::InvalidDistributionShape,
        &format!("{TYPE_FILE}#/"),
        "expected a specification file",
    );
}

#[test]
fn a_definition_file_under_a_library_dependency_is_refused() {
    let mut files = common::tree(TREE_0008);
    set(
        &mut files,
        "deps/morphir/_sdk/@/basics/int.type",
        "formatVersion: 4\nname: int\ndef:\n  Public:\n    TypeAliasDefinition:\n      typeParams: []\n      typeExp: morphir/SDK:string#string\n",
    );
    assert_diagnostic(
        &refusal(&files),
        DiagnosticCode::InvalidDistributionShape,
        "deps/morphir/_sdk/@/basics/int.type#/",
        "expected a specification file",
    );
}

#[test]
fn a_specification_file_under_an_applications_deps_is_refused() {
    // An application links its dependencies statically, so `deps/` holds definitions there
    // (distributions-0010). The kit has no case for it.
    let mut files = common::tree(TREE_0009);
    edit(
        &mut files,
        "deps/my-org/shared/@/util/module",
        "types: []",
        "types: [thing]",
    );
    set(
        &mut files,
        "deps/my-org/shared/@/util/thing.type",
        "formatVersion: 4\nname: thing\nspec:\n  OpaqueTypeSpecification: {}\n",
    );
    assert_diagnostic(
        &refusal(&files),
        DiagnosticCode::InvalidDistributionShape,
        "deps/my-org/shared/@/util/thing.type#/",
        "expected a definition file",
    );
}

#[test]
fn an_applications_dependency_definitions_read_into_the_model() {
    let case = CASES
        .iter()
        .find(|case| case.id == "document-tree-0009")
        .expect("the kit's application case");
    let (file, _) = read_yaml(&case.tree());
    let Distribution::Application(content) = &file.distribution else {
        panic!("document-tree-0009 is an Application");
    };
    let dependency = content
        .dependencies
        .get("my-org/shared")
        .expect("the dependency the manifest lists");
    let (_, module) = dependency.modules.first().expect("the dependency's module");
    assert_eq!(module.access, morphir_core::ir::Access::Public);
}

// =============================================================================
// Row 8: the three wordings of a stray file
// =============================================================================

const STRAY: &str = "pkg/my-org/my-project/domain/stray.type";

#[test]
fn a_file_no_module_claims_is_refused_as_belonging_to_no_module() {
    let diagnostic = refusal(&escape_tree(|files| {
        set(
            files,
            STRAY,
            "formatVersion: 4\nname: stray\ndef:\n  Public:\n    TypeAliasDefinition:\n      typeParams: []\n      typeExp: morphir/SDK:string#string\n",
        );
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::InvalidDistributionShape,
        &format!("{STRAY}#/"),
        "file belongs to no module",
    );
}

#[test]
fn a_file_under_pkg_the_grammar_does_not_recognize_is_stray() {
    let diagnostic = refusal(&escape_tree(|files| {
        set(files, "pkg/notes", "formatVersion: 4\n");
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::InvalidDistributionShape,
        "pkg/notes#/",
        "file belongs to no module",
    );
}

#[test]
fn a_directory_of_node_files_with_no_module_manifest_is_all_stray() {
    // Without a `module` file the directory is never visited, so its node files are unclaimed.
    let diagnostic = refusal(&escape_tree(|files| {
        files.remove(MODULE);
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::InvalidDistributionShape,
        &format!("{TYPE_FILE}#/"),
        "file belongs to no module",
    );
}

#[test]
fn a_dependency_directory_carrying_a_version_names_the_segment() {
    let mut files = common::tree(TREE_0008);
    files.remove("deps/morphir/_sdk/@/basics/module");
    files.remove("deps/morphir/_sdk/@/basics/int.type");
    set(
        &mut files,
        "deps/morphir/_sdk/@1.0.0/basics/module",
        "formatVersion: 4\npath: basics\ntypes: [int]\nvalues: []\n",
    );
    set(
        &mut files,
        "deps/morphir/_sdk/@1.0.0/basics/int.type",
        "formatVersion: 4\nname: int\nspec:\n  OpaqueTypeSpecification: {}\n",
    );
    assert_diagnostic(
        &refusal(&files),
        DiagnosticCode::InvalidDistributionShape,
        "deps/morphir/_sdk/@1.0.0/basics/int.type#/",
        "the dependency directory's version segment \"@1.0.0\" carries a version, but the v4 model \
         has no package version to hold (decision 0015); expected a bare \"@\"",
    );
}

#[test]
fn a_dependency_directory_with_no_version_segment_is_reported_as_missing_one() {
    let mut files = common::tree(TREE_0008);
    files.remove("deps/morphir/_sdk/@/basics/module");
    files.remove("deps/morphir/_sdk/@/basics/int.type");
    set(
        &mut files,
        "deps/morphir/_sdk/basics/module",
        "formatVersion: 4\npath: basics\ntypes: [int]\nvalues: []\n",
    );
    set(
        &mut files,
        "deps/morphir/_sdk/basics/int.type",
        "formatVersion: 4\nname: int\nspec:\n  OpaqueTypeSpecification: {}\n",
    );
    assert_diagnostic(
        &refusal(&files),
        DiagnosticCode::InvalidDistributionShape,
        "deps/morphir/_sdk/basics/int.type#/",
        "file belongs to no module; a dependency directory expects a version segment (\"@\") after \
         the package path",
    );
}

#[test]
fn only_the_first_stray_in_sorted_order_is_reported() {
    let diagnostic = refusal(&escape_tree(|files| {
        set(files, "pkg/zzz", "formatVersion: 4\n");
        set(files, "pkg/aaa", "formatVersion: 4\n");
    }));
    assert_eq!(diagnostic.cursor, "pkg/aaa#/");
}

// =============================================================================
// Row 9: a duplicate dependency
// =============================================================================

#[test]
fn a_manifest_that_lists_a_dependency_twice_is_refused() {
    let mut files = common::tree(TREE_0008);
    edit(
        &mut files,
        "manifest",
        "dependencies: [morphir/SDK]",
        "dependencies: [morphir/SDK, morphir/SDK]",
    );
    assert_diagnostic(
        &refusal(&files),
        DiagnosticCode::DuplicateMember,
        "manifest#/dependencies/1",
        "duplicate dependency \"morphir/SDK\"",
    );
}

// =============================================================================
// Rows 10 to 12: the distribution manifest's own members, re-cursored
// =============================================================================

#[test]
fn a_path_budget_below_the_floor_is_refused_under_the_manifests_path() {
    let diagnostic = refusal(&escape_tree(|files| {
        edit(files, "manifest", "pathBudget: 4000", "pathBudget: 63");
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::InvalidType,
        "manifest#/pathBudget",
        "pathBudget must be an integer of at least 64",
    );
}

#[test]
fn an_unknown_distribution_string_is_refused() {
    let diagnostic = refusal(&escape_tree(|files| {
        edit(
            files,
            "manifest",
            "distribution: Library",
            "distribution: Widget",
        );
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::InvalidDistributionShape,
        "manifest#/distribution",
        "unknown distribution \"Widget\"",
    );
}

#[test]
fn entry_points_on_a_library_manifest_are_an_unknown_member() {
    let diagnostic = refusal(&escape_tree(|files| {
        edit(
            files,
            "manifest",
            "pathBudget: 4000\n",
            "pathBudget: 4000\nentryPoints:\n  start:\n    target: example:main#run\n    kind: main\n",
        );
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::UnknownMember,
        "manifest#/entryPoints",
        "unknown member \"entryPoints\" on a Library manifest",
    );
}

#[test]
fn an_application_without_entry_points_reads() {
    // The spec page asks for a non-empty `entryPoints`; the reference accepts an absent one and
    // defaults to none, and the reference is what the kit judges.
    let files = common::tree(&[
        (
            "manifest",
            "formatVersion: 4\ndistribution: Application\npackage: example\npathBudget: 4000\n",
        ),
        (
            "pkg/example/main/module",
            "formatVersion: 4\npath: main\ntypes: []\nvalues: []\n",
        ),
    ]);
    let (file, _) = read_yaml(&files);
    let Distribution::Application(content) = &file.distribution else {
        panic!("this tree is an Application");
    };
    assert!(content.entry_points.is_empty());
}

// =============================================================================
// Rows 13 and 14: the module manifest's path member
// =============================================================================

#[test]
fn a_module_manifest_with_both_path_and_module_is_refused() {
    let diagnostic = refusal(&escape_tree(|files| {
        edit(
            files,
            MODULE,
            "path: domain",
            "path: domain\nmodule: domain",
        );
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::UnknownMember,
        &format!("{MODULE}#/module"),
        "module is the legacy spelling of path; write only one",
    );
}

#[test]
fn a_module_manifest_with_neither_path_nor_module_is_refused() {
    let diagnostic = refusal(&escape_tree(|files| {
        edit(files, MODULE, "path: domain\n", "");
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::MissingMember,
        &format!("{MODULE}#/"),
        "missing member \"path\"",
    );
}

#[test]
fn the_module_spelling_of_path_reads_without_a_warning() {
    let (file, warnings) = read_yaml(&escape_tree(|files| {
        edit(files, MODULE, "path: domain", "module: domain");
    }));
    assert_eq!(canonical(&file), canonical(&yaml_document(DOCUMENT_0003)));
    assert!(warnings.is_empty(), "the module spelling warns nothing");
}

// =============================================================================
// Row 15: a node file with neither or both of def and spec
// =============================================================================

#[test]
fn a_node_file_with_both_def_and_spec_is_refused() {
    let diagnostic = refusal(&escape_tree(|files| {
        set(
            files,
            TYPE_FILE,
            "formatVersion: 4\nname: user-ID\ndef:\n  Public:\n    TypeAliasDefinition:\n      typeParams: []\n      typeExp: morphir/SDK:string#string\nspec:\n  OpaqueTypeSpecification: {}\n",
        );
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::InvalidDistributionShape,
        &format!("{TYPE_FILE}#/"),
        "exactly one of def or spec",
    );
}

// =============================================================================
// Rows 16 and 17: fileNames
// =============================================================================

#[test]
fn a_file_names_key_the_module_does_not_list_is_refused() {
    let diagnostic = refusal(&escape_tree(|files| {
        edit(
            files,
            MODULE,
            "values: []\n",
            "values: []\nfileNames:\n  other: other\n",
        );
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::InvalidDistributionShape,
        &format!("{MODULE}#/fileNames/other"),
        "fileNames key not listed in types or values",
    );
}

#[test]
fn a_file_names_value_that_is_not_an_escaped_stem_is_refused() {
    let diagnostic = refusal(&escape_tree(|files| {
        edit(
            files,
            MODULE,
            "values: []\n",
            "values: []\nfileNames:\n  user-ID: Not A Stem\n",
        );
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::InvalidName,
        &format!("{MODULE}#/fileNames/user-ID"),
        "\"Not A Stem\" is not an escaped stem",
    );
}

#[test]
fn the_reader_trusts_the_stem_file_names_records() {
    // The reader never recomputes a truncation: a name mapped to any stem reads from that file.
    let files = common::tree(&[
        (
            "manifest",
            "formatVersion: 4\ndistribution: Library\npackage: example\npathBudget: 4000\n",
        ),
        (
            "pkg/example/main/module",
            "formatVersion: 4\npath: main\ntypes: [user-ID]\nvalues: []\nfileNames:\n  user-ID: elsewhere\n",
        ),
        (
            "pkg/example/main/elsewhere.type",
            "formatVersion: 4\nname: user-ID\nspec:\n  OpaqueTypeSpecification: {}\n",
        ),
    ]);
    // A Library's own package holds definitions, so the `spec` body is what is refused here —
    // proving the file under the recorded stem is the one that was read.
    assert_diagnostic(
        &refusal(&files),
        DiagnosticCode::InvalidDistributionShape,
        "pkg/example/main/elsewhere.type#/",
        "expected a definition file",
    );
}

// =============================================================================
// Rows 18 and 19: every file carries its own format version
// =============================================================================

#[test]
fn a_node_file_with_no_format_version_is_refused_at_the_files_root() {
    let diagnostic = refusal(&escape_tree(|files| {
        edit(files, TYPE_FILE, "formatVersion: 4\n", "");
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::MissingFormatVersion,
        &format!("{TYPE_FILE}#/"),
        "the root has no formatVersion member",
    );
}

#[test]
fn a_node_file_with_an_unsupported_format_version_is_refused() {
    let diagnostic = refusal(&escape_tree(|files| {
        edit(files, TYPE_FILE, "formatVersion: 4", "formatVersion: 5");
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::UnsupportedFormatVersionMajor,
        &format!("{TYPE_FILE}#/formatVersion"),
        "no supported release exists for major family 5.0.0",
    );
}

// =============================================================================
// Rows 20 and 21: inline entries and unknown members
// =============================================================================

#[test]
fn an_access_controlled_inline_entry_where_a_specification_belongs_is_refused() {
    let files = common::tree(&[
        (
            "manifest",
            "formatVersion: 4\ndistribution: Specs\npackage: example\npathBudget: 4000\n",
        ),
        (
            "pkg/example/main/module",
            "formatVersion: 4\npath: main\ntypes:\n  sku:\n    Public:\n      OpaqueTypeSpecification: {}\nvalues: {}\n",
        ),
    ]);
    assert_diagnostic(
        &refusal(&files),
        DiagnosticCode::InvalidDistributionShape,
        "pkg/example/main/module#/types/sku",
        "expected a specification, found an access-controlled definition",
    );
}

#[test]
fn an_unknown_member_in_a_tree_file_is_refused_under_that_files_path() {
    let diagnostic = refusal(&escape_tree(|files| {
        edit(files, MODULE, "path: domain", "path: domain\nnotes: hello");
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::UnknownMember,
        &format!("{MODULE}#/notes"),
        "unexpected member notes",
    );
}

// =============================================================================
// Precedence: the stray check runs last
// =============================================================================

#[test]
fn a_name_mismatch_is_reported_before_a_stray_file() {
    let diagnostic = refusal(&escape_tree(|files| {
        edit(files, TYPE_FILE, "name: user-ID", "name: other");
        set(files, "pkg/notes", "formatVersion: 4\n");
    }));
    assert_diagnostic(
        &diagnostic,
        DiagnosticCode::InvalidDistributionShape,
        &format!("{TYPE_FILE}#/name"),
        &format!("expected \"user-ID\", the name {MODULE} listed, found \"other\""),
    );
}

#[test]
fn an_inline_entry_leaves_a_same_named_node_file_stray() {
    // `consumed` is recorded where a file is read, and an inline entry reads no file at all.
    let files = common::tree(&[
        (
            "manifest",
            "formatVersion: 4\ndistribution: Library\npackage: example\npathBudget: 4000\n",
        ),
        (
            "pkg/example/main/module",
            "formatVersion: 4\npath: main\ntypes:\n  user-ID:\n    Public:\n      TypeAliasDefinition:\n        typeParams: []\n        typeExp: morphir/SDK:string#string\nvalues: []\n",
        ),
        (
            "pkg/example/main/user-_id.type",
            "formatVersion: 4\nname: user-ID\ndef:\n  Public:\n    TypeAliasDefinition:\n      typeParams: []\n      typeExp: morphir/SDK:string#string\n",
        ),
    ]);
    assert_diagnostic(
        &refusal(&files),
        DiagnosticCode::InvalidDistributionShape,
        "pkg/example/main/user-_id.type#/",
        "file belongs to no module",
    );
}

// =============================================================================
// What a tree read ignores, sorts and accepts
// =============================================================================

#[test]
fn a_file_outside_pkg_and_deps_is_ignored() {
    let (file, _) = read_yaml(&escape_tree(|files| {
        set(files, "notes", "anything at all");
    }));
    assert_eq!(canonical(&file), canonical(&yaml_document(DOCUMENT_0003)));
}

const OUT_OF_ORDER: &[(&str, &str)] = &[
    (
        "manifest",
        "formatVersion: 4\ndistribution: Library\npackage: example\npathBudget: 4000\n",
    ),
    (
        "pkg/example/zeta/module",
        "formatVersion: 4\npath: zeta\ntypes: []\nvalues: []\n",
    ),
    (
        "pkg/example/alpha/module",
        "formatVersion: 4\npath: alpha\ntypes: []\nvalues: []\n",
    ),
];

#[test]
fn modules_come_back_sorted_by_logical_path() {
    let (file, _) = read_yaml(&common::tree(OUT_OF_ORDER));
    let Distribution::Library(content) = &file.distribution else {
        panic!("this tree is a Library");
    };
    assert_eq!(
        content.def.modules.keys().collect::<Vec<_>>(),
        vec!["alpha", "zeta"]
    );
}

/// One type alias whose `typeExp` spells `Function`'s parameter type with the legacy `arg`, so
/// reading the file records one `legacy_spelling` warning.
fn warning_type_file(name: &str) -> String {
    format!(
        "formatVersion: 4\nname: {name}\ndef:\n  Public:\n    TypeAliasDefinition:\n      typeParams: []\n      typeExp:\n        Function:\n          arg: morphir/SDK:string#string\n          returnType: morphir/SDK:string#string\n"
    )
}

#[test]
fn warnings_come_back_ordered_by_logical_path_not_by_listing_order() {
    // The module lists `zeta` before `alpha`, so the files are read in that order; the warnings
    // still come back in the order a caller can rely on, which is the tree's own.
    let mut files = common::tree(&[
        (
            "manifest",
            "formatVersion: 4\ndistribution: Library\npackage: example\npathBudget: 4000\n",
        ),
        (
            "pkg/example/main/module",
            "formatVersion: 4\npath: main\ntypes: [zeta, alpha]\nvalues: []\n",
        ),
    ]);
    set(
        &mut files,
        "pkg/example/main/alpha.type",
        &warning_type_file("alpha"),
    );
    set(
        &mut files,
        "pkg/example/main/zeta.type",
        &warning_type_file("zeta"),
    );

    let (_, warnings) = read_yaml(&files);
    assert_eq!(warnings.len(), 2, "one warning per file");
    assert!(
        warnings
            .iter()
            .all(|warning| warning.code == DiagnosticCode::LegacySpelling)
    );
    assert_eq!(
        warnings
            .iter()
            .map(|warning| warning.cursor.as_str())
            .collect::<Vec<_>>(),
        vec![
            "pkg/example/main/alpha.type#/def/Public/TypeAliasDefinition/typeExp/Function/arg",
            "pkg/example/main/zeta.type#/def/Public/TypeAliasDefinition/typeExp/Function/arg",
        ],
    );
}

#[test]
fn a_hybrid_module_reads_listed_types_and_inline_values() {
    let files = common::tree(&[
        (
            "manifest",
            "formatVersion: 4\ndistribution: Library\npackage: example\npathBudget: 4000\n",
        ),
        (
            "pkg/example/main/module",
            "formatVersion: 4\npath: main\ntypes: [user-ID]\nvalues:\n  run:\n    Public:\n      ExpressionBody:\n        inputTypes: {}\n        outputType: morphir/SDK:basics#unit\n        body:\n          Unit: {}\n",
        ),
        (
            "pkg/example/main/user-_id.type",
            "formatVersion: 4\nname: user-ID\ndef:\n  Public:\n    TypeAliasDefinition:\n      typeParams: []\n      typeExp: morphir/SDK:string#string\n",
        ),
    ]);
    let (file, _) = read_yaml(&files);
    let expected = yaml_document(
        "formatVersion: 4\ndistribution:\n  Library:\n    packageName: example\n    dependencies: {}\n    def:\n      modules:\n        main:\n          Public:\n            types:\n              user-ID:\n                Public:\n                  TypeAliasDefinition:\n                    typeParams: []\n                    typeExp: morphir/SDK:string#string\n            values:\n              run:\n                Public:\n                  ExpressionBody:\n                    inputTypes: {}\n                    outputType: morphir/SDK:basics#unit\n                    body:\n                      Unit: {}\n",
    );
    assert_eq!(canonical(&file), canonical(&expected));
}

#[test]
fn nested_dependency_packages_are_told_apart_by_the_version_slot() {
    // Decision 0015's motivating case: dependency `a`'s module `b/c` and dependency `a/b`'s module
    // `c` would want the same directory without the slot.
    let files = common::tree(&[
        (
            "manifest",
            "formatVersion: 4\ndistribution: Library\npackage: example\npathBudget: 4000\ndependencies: [a, a/b]\n",
        ),
        (
            "deps/a/@/b/c/module",
            "formatVersion: 4\npath: b/c\ntypes: []\nvalues: []\n",
        ),
        (
            "deps/a/b/@/c/module",
            "formatVersion: 4\npath: c\ntypes: []\nvalues: []\n",
        ),
    ]);
    let (file, _) = read_yaml(&files);
    let Distribution::Library(content) = &file.distribution else {
        panic!("this tree is a Library");
    };
    assert_eq!(
        content.dependencies.keys().collect::<Vec<_>>(),
        vec!["a", "a/b"]
    );
    assert_eq!(
        content
            .dependencies
            .get("a")
            .expect("dependency a")
            .modules
            .keys()
            .collect::<Vec<_>>(),
        vec!["b/c"]
    );
}

#[test]
fn a_legacy_spelling_inside_a_node_file_warns_under_that_files_path() {
    let (_, warnings) = read_yaml(&escape_tree(|files| {
        set(
            files,
            TYPE_FILE,
            "formatVersion: 4\nname: user-ID\ndef:\n  Public:\n    doc: The user's identifier\n    TypeAliasDefinition:\n      typeParams: []\n      typeExp:\n        Function:\n          arg: morphir/SDK:string#string\n          returnType: morphir/SDK:string#string\n",
        );
    }));
    assert_eq!(warnings.len(), 1, "one legacy spelling, one warning");
    let warning = &warnings[0];
    assert_eq!(warning.code, DiagnosticCode::LegacySpelling);
    assert!(
        warning.cursor.starts_with(&format!("{TYPE_FILE}#/")),
        "the warning carries the file it came from: {}",
        warning.cursor
    );
    assert!(warning.cursor.ends_with("/arg"), "{}", warning.cursor);
}

#[test]
fn a_node_files_format_version_need_not_match_the_manifests() {
    // The `IRFile`'s format version is the manifest's; no cross-check exists.
    let (file, _) = read_yaml(&escape_tree(|files| {
        edit(
            files,
            TYPE_FILE,
            "formatVersion: 4",
            "formatVersion: \"4.0.0\"",
        );
    }));
    assert_eq!(canonical(&file), canonical(&yaml_document(DOCUMENT_0003)));
}

#[test]
fn a_json_tree_reads_under_the_json_profile() {
    let case = CASES
        .iter()
        .find(|case| case.id == "document-tree-0006")
        .expect("the kit's json case");
    let (file, _) = read_tree(&case.tree(), Profile::Json).expect("the json set reads");
    assert_eq!(
        canonical(&file),
        canonical(&read_document(Profile::Json, case.document))
    );
}
