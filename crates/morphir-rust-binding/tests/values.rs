use morphir_extension_sdk::{
    CompileOptions, CompilePackage, CompileRequest, CompileResult, Frontend, SourceDocument,
    SourceSet,
};
use morphir_rust_binding::RustExtension;

fn compile(source: &str, version: &str, types_only: bool) -> CompileResult {
    RustExtension
        .compile(CompileRequest {
            language_id: "rust".into(),
            sources: SourceSet {
                root: None,
                documents: vec![SourceDocument {
                    uri: "file:///src/models.rs".into(),
                    language_id: "rust".into(),
                    text: source.into(),
                    ..Default::default()
                }],
            },
            package: CompilePackage {
                name: "acme/example".into(),
                exposed_modules: Some(vec!["Models".into()]),
            },
            options: CompileOptions {
                types_only,
                ir_version: version.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap()
}

#[test]
fn conditional_values_compile_in_both_versions() {
    for version in ["3", "4"] {
        for source in [
            "pub fn pick(x: i64, y: i64) -> i64 { if x < y { x } else { y } }",
            "pub fn f(a: bool, b: bool) -> (bool, i64) { let c = a && !b; if c || b { (true, -9223372036854775808) } else { (false, 12i64) } }",
            "pub fn f(x: i64) -> i64 { let x = x; let x = if x == 1 { 2 } else { x }; x }",
            "pub fn f<T>(a: bool, x: T, y: T) -> T { if a { x } else { y } }",
            "pub fn f() {}",
            "pub fn f() -> (f64, char) { (1.25, 'a') }",
            "pub fn f() -> (f64, f64) { (1f64, -2f64) }",
        ] {
            let result = compile(source, version, false);
            assert!(
                result.success,
                "{version} {source}: {:?}",
                result.diagnostics
            );
            assert!(result.ir.is_some());
        }
    }
}

#[test]
fn rejects_unsupported_or_ill_typed_expressions_without_partial_ir() {
    for source in [
        "fn f() -> i64 { missing }",
        "fn f() -> i64 { true }",
        "fn f() -> i64 { if 1 { 2 } else { 3 } }",
        "fn f() -> i64 { if true { 2 } else { false } }",
        "fn f() -> i64 { 1 + 2 }",
        "fn f() -> bool { 1 && true }",
        "fn f() -> bool { !1 }",
        "fn f() -> bool { 1 == false }",
        "fn f() -> i64 { let mut x = 1; x }",
        "fn f() -> i64 { let x = x; x }",
        "fn f() -> i64 { if true { let x = 1; x } else { x } }",
        "fn f() -> i64 { 9223372036854775808 }",
        "fn f() -> f64 { 1e999 }",
        "fn f() -> String { \"borrowed\" }",
        "fn f() -> i64 { 1u32 }",
        "async fn f() -> i64 { 1 }",
        "fn f(mut x: i64) -> i64 { x }",
        "fn f() -> i64 { return 1; }",
        "fn f() -> i64 { other() }",
        "fn f() -> i64 { let x = 1; x = 2; x }",
        "fn f() -> i64 { let x: bool = 1; x }",
        "fn f<T>(x: T) -> (T, T) { (x, x) }",
        "fn f<T>() -> i64 { 1 }",
        "fn f<T, U>(x: T) -> T { x }",
        "fn f() {} fn f() {}",
    ] {
        for version in ["3", "4"] {
            let result = compile(source, version, false);
            assert!(!result.success, "accepted {source}");
            assert!(result.ir.is_none());
            assert!(result.diagnostics.iter().any(|d| d.location.is_some()));
        }
    }
}

#[test]
fn expressions_preserve_both_binding_kinds_and_short_circuit_structure() {
    let result = compile(
        r#"
        #[morphir::native(hint="comparison")] pub fn host(x: i64) -> bool { ignored() }
        pub fn choose(a: bool, b: bool) -> bool { a && b || !a }
    "#,
        "4",
        false,
    );
    assert!(result.success, "{:?}", result.diagnostics);
    let text = result.ir.unwrap().to_string();
    assert!(text.contains("NativeBody"));
    assert!(text.contains("ExpressionBody"));
    assert!(text.contains("IfThenElse"));
    assert!(!text.contains("#and"));
    assert!(!text.contains("#or"));
}

#[test]
fn source_validation_covers_attributes_and_function_namespace() {
    for source in [
        "#[inline] fn f() {}",
        "fn foo_bar() {} fn fooBar() {}",
        "pub struct FooBar; pub fn foo_bar() {}",
        "fn f(#[cfg(any())] x: bool) -> bool { x }",
        "type Number = i64; fn f() -> Number { 1 }",
        "fn f() -> bool { #[cfg(any())] (true) }",
        r#"#[morphir::native(hint="comparison")] fn foo_bar() {} fn fooBar() {}"#,
    ] {
        let result = compile(source, "4", false);
        assert!(!result.success, "accepted {source}");
        assert!(result.ir.is_none());
        assert!(result.diagnostics.iter().any(|d| d.location.is_some()));
    }
}

#[test]
fn executable_signatures_do_not_erase_box_ownership() {
    for source in [
        "fn f(x: Box<bool>) -> i64 { if x { 1 } else { 2 } }",
        "fn f(x: Box<i64>) -> (Box<i64>, Box<i64>) { (x, x) }",
        "fn f(x: (i64, Box<i64>)) -> (i64, Box<i64>) { x }",
        "fn f() -> i64 { let x: Box<i64> = 1; x }",
    ] {
        for version in ["3", "4"] {
            assert!(
                !compile(source, version, false).success,
                "accepted {source}"
            );
            assert!(compile(source, version, true).success);
        }
    }
    assert!(compile("struct Box; fn f(x: Box) -> Box { x }", "4", false).success);
}
