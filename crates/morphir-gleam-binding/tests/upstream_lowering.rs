use morphir_extension_sdk::prelude::*;
use morphir_gleam_binding::GleamExtension;
use std::collections::HashMap;

fn request(source: &str, version: &str, types_only: bool) -> CompileRequest {
    CompileRequest {
        language_id: "gleam".into(),
        documents: vec![document("main", source)],
        package: CompilePackage {
            name: "example/package".into(),
            exposed_modules: None,
        },
        options: CompileOptions {
            ir_version: version.into(),
            types_only,
            extra: HashMap::from([("emitParseStage".into(), false.into())]),
        },
        ..Default::default()
    }
}
fn document(name: &str, source: &str) -> SourceDocument {
    SourceDocument {
        uri: format!("file:///workspace/src/{name}.gleam"),
        language_id: "gleam".into(),
        version: 1,
        text: source.into(),
    }
}

#[test]
fn unsupported_valid_syntax_has_a_source_located_lowering_diagnostic() {
    let source = "// 😀\npub type Model = Int\npub fn run() {\n  todo as \"later\"\n}";
    let result = GleamExtension.compile(request(source, "4", false)).unwrap();
    assert!(!result.success);
    assert!(result.ir.is_none());
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code.as_deref() == Some("GLEAM_UNSUPPORTED_VALUE"))
        .expect("unsupported syntax diagnostic");
    let location = diagnostic.location.as_ref().expect("source location");
    assert_eq!(location.uri, "file:///workspace/src/main.gleam");
    assert_eq!(location.range.start.line, 3);
    assert_eq!(location.range.start.character, 2);
    assert!(location.range.end.character > 2);
}

#[test]
fn unsupported_values_do_not_prevent_types_only_or_ir3_output() {
    for (version, types_only) in [("4", true), ("3", false)] {
        let result = GleamExtension
            .compile(request(
                "pub type Model = Int\npub fn run() { todo }",
                version,
                types_only,
            ))
            .unwrap();
        assert!(result.success, "{:?}", result.diagnostics);
        assert!(result.ir.is_some());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("GLEAM_VALUE_SKIPPED"))
        );
    }
}

#[test]
fn unsupported_nodes_cannot_hide_in_discarded_statements_or_list_tails() {
    for source in [
        "pub fn run() { todo\n  1 }",
        "pub fn run() { let ignored = todo\n  1 }",
        "pub fn run() { [1, ..{ todo }] }",
        "pub fn run() { case 1 { 1 -> todo\n _ -> 2 } }",
    ] {
        let result = GleamExtension.compile(request(source, "4", false)).unwrap();
        assert!(!result.success, "must reject: {source}");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("GLEAM_UNSUPPORTED_VALUE")),
            "{:?}",
            result.diagnostics
        );
    }
}

#[test]
fn labelled_calls_fail_instead_of_silently_using_source_order() {
    let result = GleamExtension.compile(request("pub fn run() { subtract(right: 1, left: 2) }\npub fn subtract(left a: Int, right b: Int) -> Int { a - b }", "4", false)).unwrap();
    assert!(!result.success);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("GLEAM_UNSUPPORTED_VALUE")
                && d.message.contains("labelled call arguments")),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn labelled_function_declarations_fail_instead_of_discarding_external_labels() {
    let source = "pub type Model = Int\npub fn rename(label value: Int) -> Int { value }";
    let result = GleamExtension.compile(request(source, "4", false)).unwrap();
    assert!(
        !result.success,
        "external parameter labels must not be discarded"
    );
    assert!(result.ir.is_none());
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|d| d.code.as_deref() == Some("GLEAM_UNSUPPORTED_VALUE"))
        .expect("explicit labelled declaration limitation");
    assert!(diagnostic.message.contains("labelled function parameters"));
    assert_eq!(diagnostic.location.as_ref().unwrap().range.start.line, 1);
    for (version, types_only) in [("4", true), ("3", false)] {
        let result = GleamExtension
            .compile(request(source, version, types_only))
            .unwrap();
        assert!(result.success, "{:?}", result.diagnostics);
        assert!(result.ir.is_some());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("GLEAM_VALUE_SKIPPED"))
        );
    }
}

#[test]
fn unused_value_imports_are_retained_but_uses_require_value_resolution() {
    let mut unused = request(
        "import other.{answer as imported}\npub type Model = Int",
        "4",
        false,
    );
    unused
        .documents
        .push(document("other", "pub fn answer() { 42 }"));
    let result = GleamExtension.compile(unused.clone()).unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
    for expression in ["imported()", "other.answer()"] {
        let mut used = unused.clone();
        used.documents[0]
            .text
            .push_str(&format!("\npub fn run() {{ {expression} }}"));
        let result = GleamExtension.compile(used).unwrap();
        assert!(!result.success, "{expression}");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("GLEAM_UNSUPPORTED_VALUE")
                    && d.message.contains("imported")),
            "{:?}",
            result.diagnostics
        );
    }
    let mut types_only = unused;
    types_only.documents[0]
        .text
        .push_str("\npub fn run() { imported() }");
    types_only.options.types_only = true;
    assert!(GleamExtension.compile(types_only).unwrap().success);
}

#[test]
fn local_parameters_shadow_imported_values_without_false_diagnostics() {
    let mut compile = request(
        "import other.{answer as imported}\npub fn run(imported: Int) -> Int { imported }",
        "4",
        false,
    );
    compile
        .documents
        .push(document("other", "pub fn answer() { 42 }"));
    let result = GleamExtension.compile(compile).unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
}

#[test]
fn ordinary_literals_and_function_parameters_keep_existing_lowering() {
    let result = GleamExtension
        .compile(request(
            "pub fn identity(value: Int) -> Int { value }\npub const answer: Int = 42",
            "4",
            false,
        ))
        .unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(result.module_results[0].status, ModuleStatus::Compiled);
}

#[test]
fn imported_module_access_cannot_hide_behind_a_non_record_parameter() {
    let mut compile = request(
        "import other\npub fn run(other: Int) { other.answer() }",
        "4",
        false,
    );
    compile
        .documents
        .push(document("other", "pub fn answer() { 42 }"));
    let result = GleamExtension.compile(compile).unwrap();
    assert!(!result.success);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("GLEAM_UNSUPPORTED_VALUE")),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn imported_constructor_qualifiers_are_not_local_variable_bindings() {
    let mut compile = request(
        "import other\npub fn run(other: Int) { other.Value }",
        "4",
        false,
    );
    compile
        .documents
        .push(document("other", "pub type Value { Value }"));
    let result = GleamExtension.compile(compile).unwrap();
    assert!(!result.success);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("GLEAM_UNSUPPORTED_VALUE")),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn constants_requiring_inference_or_function_kind_are_explicitly_unsupported() {
    for declaration in [
        "pub const answer = 42",
        "pub const handler: fn(Int) -> Int = identity",
    ] {
        let source = format!(
            "pub type Model = Int\npub fn identity(value: Int) -> Int {{ value }}\n{declaration}"
        );
        let result = GleamExtension
            .compile(request(&source, "4", false))
            .unwrap();
        assert!(
            !result.success,
            "must not fabricate a type/signature for {declaration}"
        );
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|d| d.code.as_deref() == Some("GLEAM_UNSUPPORTED_VALUE"))
            .expect("explicit constant limitation");
        assert!(diagnostic.message.contains("constant"));
        assert_eq!(diagnostic.location.as_ref().unwrap().range.start.line, 2);
        for (version, types_only) in [("4", true), ("3", false)] {
            let result = GleamExtension
                .compile(request(&source, version, types_only))
                .unwrap();
            assert!(result.success, "{:?}", result.diagnostics);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|d| d.code.as_deref() == Some("GLEAM_VALUE_SKIPPED"))
            );
        }
    }
}

#[test]
fn new_constant_forms_do_not_inherit_legacy_function_lowering_shortcuts() {
    for declaration in [
        "pub const original: Int = 1\npub const copied: Int = original",
        "pub const joined: String = \"a\" <> \"b\"",
        "pub const values: List(Int) = [1, ..[2]]",
        "pub const nothing: Nil = Nil",
    ] {
        let source = format!("pub type Model = Int\n{declaration}");
        let result = GleamExtension
            .compile(request(&source, "4", false))
            .unwrap();
        assert!(!result.success, "must reject {declaration}");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("GLEAM_UNSUPPORTED_VALUE")),
            "{:?}",
            result.diagnostics
        );
        for (version, types_only) in [("4", true), ("3", false)] {
            let result = GleamExtension
                .compile(request(&source, version, types_only))
                .unwrap();
            assert!(result.success, "{:?}", result.diagnostics);
        }
    }
}
