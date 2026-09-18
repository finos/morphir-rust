//! End-to-end frontend acceptance: a compile request in, a Morphir IR
//! distribution out, through the same `NativeExtension` handle the daemon uses.

use std::collections::BTreeSet;

use morphir_elm_binding::ElmExtension;
use morphir_extension_sdk::prelude::*;

const TYPES: &str = include_str!("fixtures/Types.elm");
const OTHER: &str = "module My.Other exposing (Thing)\n\ntype Thing = Thing\n";
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
    compile_as("elm", documents, ir_version)
}

fn compile_as(
    language_id: &str,
    documents: Vec<SourceDocument>,
    ir_version: &str,
) -> CompileResult {
    let extension = NativeExtension::frontend_backend(ElmExtension).unwrap();
    extension
        .frontend()
        .unwrap()
        .compile(CompileRequest {
            language_id: language_id.into(),
            documents,
            package: CompilePackage {
                name: "local/example".into(),
                exposed_modules: None,
            },
            dependencies: vec![],
            options: CompileOptions {
                types_only: false,
                ir_version: ir_version.into(),
                extra: Default::default(),
            },
            baseline: None,
        })
        .unwrap()
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
        vec![document("file:///work/My/Other.elm", OTHER)],
        "3",
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

#[test]
fn generation_is_not_implemented_yet() {
    let extension = NativeExtension::frontend_backend(ElmExtension).unwrap();
    let result = extension
        .backend()
        .unwrap()
        .generate(GenerateRequest {
            ir: serde_json::json!({"formatVersion": 3}),
            target: "elm".into(),
            options: Default::default(),
        })
        .unwrap();

    assert!(!result.success);
    assert!(result.artifacts.is_empty());
    assert_eq!(
        result.diagnostics[0].code.as_deref(),
        Some("ELM_UNSUPPORTED")
    );
}
