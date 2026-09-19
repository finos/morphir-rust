//! End-to-end frontend acceptance: a compile request in, a Morphir IR
//! distribution out, through the same `NativeExtension` handle the daemon uses.

use std::collections::BTreeSet;

use morphir_elm_binding::ElmExtension;
use morphir_elm_binding::resolved::{RType, ResolvedBody};
use morphir_extension_sdk::prelude::*;

const TYPES: &str = include_str!("fixtures/Types.elm");
const OTHER: &str = "module My.Other exposing (Thing)\n\ntype Thing = Thing\n";
const UNDERSCORE_B: &str = "module B exposing (Foo_Bar)\n\ntype alias Foo_Bar = Int\n";
const UNDERSCORE_A: &str = "module A exposing (T)\n\nimport B\n\ntype alias T = B.Foo_Bar\n";
const DAEMON_EXAMPLE: &str =
    include_str!("../../morphir-daemon/tests/fixtures/morphir-elm-extension/Example.elm");
const DAEMON_INVALID: &str =
    include_str!("../../morphir-daemon/tests/fixtures/morphir-elm-extension/Invalid.elm");

fn document(uri: &str, text: &str) -> SourceDocument {
    SourceDocument {
        uri: uri.into(),
        language_id: "elm".into(),
        version: 1,
        text: text.into(),
    }
}

fn compile(documents: Vec<SourceDocument>, ir_version: &str) -> CompileResult {
    compile_as("elm", "local/example", documents, ir_version, vec![])
}

fn compile_as(
    language_id: &str,
    package_name: &str,
    documents: Vec<SourceDocument>,
    ir_version: &str,
    dependencies: Vec<CompileDependency>,
) -> CompileResult {
    let extension = NativeExtension::frontend_backend(ElmExtension).unwrap();
    extension
        .frontend()
        .unwrap()
        .compile(CompileRequest {
            language_id: language_id.into(),
            documents,
            package: CompilePackage {
                name: package_name.into(),
                exposed_modules: None,
            },
            dependencies,
            options: CompileOptions {
                types_only: false,
                ir_version: ir_version.into(),
                extra: Default::default(),
            },
            baseline: None,
        })
        .unwrap()
}

/// Compiles a v3 package with an exact exposed-module list.
fn compile_exposing(
    package_name: &str,
    exposed: &[&str],
    documents: Vec<SourceDocument>,
) -> CompileResult {
    let extension = NativeExtension::frontend_backend(ElmExtension).unwrap();
    extension
        .frontend()
        .unwrap()
        .compile(CompileRequest {
            language_id: "elm".into(),
            documents,
            package: CompilePackage {
                name: package_name.into(),
                exposed_modules: Some(exposed.iter().map(|name| name.to_string()).collect()),
            },
            dependencies: vec![],
            options: CompileOptions {
                types_only: false,
                ir_version: "3".into(),
                extra: Default::default(),
            },
            baseline: None,
        })
        .unwrap()
}

/// Each module of a v3 distribution by its IR module path, with the access the
/// document writes for it.
fn accesses(result: &CompileResult) -> std::collections::BTreeMap<String, String> {
    assert!(result.success, "{:?}", result.diagnostics);
    result.ir.as_ref().expect("a distribution")["distribution"][3]["modules"]
        .as_array()
        .expect("a module list")
        .iter()
        .map(|entry| {
            let path = entry[0]
                .as_array()
                .expect("a module path")
                .iter()
                .map(|name| {
                    name.as_array()
                        .expect("a name")
                        .iter()
                        .map(|word| word.as_str().expect("a word").to_string())
                        .collect::<Vec<_>>()
                        .join("-")
                })
                .collect::<Vec<_>>()
                .join(".");
            let access = entry[1]["access"].as_str().expect("an access").to_string();
            (path, access)
        })
        .collect()
}

fn codes(result: &CompileResult, code: &str) -> Vec<Diagnostic> {
    result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code.as_deref() == Some(code))
        .cloned()
        .collect()
}

#[test]
fn compiles_a_two_module_package_for_every_supported_ir_version() {
    for version in ["3", "4"] {
        let result = compile(
            vec![
                document("file:///work/My/Domain/Types.elm", TYPES),
                document("file:///work/My/Other.elm", OTHER),
            ],
            version,
        );

        assert!(result.success, "{version}: {:?}", result.diagnostics);
        assert_eq!(result.ir_version.as_deref(), Some(version));
        assert_eq!(
            result.modules.iter().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from(["My.Domain.Types".to_string(), "My.Other".to_string()])
        );
        assert_eq!(result.module_results.len(), 2);
        assert!(
            result
                .module_results
                .iter()
                .all(|module| module.status == ModuleStatus::Compiled)
        );
        assert!(
            result
                .module_results
                .iter()
                .all(|module| module.ir.is_some() && module.interface_digest.is_some())
        );
        // `My.Other` declares no imports, so it is written before the module
        // that imports it.
        assert_eq!(result.module_results[0].name, "My.Other");

        let skipped = codes(&result, "ELM_VALUE_SKIPPED");
        assert_eq!(skipped.len(), 1, "{version}: {skipped:?}");
        assert_eq!(skipped[0].severity, DiagnosticSeverity::Warning);
        assert!(skipped[0].message.contains("greet"), "{skipped:?}");
        let location = skipped[0].location.as_ref().expect("a source location");
        assert_eq!(location.uri, "file:///work/My/Domain/Types.elm");

        let ir = result.ir.as_ref().expect("a distribution");
        if version == "3" {
            let modules = ir["distribution"][3]["modules"]
                .as_array()
                .expect("a v3 package definition with a module list");
            assert_eq!(modules.len(), 2);
        } else {
            let modules = ir["distribution"]["Library"]["def"]["modules"]
                .as_object()
                .expect("a v4 library with a module map");
            assert_eq!(modules.len(), 2);
        }
    }
}

#[test]
fn rejects_other_languages() {
    let result = compile_as(
        "gleam",
        "local/example",
        vec![document("file:///work/My/Other.elm", OTHER)],
        "3",
        vec![],
    );

    assert!(!result.success);
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].code.as_deref(), Some("ELM_REQUEST"));
    assert!(result.module_results.is_empty());
}

#[test]
fn rejects_ir_version_5() {
    let result = compile(vec![document("file:///work/My/Other.elm", OTHER)], "5");

    assert!(!result.success);
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].code.as_deref(), Some("ELM_REQUEST"));
    assert!(result.diagnostics[0].message.contains('5'));
}

#[test]
fn reports_syntax_errors_with_locations() {
    let result = compile(
        vec![document("file:///work/Invalid.elm", DAEMON_INVALID)],
        "3",
    );

    assert!(!result.success);
    assert_eq!(result.module_results.len(), 1);
    assert_eq!(result.module_results[0].status, ModuleStatus::Failed);
    assert!(result.module_results[0].source_digest.is_some());
    assert!(result.module_results[0].ir.is_none());

    let syntax = codes(&result, "ELM_SYNTAX");
    assert!(!syntax.is_empty(), "{:?}", result.diagnostics);
    let location = syntax[0].location.as_ref().expect("a source location");
    assert_eq!(location.uri, "file:///work/Invalid.elm");
    assert_eq!(location.range.start.line, 2);
}

#[test]
fn daemon_example_fixture_compiles() {
    let result = compile(
        vec![document("file:///work/Example.elm", DAEMON_EXAMPLE)],
        "3",
    );

    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(result.modules, vec!["Example".to_string()]);
    let skipped = codes(&result, "ELM_VALUE_SKIPPED");
    assert_eq!(skipped.len(), 1, "{skipped:?}");
    assert!(skipped[0].message.contains("add"));
}

/// A v4 `Library` holds each dependency's *specification*, keyed by canonical
/// package name. The request supplies the dependency's definitions, so the
/// specification is derived from them and forwarded into the document.
#[test]
fn a_v4_distribution_forwards_its_dependencies_specifications() {
    let dependency = compile_as(
        "elm",
        "acme/lib",
        vec![document("file:///lib/My/Other.elm", OTHER)],
        "4",
        vec![],
    );
    assert!(dependency.success, "{:?}", dependency.diagnostics);

    let result = compile_as(
        "elm",
        "local/example",
        vec![document("file:///work/Example.elm", DAEMON_EXAMPLE)],
        "4",
        vec![CompileDependency {
            package_name: "acme/lib".into(),
            ir_version: "4".into(),
            distribution: dependency.ir.expect("the dependency's distribution"),
        }],
    );

    assert!(result.success, "{:?}", result.diagnostics);
    let dependencies =
        result.ir.as_ref().expect("a distribution")["distribution"]["Library"]["dependencies"]
            .as_object()
            .expect("a dependency map");
    let expected = morphir_core::naming::PackageName::new(morphir_core::naming::Path {
        segments: vec![
            morphir_core::naming::Name::from("acme"),
            morphir_core::naming::Name::from("lib"),
        ],
    })
    .to_canonical_string();
    assert_eq!(dependencies.keys().collect::<Vec<_>>(), vec![&expected]);
    // The specification carries the dependency's public module, not its definitions.
    assert!(
        dependencies[&expected]["modules"]
            .as_object()
            .expect("a module map")
            .len()
            == 1
    );
}

/// A package we compile can be depended on under its natural Elm name.
///
/// `Acme.Lib` files its module `Acme.Lib.Types` under the module path `Types`,
/// because the package path is the module's prefix — the rule morphir-elm
/// follows. A dependent that writes `import Acme.Lib.Types` then finds it by
/// the very prefix match the resolver already does for dependency packages,
/// and the reference is `Acme.Lib:Types#T`.
#[test]
fn a_package_is_imported_under_its_full_elm_module_name() {
    const LIB: &str = "module Acme.Lib.Types exposing (T)\n\ntype alias T = Int\n";
    const APP: &str = "module App exposing (Wrapped)\n\nimport Acme.Lib.Types\n\ntype alias Wrapped = Acme.Lib.Types.T\n";

    for version in ["3", "4"] {
        let library = compile_as(
            "elm",
            "Acme.Lib",
            vec![document("file:///lib/Acme/Lib/Types.elm", LIB)],
            version,
            vec![],
        );
        assert!(library.success, "{version}: {:?}", library.diagnostics);

        let result = compile_as(
            "elm",
            "My.App",
            vec![document("file:///work/App.elm", APP)],
            version,
            vec![CompileDependency {
                package_name: "Acme.Lib".into(),
                ir_version: version.into(),
                distribution: library.ir.expect("the library's distribution"),
            }],
        );

        assert!(result.success, "{version}: {:?}", result.diagnostics);
        let decoded = morphir_elm_binding::backend::decode::module_of(
            version,
            &["App".to_string()],
            result.module_results[0]
                .ir
                .as_ref()
                .expect("the module's IR"),
        )
        .unwrap_or_else(|error| panic!("v{version}: {error}"));
        let ResolvedBody::Alias(RType::Ref(reference, _)) = &decoded.module.types[0].body else {
            panic!("v{version}: `Wrapped` is not an alias to a reference");
        };
        assert_eq!(reference.package, ["Acme", "Lib"], "v{version}");
        assert_eq!(reference.module, ["Types"], "v{version}");
        assert_eq!(reference.name, "T", "v{version}");
    }
}

#[test]
fn a_classic_distribution_still_writes_no_dependencies() {
    let result = compile(
        vec![document("file:///work/Example.elm", DAEMON_EXAMPLE)],
        "3",
    );

    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(
        result.ir.as_ref().expect("a distribution")["distribution"][2],
        serde_json::json!([])
    );
}

/// The v3 type definition a distribution holds for the single type of its
/// single module.
fn only_v3_type_definition(ir: &serde_json::Value) -> serde_json::Value {
    let modules = &ir["distribution"][3]["modules"];
    assert_eq!(modules.as_array().map(Vec::len), Some(1), "one module");
    let types = &modules[0][1]["value"]["types"];
    assert_eq!(types.as_array().map(Vec::len), Some(1), "one type");
    types[0][1]["value"]["value"].clone()
}

/// A function type is a spine of segments, and every segment keeps the type
/// arguments it was written with. This pins the whole v3 definition against the
/// one morphir-elm 2.100.0 writes for the very same alias, copied out of its
/// `morphir-ir.json`: an earlier lowering dropped `List Int` from
/// `List Int -> List Int` and answered `(Int -> Int) -> List Int` with no
/// diagnostic at all.
#[test]
fn a_higher_order_alias_is_written_the_way_morphir_elm_writes_it() {
    let source = "module My.Pkg.Aliases exposing (..)\n\n\
                  type alias AHigherOrder =\n    (Int -> Int) -> List Int -> List Int\n";
    let result = compile_as(
        "elm",
        "My.Pkg",
        vec![document("file:///work/My/Pkg/Aliases.elm", source)],
        "3",
        vec![],
    );

    assert!(result.success, "{:?}", result.diagnostics);
    let int = serde_json::json!([
        "Reference",
        {},
        [[["morphir"], ["s", "d", "k"]], [["basics"]], ["int"]],
        []
    ]);
    let list_int = serde_json::json!([
        "Reference",
        {},
        [[["morphir"], ["s", "d", "k"]], [["list"]], ["list"]],
        [int]
    ]);
    assert_eq!(
        only_v3_type_definition(result.ir.as_ref().expect("a distribution")),
        serde_json::json!([
            "TypeAliasDefinition",
            [],
            [
                "Function",
                {},
                ["Function", {}, int, int],
                ["Function", {}, list_int, list_int]
            ]
        ])
    );
}

/// `morphir.json` lists exposed modules package-relative, so a package
/// `My.Pkg` exposes `My.Pkg.Aliases` by writing `Aliases`. Matching the entry
/// against the full dotted name alone never hit, and every module of every
/// ordinary package came out `Private` where morphir-elm writes `Public`.
#[test]
fn a_package_relative_exposed_module_entry_makes_the_module_public() {
    let result = compile_exposing(
        "My.Pkg",
        &["Aliases"],
        vec![
            document(
                "file:///work/My/Pkg/Aliases.elm",
                "module My.Pkg.Aliases exposing (..)\n\ntype alias T = Int\n",
            ),
            document(
                "file:///work/My/Pkg/Other.elm",
                "module My.Pkg.Other exposing (..)\n\ntype alias U = Int\n",
            ),
        ],
    );

    assert_eq!(
        accesses(&result),
        [
            ("aliases".to_string(), "Public".to_string()),
            ("other".to_string(), "Private".to_string()),
        ]
        .into_iter()
        .collect()
    );
}

/// An underscore is a legal part of an Elm type name, and a Morphir name keeps
/// only the words, so `Foo_Bar` is written `["foo","bar"]`. A cross-module
/// reference to it has to be looked up in that same spelling, or a module
/// interface — which can only state the written spelling — would never match.
#[test]
fn an_underscored_type_name_resolves_across_modules() {
    for version in ["3", "4"] {
        let result = compile(
            vec![
                document("file:///work/A.elm", UNDERSCORE_A),
                document("file:///work/B.elm", UNDERSCORE_B),
            ],
            version,
        );

        assert!(result.success, "{version}: {:?}", result.diagnostics);
        assert_eq!(result.modules.len(), 2, "{version}");
    }
}

/// The flip side: two declarations a Morphir document cannot tell apart are
/// refused, rather than one silently replacing the other in the IR.
#[test]
fn two_type_names_a_document_cannot_tell_apart_are_refused() {
    let result = compile(
        vec![document(
            "file:///work/B.elm",
            "module B exposing (Foo_Bar, FooBar)\n\ntype alias Foo_Bar = Int\n\n\ntype alias FooBar = Int\n",
        )],
        "3",
    );

    assert!(!result.success);
    assert_eq!(result.module_results[0].status, ModuleStatus::Failed);
    let duplicate = codes(&result, "ELM_DUPLICATE_TYPE");
    assert_eq!(duplicate.len(), 1, "{:?}", result.diagnostics);
    assert!(duplicate[0].message.contains("Foo_Bar"), "{duplicate:?}");
    assert!(duplicate[0].message.contains("FooBar"), "{duplicate:?}");
    assert!(duplicate[0].location.is_some());
}

/// A dependency distribution that will not read is reported and then ignored,
/// so every module in the request can still compile and the distribution is
/// still assembled. The request was not carried out as asked, though — a module
/// that *had* named the dependency would have failed — so the result does not
/// claim success.
#[test]
fn an_unreadable_dependency_is_an_error_even_when_no_module_names_it() {
    let result = compile_as(
        "elm",
        "local/example",
        vec![document("file:///work/My/Other.elm", OTHER)],
        "3",
        vec![CompileDependency {
            package_name: "acme/lib".into(),
            ir_version: "3".into(),
            distribution: serde_json::json!({"garbage": true}),
        }],
    );

    assert!(!result.success, "{:?}", result.diagnostics);
    let request = codes(&result, "ELM_REQUEST");
    assert_eq!(request.len(), 1, "{:?}", result.diagnostics);
    assert_eq!(request[0].severity, DiagnosticSeverity::Error);
    assert!(request[0].message.contains("acme/lib"), "{request:?}");
    // The module that did not need the dependency still compiled, and the
    // distribution it produced is still handed back.
    assert!(result.ir.is_some());
    assert_eq!(result.modules, vec!["My.Other".to_string()]);
    assert_eq!(result.module_results[0].status, ModuleStatus::Compiled);
}

/// An alias is the type it stands for, written out, so a circle of aliases
/// describes nothing. Every declaration on the circle is reported.
#[test]
fn circular_type_aliases_are_refused() {
    let result = compile(
        vec![document(
            "file:///work/Loop.elm",
            "module Loop exposing (A, B)\n\ntype alias A = B\n\n\ntype alias B = A\n",
        )],
        "3",
    );

    assert!(!result.success);
    assert_eq!(result.module_results[0].status, ModuleStatus::Failed);
    let cycles = codes(&result, "ELM_TYPE_CYCLE");
    assert_eq!(cycles.len(), 2, "{:?}", result.diagnostics);
    assert!(cycles.iter().all(|c| c.location.is_some()));
    assert!(
        cycles.iter().any(|c| c.message.contains("`A`"))
            && cycles.iter().any(|c| c.message.contains("`B`")),
        "{cycles:?}"
    );
}

/// A custom type is a type of its own, not a spelling of another one, so it may
/// name itself as often as it likes.
#[test]
fn a_recursive_custom_type_is_not_a_cycle() {
    let result = compile(
        vec![document(
            "file:///work/Tree.elm",
            "module Tree exposing (Tree)\n\ntype Tree = Leaf | Node Tree Tree\n",
        )],
        "3",
    );

    assert!(result.success, "{:?}", result.diagnostics);
    assert!(codes(&result, "ELM_TYPE_CYCLE").is_empty());
}

#[test]
fn two_documents_declaring_one_module_are_refused() {
    let result = compile(
        vec![
            document("file:///work/One.elm", OTHER),
            document("file:///work/Two.elm", OTHER),
        ],
        "3",
    );

    assert!(!result.success);
    assert_eq!(result.module_results.len(), 1);
    let request = codes(&result, "ELM_REQUEST");
    assert_eq!(request.len(), 1, "{:?}", result.diagnostics);
    assert!(request[0].message.contains("My.Other"), "{request:?}");
    assert_eq!(
        request[0].location.as_ref().expect("a source location").uri,
        "file:///work/Two.elm"
    );
}

/// Two Elm module names can differ and still write the same in-package IR
/// path once the package prefix is stripped from each: a module named
/// exactly `Foo`, and a module named `My.Foo`, both become the IR path `Foo`
/// under package `My` — `My.Foo` is the package-qualified spelling of the
/// very `Foo` a bare `Foo` document already publishes.
///
/// The collision is caught before either module is compiled, not left to
/// fail late in the emitter: the first document to claim the path keeps
/// compiling, the second is refused with one `ELM_REQUEST` error naming both
/// modules and both uris, and a dependent of the refused module is blocked
/// exactly as it would be for any other module that failed to compile.
#[test]
fn two_modules_that_collide_once_the_package_is_stripped_are_refused() {
    const FOO: &str = "module Foo exposing (T)\n\ntype alias T = Int\n";
    const MY_FOO: &str = "module My.Foo exposing (S)\n\ntype alias S = Int\n";
    const DEPENDENT: &str =
        "module Dependent exposing (D)\n\nimport My.Foo\n\ntype alias D = My.Foo.S\n";

    let result = compile_as(
        "elm",
        "My",
        vec![
            document("file:///work/Foo.elm", FOO),
            document("file:///work/My/Foo.elm", MY_FOO),
            document("file:///work/Dependent.elm", DEPENDENT),
        ],
        "3",
        vec![],
    );

    assert!(!result.success, "{:?}", result.diagnostics);
    let request = codes(&result, "ELM_REQUEST");
    assert_eq!(request.len(), 1, "{:?}", result.diagnostics);
    assert_eq!(request[0].severity, DiagnosticSeverity::Error);
    assert!(request[0].message.contains("Foo"), "{request:?}");
    assert!(request[0].message.contains("My.Foo"), "{request:?}");
    assert!(
        request[0].message.contains("file:///work/Foo.elm"),
        "{request:?}"
    );
    assert!(
        request[0].message.contains("file:///work/My/Foo.elm"),
        "{request:?}"
    );
    assert_eq!(
        request[0].location.as_ref().expect("a source location").uri,
        "file:///work/My/Foo.elm",
        "the later document is the one refused"
    );

    let status = |name: &str| {
        result
            .module_results
            .iter()
            .find(|module| module.name == name)
            .unwrap_or_else(|| panic!("no result for module {name} in {:?}", result.module_results))
            .status
    };
    assert_eq!(result.module_results.len(), 3);
    assert_eq!(status("Foo"), ModuleStatus::Compiled);
    assert_eq!(status("My.Foo"), ModuleStatus::Failed);
    assert_eq!(status("Dependent"), ModuleStatus::Blocked);
    let blocked = codes(&result, "ELM_BLOCKED");
    assert_eq!(blocked.len(), 1, "{:?}", result.diagnostics);
}

#[test]
fn the_extension_advertises_the_elm_frontend_and_backend() {
    let info = ElmExtension::info();
    assert_eq!(info.id, "morphir-elm-native");
    assert_eq!(info.name, "Morphir Elm (native)");

    let capabilities = ElmExtension::capabilities();
    let frontend = capabilities.frontend.expect("a frontend capability");
    assert_eq!(frontend.languages[0].id, "elm");
    assert_eq!(frontend.languages[0].file_extensions, vec![".elm"]);
    assert_eq!(frontend.ir_versions, vec!["3", "4"]);
    assert!(frontend.compile && frontend.incremental && !frontend.fragments);
    assert!(capabilities.incremental);

    let backend = capabilities.backend.expect("a backend capability");
    assert_eq!(backend.targets, vec!["elm"]);
    assert!(backend.generate);
}

/// The frontend and the backend meet: what this extension compiles, it
/// generates again. `tests/backend.rs` and `tests/roundtrip.rs` take that
/// apart; this is the end-to-end handle the daemon holds.
///
/// The package is `My`, which is the prefix of both module names, so the
/// frontend files them under `Domain.Types` and `Other` and the backend writes
/// the prefix back on.
#[test]
fn the_distribution_the_frontend_writes_generates_elm_again() {
    let compiled = compile_as(
        "elm",
        "My",
        vec![
            document("file:///work/My/Domain/Types.elm", TYPES),
            document("file:///work/My/Other.elm", OTHER),
        ],
        "3",
        vec![],
    );
    assert!(compiled.success, "{:?}", compiled.diagnostics);
    let extension = NativeExtension::frontend_backend(ElmExtension).unwrap();
    let result = extension
        .backend()
        .unwrap()
        .generate(GenerateRequest {
            ir: compiled.ir.expect("a distribution"),
            target: "elm".into(),
            options: Default::default(),
        })
        .unwrap();

    assert!(result.success, "{:?}", result.diagnostics);
    let mut paths: Vec<&str> = result
        .artifacts
        .iter()
        .map(|artifact| artifact.path.as_str())
        .collect();
    paths.sort_unstable();
    assert_eq!(paths, ["src/My/Domain/Types.elm", "src/My/Other.elm"]);
}
