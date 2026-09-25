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
    include_str!("../../morphir-host-native/tests/fixtures/morphir-elm-extension/Example.elm");
const DAEMON_INVALID: &str =
    include_str!("../../morphir-host-native/tests/fixtures/morphir-elm-extension/Invalid.elm");

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
    let extension = NativeExtension::builder(ElmExtension)
        .with_frontend()
        .with_backend()
        .with_workspace()
        .finish()
        .unwrap();
    extension
        .frontend()
        .unwrap()
        .compile(CompileRequest {
            language_id: language_id.into(),
            sources: SourceSet {
                root: None,
                documents,
            },
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
    let extension = NativeExtension::builder(ElmExtension)
        .with_frontend()
        .with_backend()
        .with_workspace()
        .finish()
        .unwrap();
    extension
        .frontend()
        .unwrap()
        .compile(CompileRequest {
            language_id: "elm".into(),
            sources: SourceSet {
                root: None,
                documents,
            },
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

/// Compiles a v3 package with the `elmOrdering` option set (or, for `None`,
/// left out so the default applies).
fn compile_ordered(order: Option<&str>, documents: Vec<SourceDocument>) -> CompileResult {
    let extension = NativeExtension::builder(ElmExtension)
        .with_frontend()
        .with_backend()
        .with_workspace()
        .finish()
        .unwrap();
    extension
        .frontend()
        .unwrap()
        .compile(CompileRequest {
            language_id: "elm".into(),
            sources: SourceSet {
                root: None,
                documents,
            },
            package: CompilePackage {
                name: "My.Pkg".into(),
                exposed_modules: None,
            },
            dependencies: vec![],
            options: CompileOptions {
                types_only: false,
                ir_version: "3".into(),
                extra: order
                    .map(|order| ("elmOrdering".to_string(), serde_json::Value::from(order)))
                    .into_iter()
                    .collect(),
            },
            baseline: None,
        })
        .unwrap()
}

/// The v3 distribution's module paths in the order it writes them, each with
/// its type names and each custom type's constructor names, all in order.
fn v3_layout(result: &CompileResult) -> Vec<(String, Vec<String>, Vec<Vec<String>>)> {
    assert!(result.success, "{:?}", result.diagnostics);
    let dashed = |name: &serde_json::Value| {
        name.as_array()
            .expect("a name")
            .iter()
            .map(|word| word.as_str().expect("a word").to_string())
            .collect::<Vec<_>>()
            .join("-")
    };
    result.ir.as_ref().expect("a distribution")["distribution"][3]["modules"]
        .as_array()
        .expect("a module list")
        .iter()
        .map(|entry| {
            let path = entry[0]
                .as_array()
                .expect("a module path")
                .iter()
                .map(dashed)
                .collect::<Vec<_>>()
                .join(".");
            let types = entry[1]["value"]["types"]
                .as_array()
                .expect("a type list")
                .to_vec();
            let constructors = types
                .iter()
                .filter_map(|ty| {
                    ty[1]["value"]["value"][2]["value"]
                        .as_array()
                        .map(|list| list.iter().map(|entry| dashed(&entry[0])).collect())
                })
                .collect();
            (
                path,
                types.iter().map(|ty| dashed(&ty[0])).collect(),
                constructors,
            )
        })
        .collect()
}

/// Compiles one v3 module with the `elmDocComments` option set (or, for
/// `None`, left out so the default applies).
fn compile_with_doc_mode(mode: Option<&str>, text: &str) -> CompileResult {
    let extension = NativeExtension::builder(ElmExtension)
        .with_frontend()
        .with_backend()
        .with_workspace()
        .finish()
        .unwrap();
    extension
        .frontend()
        .unwrap()
        .compile(CompileRequest {
            language_id: "elm".into(),
            sources: SourceSet {
                root: None,
                documents: vec![document("file:///work/Docs.elm", text)],
            },
            package: CompilePackage {
                name: "local/example".into(),
                exposed_modules: None,
            },
            dependencies: vec![],
            options: CompileOptions {
                types_only: false,
                ir_version: "3".into(),
                extra: mode
                    .map(|mode| ("elmDocComments".to_string(), serde_json::Value::from(mode)))
                    .into_iter()
                    .collect(),
            },
            baseline: None,
        })
        .unwrap()
}

/// The module doc a v3 distribution's only module carries, and the doc of each
/// of its types by name.
fn v3_docs(
    result: &CompileResult,
) -> (
    serde_json::Value,
    std::collections::BTreeMap<String, serde_json::Value>,
) {
    assert!(result.success, "{:?}", result.diagnostics);
    let module =
        &result.ir.as_ref().expect("a distribution")["distribution"][3]["modules"][0][1]["value"];
    let types = module["types"]
        .as_array()
        .expect("a type list")
        .iter()
        .map(|entry| {
            let name = entry[0]
                .as_array()
                .expect("a name")
                .iter()
                .map(|word| word.as_str().expect("a word").to_string())
                .collect::<Vec<_>>()
                .join("-");
            (name, entry[1]["value"]["doc"].clone())
        })
        .collect();
    (module["doc"].clone(), types)
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

/// An exposed module whose public surface names a type of an unexposed module
/// would describe a type nobody outside the package may name, so morphir-elm
/// publishes the module that owns it. A module nothing reaches into stays
/// private.
#[test]
fn a_module_an_exposed_module_reaches_into_is_published_too() {
    let result = compile_exposing(
        "My.Pkg",
        &["Aliases"],
        vec![
            document(
                "file:///work/My/Pkg/Aliases.elm",
                "module My.Pkg.Aliases exposing (..)\n\n\
                 import My.Pkg.Hidden\n\n\
                 type alias FromHidden = My.Pkg.Hidden.Secret\n",
            ),
            document(
                "file:///work/My/Pkg/Hidden.elm",
                "module My.Pkg.Hidden exposing (Secret)\n\ntype alias Secret = { key : String }\n",
            ),
            document(
                "file:///work/My/Pkg/Unreached.elm",
                "module My.Pkg.Unreached exposing (..)\n\ntype alias Lonely = Int\n",
            ),
        ],
    );

    assert_eq!(
        accesses(&result),
        [
            ("aliases".to_string(), "Public".to_string()),
            ("hidden".to_string(), "Public".to_string()),
            ("unreached".to_string(), "Private".to_string()),
        ]
        .into_iter()
        .collect()
    );
}

/// The promotion is transitive, as morphir-elm's is: the very type that
/// published a module contributes its own references in turn, so a chain of
/// unexposed modules is published end to end.
#[test]
fn implicit_exposure_follows_the_chain_it_opened() {
    let result = compile_exposing(
        "My.Pkg",
        &["Api"],
        vec![
            document(
                "file:///work/My/Pkg/Api.elm",
                "module My.Pkg.Api exposing (..)\n\n\
                 import My.Pkg.Middle\n\n\
                 type Request = Request My.Pkg.Middle.Body\n",
            ),
            document(
                "file:///work/My/Pkg/Middle.elm",
                "module My.Pkg.Middle exposing (Body)\n\n\
                 import My.Pkg.Deep\n\n\
                 type alias Body = { tag : My.Pkg.Deep.Tag }\n",
            ),
            document(
                "file:///work/My/Pkg/Deep.elm",
                "module My.Pkg.Deep exposing (Tag)\n\ntype alias Tag = String\n",
            ),
        ],
    );

    assert_eq!(
        accesses(&result),
        [
            ("api".to_string(), "Public".to_string()),
            ("deep".to_string(), "Public".to_string()),
            ("middle".to_string(), "Public".to_string()),
        ]
        .into_iter()
        .collect()
    );
}

/// Two types of one module open two different modules in turn, and both are
/// published.
///
/// This is where the walk parts company with morphir-elm, which stops at the
/// module: `Morphir.Elm.IncrementalFrontend` (1250-1252) drops a reference into
/// an already-published module without following it, so only whichever of
/// `Hidden.A` and `Hidden.B` it reached first would have its own references
/// followed and only one of `DeepA` and `DeepB` would come out public — while a
/// public type still pointed into the other.
#[test]
fn every_declaration_that_publishes_a_module_is_followed_not_just_the_first() {
    let result = compile_exposing(
        "My.Pkg",
        &["Api"],
        vec![
            document(
                "file:///work/My/Pkg/Api.elm",
                "module My.Pkg.Api exposing (..)\n\n\
                 import My.Pkg.Hidden\n\n\
                 type alias First = My.Pkg.Hidden.A\n\n\n\
                 type alias Second = My.Pkg.Hidden.B\n",
            ),
            document(
                "file:///work/My/Pkg/Hidden.elm",
                "module My.Pkg.Hidden exposing (A, B)\n\n\
                 import My.Pkg.DeepA\nimport My.Pkg.DeepB\n\n\n\
                 type alias A = My.Pkg.DeepA.T\n\n\n\
                 type alias B = My.Pkg.DeepB.T\n",
            ),
            document(
                "file:///work/My/Pkg/DeepA.elm",
                "module My.Pkg.DeepA exposing (T)\n\ntype alias T = Int\n",
            ),
            document(
                "file:///work/My/Pkg/DeepB.elm",
                "module My.Pkg.DeepB exposing (T)\n\ntype alias T = String\n",
            ),
        ],
    );

    assert_eq!(
        accesses(&result),
        [
            ("api".to_string(), "Public".to_string()),
            ("deep-a".to_string(), "Public".to_string()),
            ("deep-b".to_string(), "Public".to_string()),
            ("hidden".to_string(), "Public".to_string()),
        ]
        .into_iter()
        .collect()
    );
}

/// A type that refers to itself, and a pair that refer to each other, do not
/// send the walk round for ever: a declaration is followed once.
///
/// The cycle is inside one module because that is the only place this frontend
/// can have one — two modules that referred to each other would have to import
/// each other, which is refused as `ELM_IMPORT_CYCLE` long before this runs.
#[test]
fn a_reference_cycle_inside_a_published_module_terminates() {
    let result = compile_exposing(
        "My.Pkg",
        &["Api"],
        vec![
            document(
                "file:///work/My/Pkg/Api.elm",
                "module My.Pkg.Api exposing (..)\n\n\
                 import My.Pkg.Knot\n\n\
                 type alias Entry = My.Pkg.Knot.Tree\n",
            ),
            document(
                "file:///work/My/Pkg/Knot.elm",
                "module My.Pkg.Knot exposing (Tree(..), Odd(..), Even(..))\n\n\
                 import My.Pkg.Tail\n\n\n\
                 type Tree = Leaf | Branch Tree Odd\n\n\n\
                 type Odd = Odd Even\n\n\n\
                 type Even = Even Odd My.Pkg.Tail.End\n",
            ),
            document(
                "file:///work/My/Pkg/Tail.elm",
                "module My.Pkg.Tail exposing (End)\n\ntype alias End = Int\n",
            ),
        ],
    );

    // `Tail` is reached only through `Knot.Even`, which is reached only through
    // `Knot.Odd` — so the cycle has to be walked all the way, once, for it to
    // be published at all.
    assert_eq!(
        accesses(&result),
        [
            ("api".to_string(), "Public".to_string()),
            ("knot".to_string(), "Public".to_string()),
            ("tail".to_string(), "Public".to_string()),
        ]
        .into_iter()
        .collect()
    );
}

/// Only what a module actually publishes counts. A private type's body, and an
/// opaque custom type's constructor arguments, show a dependent nothing, so
/// neither publishes the module it names.
#[test]
fn a_reference_a_module_does_not_publish_exposes_nothing() {
    let result = compile_exposing(
        "My.Pkg",
        &["Api"],
        vec![
            document(
                "file:///work/My/Pkg/Api.elm",
                "module My.Pkg.Api exposing (Opaque)\n\n\
                 import My.Pkg.Inner\n\n\
                 type Opaque = Opaque My.Pkg.Inner.Hidden\n\n\n\
                 type alias Private = My.Pkg.Inner.Hidden\n",
            ),
            document(
                "file:///work/My/Pkg/Inner.elm",
                "module My.Pkg.Inner exposing (Hidden)\n\ntype alias Hidden = Int\n",
            ),
        ],
    );

    assert_eq!(
        accesses(&result),
        [
            ("api".to_string(), "Public".to_string()),
            ("inner".to_string(), "Private".to_string()),
        ]
        .into_iter()
        .collect()
    );
}

const DOCS: &str = "module Example exposing (..)\n\n\
                    {-| Values of several kinds.\n-}\n\n\
                    import Dict\n\n\n\
                    {-|    Leading and trailing whitespace in a doc.   \n-}\n\
                    type alias Padded =\n    Int\n\n\n\
                    {-| First line.\n\n  - a bullet\n\n-}\n\
                    type alias Spread =\n    Int\n\n\n\
                    {-| | Doc with a leading bar.\n-}\n\
                    type alias Barred =\n    Int\n\n\n\
                    type alias Undocumented =\n    Int\n";

/// The default mode writes what morphir-elm writes: the text between the
/// delimiters, with every space and newline of its own kept. An undocumented
/// type carries the empty string a classic document uses, and a module with no
/// doc carries `null`.
#[test]
fn the_default_doc_comment_mode_keeps_the_text_morphir_elm_keeps() {
    for mode in [None, Some("morphir-elm")] {
        let (module_doc, types) = v3_docs(&compile_with_doc_mode(mode, DOCS));

        assert_eq!(module_doc, serde_json::json!(" Values of several kinds."));
        assert_eq!(
            types["padded"],
            serde_json::json!("    Leading and trailing whitespace in a doc.   \n")
        );
        // A declaration's doc loses the delimiters and nothing else, so the
        // blank line in front of `-}` is part of it. A *module* doc loses one
        // character more — morphir-elm's `String.dropRight 3` — which is why
        // the module doc above has no trailing newline.
        assert_eq!(
            types["spread"],
            serde_json::json!(" First line.\n\n  - a bullet\n\n")
        );
        assert_eq!(
            types["barred"],
            serde_json::json!(" | Doc with a leading bar.\n")
        );
        assert_eq!(types["undocumented"], serde_json::json!(""));
    }
}

/// `trimmed` takes the surrounding whitespace off, which is what this frontend
/// did before the mode existed.
#[test]
fn the_trimmed_doc_comment_mode_takes_the_surrounding_whitespace_off() {
    let (module_doc, types) = v3_docs(&compile_with_doc_mode(Some("trimmed"), DOCS));

    assert_eq!(module_doc, serde_json::json!("Values of several kinds."));
    assert_eq!(
        types["padded"],
        serde_json::json!("Leading and trailing whitespace in a doc.")
    );
    assert_eq!(
        types["spread"],
        serde_json::json!("First line.\n\n  - a bullet")
    );
    assert_eq!(
        types["barred"],
        serde_json::json!("| Doc with a leading bar.")
    );
    assert_eq!(types["undocumented"], serde_json::json!(""));
}

/// An undocumented module carries `null` in either mode.
#[test]
fn an_undocumented_module_has_no_doc_in_either_mode() {
    let source = "module Example exposing (..)\n\ntype alias T =\n    Int\n";
    for mode in [Some("morphir-elm"), Some("trimmed")] {
        let (module_doc, types) = v3_docs(&compile_with_doc_mode(mode, source));
        assert_eq!(module_doc, serde_json::Value::Null, "{mode:?}");
        assert_eq!(types["t"], serde_json::json!(""), "{mode:?}");
    }
}

/// A mode nobody implements is a request this extension cannot act on, and is
/// refused by name rather than quietly falling back to the default.
#[test]
fn an_unknown_doc_comment_mode_is_refused() {
    let result = compile_with_doc_mode(Some("verbatim"), DOCS);

    assert!(!result.success);
    let refusals = codes(&result, "ELM_REQUEST");
    assert_eq!(refusals.len(), 1, "{:?}", result.diagnostics);
    assert!(
        refusals[0].message.contains("elmDocComments") && refusals[0].message.contains("verbatim"),
        "{}",
        refusals[0].message
    );
    assert!(result.ir.is_none());
}

/// A module's interface is what its dependents can observe, and a doc is not
/// part of it: editing only a doc comment must not make every dependent
/// recompile. The doc *is* part of the compile context, though, so switching
/// modes invalidates the baseline as a whole.
#[test]
fn a_doc_only_edit_does_not_change_the_interface_digest() {
    let before = compile_with_doc_mode(None, DOCS);
    let after = compile_with_doc_mode(None, &DOCS.replace("First line.", "A different line."));

    assert_ne!(before.ir, after.ir, "the doc text itself did change");
    assert_eq!(
        before.module_results[0].interface_digest,
        after.module_results[0].interface_digest
    );
    assert_eq!(
        before.context_digest, after.context_digest,
        "the same options are the same context"
    );
    assert_ne!(
        before.context_digest,
        compile_with_doc_mode(Some("trimmed"), DOCS).context_digest,
        "a different doc comment mode is a different context"
    );
}

/// Two modules whose declarations are deliberately not in alphabetical order,
/// including a custom type whose constructors are not either.
fn unsorted_package() -> Vec<SourceDocument> {
    vec![
        document(
            "file:///work/My/Pkg/Shared.elm",
            "module My.Pkg.Shared exposing (..)\n\n\
             type alias Money = Int\n\n\n\
             type Currency = USD | EUR | GBP\n\n\n\
             type alias Code = String\n",
        ),
        document(
            "file:///work/My/Pkg/Aliases.elm",
            "module My.Pkg.Aliases exposing (..)\n\n\
             type alias Zeta = Int\n\n\n\
             type alias Alpha = Int\n",
        ),
    ]
}

/// The default writes everything in the order the source declares it: modules
/// in request order, types and constructors as written.
#[test]
fn the_default_ordering_is_source_order() {
    for order in [None, Some("source")] {
        assert_eq!(
            v3_layout(&compile_ordered(order, unsorted_package())),
            vec![
                (
                    "shared".to_string(),
                    vec![
                        "money".to_string(),
                        "currency".to_string(),
                        "code".to_string()
                    ],
                    vec![vec![
                        "u-s-d".to_string(),
                        "e-u-r".to_string(),
                        "g-b-p".to_string()
                    ]]
                ),
                (
                    "aliases".to_string(),
                    vec!["zeta".to_string(), "alpha".to_string()],
                    vec![]
                ),
            ],
            "{order:?}"
        );
    }
}

/// `morphir-elm` writes them the way morphir-elm's `Dict`s do: modules, types
/// and constructors sorted by their key.
#[test]
fn the_morphir_elm_ordering_sorts_modules_types_and_constructors() {
    assert_eq!(
        v3_layout(&compile_ordered(Some("morphir-elm"), unsorted_package())),
        vec![
            (
                "aliases".to_string(),
                vec!["alpha".to_string(), "zeta".to_string()],
                vec![]
            ),
            (
                "shared".to_string(),
                vec![
                    "code".to_string(),
                    "currency".to_string(),
                    "money".to_string()
                ],
                // `["e","u","r"] < ["g","b","p"] < ["u","s","d"]`, which is the
                // order morphir-elm writes for this very type.
                vec![vec![
                    "e-u-r".to_string(),
                    "g-b-p".to_string(),
                    "u-s-d".to_string()
                ]]
            ),
        ]
    );
}

/// The sort is on the *words* a Morphir name holds, not on a rendered spelling
/// of it. `LocalDate` is `["local","date"]` and `Locale` is `["locale"]`, so
/// `Locale` sorts *after* `LocalDate` — `"local" < "locale"` element by
/// element — where a rendered `"localdate" < "locale"` comparison would agree
/// by luck, but `ListOf` (`["list","of"]`) against `Listen` (`["listen"]`)
/// would not: joined, `"listen" < "listof"`; by words, `["list","of"]` comes
/// first because `"list" < "listen"`.
#[test]
fn the_morphir_elm_ordering_sorts_on_words_not_on_a_rendered_name() {
    let result = compile_ordered(
        Some("morphir-elm"),
        vec![document(
            "file:///work/My/Pkg/Words.elm",
            "module My.Pkg.Words exposing (..)\n\n\
             type alias Listen = Int\n\n\n\
             type alias ListOf = Int\n\n\n\
             type alias Locale = Int\n\n\n\
             type alias LocalDate = Int\n",
        )],
    );

    assert_eq!(
        v3_layout(&result)[0].1,
        vec![
            "list-of".to_string(),
            "listen".to_string(),
            "local-date".to_string(),
            "locale".to_string(),
        ],
        "a rendered-name sort would give listen, list-of, locale, local-date"
    );
}

/// An order nobody implements is a request this extension cannot act on, and is
/// refused by name rather than quietly falling back to the default.
#[test]
fn an_unknown_ordering_is_refused() {
    let result = compile_ordered(Some("alphabetical"), unsorted_package());

    assert!(!result.success);
    let refusals = codes(&result, "ELM_REQUEST");
    assert_eq!(refusals.len(), 1, "{:?}", result.diagnostics);
    assert!(
        refusals[0].message.contains("elmOrdering") && refusals[0].message.contains("alphabetical"),
        "{}",
        refusals[0].message
    );
    assert!(result.ir.is_none());
}

/// The ordering changes the document, so a baseline built under one order is
/// not reusable by a run compiling under the other.
#[test]
fn the_two_orderings_are_two_compile_contexts() {
    assert_ne!(
        compile_ordered(None, unsorted_package()).context_digest,
        compile_ordered(Some("morphir-elm"), unsorted_package()).context_digest
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
fn the_extension_advertises_the_elm_frontend_backend_and_workspace() {
    let info = ElmExtension::info();
    assert_eq!(info.id, "morphir-elm-native");
    assert_eq!(info.name, "Morphir Elm (native)");
    assert_eq!(
        info.types,
        vec![
            ExtensionType::Frontend,
            ExtensionType::Backend,
            ExtensionType::Workspace,
        ]
    );

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

    let workspace = capabilities.workspace.expect("a workspace capability");
    assert_eq!(
        workspace.protocol_versions,
        vec![morphir_workspace::workspace_discovery_protocol()]
    );
    assert!(workspace.discover);
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
    let extension = NativeExtension::builder(ElmExtension)
        .with_frontend()
        .with_backend()
        .with_workspace()
        .finish()
        .unwrap();
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
