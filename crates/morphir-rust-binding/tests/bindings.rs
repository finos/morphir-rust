use morphir_core::ir::v4::{IRFile, TypeEncoding, with_type_encoding};
use morphir_extension_sdk::{
    CompileOptions, CompilePackage, CompileRequest, CompileResult, DiagnosticSeverity, Frontend,
    SourceDocument,
};
use morphir_rust_binding::RustExtension;
use serde_json::{Value, json};

fn compile(source: &str, version: &str, types_only: bool) -> CompileResult {
    RustExtension
        .compile(CompileRequest {
            language_id: "rust".into(),
            documents: vec![SourceDocument {
                uri: "file:///src/models.rs".into(),
                language_id: "rust".into(),
                text: source.into(),
                ..Default::default()
            }],
            package: CompilePackage {
                name: "acme/example".into(),
                exposed_modules: vec!["Models".into()],
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

fn values(source: &str) -> Value {
    let result = compile(source, "4", false);
    assert!(result.success, "{source}: {:?}", result.diagnostics);
    let ir = result.ir.unwrap();
    let parsed: IRFile = serde_json::from_value(ir).unwrap();
    let ir = with_type_encoding(TypeEncoding::Compact, || {
        serde_json::to_value(parsed).unwrap()
    });
    ir["distribution"]["Library"]["def"]["modules"]["models"]["Public"]["values"].clone()
}

fn reject(source: &str, version: &str, types_only: bool) -> CompileResult {
    let result = compile(source, version, types_only);
    assert!(
        !result.success,
        "accepted {source}, version={version}, types_only={types_only}"
    );
    assert!(result.ir.is_none());
    assert!(result.modules.is_empty());
    assert!(
        result.diagnostics.iter().any(|d| d.location.is_some()),
        "{:?}",
        result.diagnostics
    );
    result
}

#[test]
fn frontend_preserves_expanded_type_encoding_for_native_signatures() {
    let result = compile(
        r#"#[morphir::native(hint="comparison")] pub fn equal<T>(left: T, right: T) -> bool {}"#,
        "4",
        false,
    );
    assert!(result.success, "{:?}", result.diagnostics);
    let ir = result.ir.unwrap();
    let body = &ir["distribution"]["Library"]["def"]["modules"]["models"]["Public"]["values"]["equal"]
        ["Public"]["NativeBody"];
    assert_eq!(
        body["inputTypes"],
        json!({
            "left": {"Variable": {"name": "t"}},
            "right": {"Variable": {"name": "t"}}
        })
    );
    assert_eq!(
        body["outputType"],
        json!({
            "Reference": {"fqname": "morphir/SDK:basics#bool"}
        })
    );
}

#[test]
fn native_binding_emits_canonical_signature_documentation_and_visibility() {
    let values = values(
        r#"
        /// Add two integers.
        #[morphir::native(hint = "arithmetic", description = "Host addition")]
        pub fn add_numbers(left_value: i64, right: i64) -> i64 { unknown_macro!(); }
        #[morphir::native(hint = "comparison")]
        fn is_equal<T>(left: T, right: T) -> bool { this_body_is_not_lowered() }
    "#,
    );
    assert_eq!(
        values["add-numbers"]["Public"],
        json!({
            "doc": "Add two integers.",
            "NativeBody": {
                "inputTypes": {"left-value": "morphir/SDK:basics#int", "right": "morphir/SDK:basics#int"},
                "outputType": "morphir/SDK:basics#int",
                "nativeInfo": {"hint": {"Arithmetic": {}}, "description": "Host addition"}
            }
        })
    );
    assert_eq!(
        values["is-equal"]["Private"],
        json!({"NativeBody": {
            "inputTypes": {"left": "t", "right": "t"},
            "outputType": "morphir/SDK:basics#bool",
            "nativeInfo": {"hint": {"Comparison": {}}}
        }})
    );
}

#[test]
fn every_native_hint_and_implicit_unit_result_are_supported() {
    for (hint, expected) in [
        ("arithmetic", "Arithmetic"),
        ("comparison", "Comparison"),
        ("string_op", "StringOp"),
        ("collection_op", "CollectionOp"),
    ] {
        let source = format!("#[morphir::native(hint = \"{hint}\")] pub fn f() {{}} ");
        let actual = values(&source);
        let body = &actual["f"]["Public"]["NativeBody"];
        assert_eq!(body["nativeInfo"], json!({"hint": {expected: {}}}));
        assert_eq!(body["inputTypes"], json!({}));
        assert_eq!(body["outputType"], json!({"Unit": {}}));
        assert!(body.get("body").is_none());
    }
    let actual = values(
        r#"#[morphir::native(hint="platform_specific", platform="wasm32-unknown-unknown")] pub fn f() {}"#,
    );
    assert_eq!(
        actual["f"]["Public"]["NativeBody"]["nativeInfo"],
        json!({
            "hint": {"PlatformSpecific": {"platform": "wasm32-unknown-unknown"}}
        })
    );
}

#[test]
fn external_bindings_preserve_order_and_opaque_names_without_fallback() {
    let actual = values(
        r#"
        /// Host addition.
        #[morphir::external(target="rust", name="vendor::math::add")]
        #[morphir::external(target="JavaScript custom", name="vendor.math['add']")]
        pub fn add(left: i64, right: i64) -> i64 { left + right }
    "#,
    );
    assert_eq!(
        actual["add"]["Public"],
        json!({
            "doc": "Host addition.",
            "ExternalBody": {
                "inputTypes": {"left": "morphir/SDK:basics#int", "right": "morphir/SDK:basics#int"},
                "outputType": "morphir/SDK:basics#int",
                "externals": [
                    {"targetPlatform": "rust", "externalName": "vendor::math::add"},
                    {"targetPlatform": "JavaScript custom", "externalName": "vendor.math['add']"}
                ]
            }
        })
    );
}

#[test]
fn binding_signatures_reuse_local_and_sdk_types_and_raw_identifiers() {
    let actual = values(
        r#"
        pub struct Node;
        #[morphir::external(target="rust", name="host::convert")]
        pub fn r#convert<T>(r#type: Vec<T>, node: Box<Node>) -> Result<Option<T>, String> { unimplemented!() }
    "#,
    );
    let body = &actual["convert"]["Public"]["ExternalBody"];
    assert_eq!(
        body["inputTypes"],
        json!({
            "type": {"Reference": ["morphir/SDK:list#list", "t"]},
            "node": "acme/example:models#node"
        })
    );
    assert_eq!(
        body["outputType"],
        json!({"Reference": [
            "morphir/SDK:result#result", "morphir/SDK:string#string",
            {"Reference": ["morphir/SDK:maybe#maybe", "t"]}
        ]})
    );
}

#[test]
fn binding_attributes_are_strict_and_mutually_exclusive_even_when_types_only() {
    for attrs in [
        "#[morphir::native]",
        "#[morphir::native()]",
        r#"#[morphir::native(hint="unknown")]"#,
        r#"#[morphir::native(hint="arithmetic", hint="comparison")]"#,
        r#"#[morphir::native(hint=1)]"#,
        r#"#[morphir::native(hint="arithmetic", description=1)]"#,
        r#"#[morphir::native(hint="arithmetic", extra="x")]"#,
        r#"#[morphir::native(hint="arithmetic", platform="rust")]"#,
        r#"#[morphir::native(hint="platform_specific")]"#,
        r#"#[morphir::native(hint="platform_specific", platform="  ")]"#,
        r#"#[morphir::native(hint="arithmetic")] #[morphir::native(hint="arithmetic")]"#,
        r#"#[morphir::native(hint="arithmetic")] #[morphir::external(target="rust", name="f")]"#,
        "#[morphir::external]",
        "#[morphir::external()]",
        r#"#[morphir::external(target="rust")]"#,
        r#"#[morphir::external(name="f")]"#,
        r#"#[morphir::external(target="", name="f")]"#,
        r#"#[morphir::external(target="rust", name=" \t")]"#,
        r#"#[morphir::external(target="rust", name=3)]"#,
        r#"#[morphir::external(target="rust", name="f", unknown="x")]"#,
        r#"#[morphir::external(target="rust", target="js", name="f")]"#,
        r#"#[morphir::external(target="rust", name="f", name="g")]"#,
        r#"#[morphir::external(target="rust", name="f")] #[morphir::external(target="rust", name="g")]"#,
    ] {
        let source = format!("{attrs} pub fn f() {{}}");
        for version in ["3", "4"] {
            for types_only in [false, true] {
                reject(&source, version, types_only);
            }
        }
    }
}

#[test]
fn annotated_signatures_reject_unsupported_rust_forms_even_when_types_only() {
    for signature in [
        "pub async fn f()",
        "pub const fn f()",
        "pub unsafe fn f()",
        "pub extern \"C\" fn f()",
        "pub fn f(args: ...)",
        "pub fn f(self)",
        "pub fn f((a, b): (i64, i64))",
        "pub fn f(ref a: i64)",
        "pub fn f(mut a: i64)",
        "pub fn f(_: i64)",
        "pub fn f<'a>()",
        "pub fn f<T: Clone>(x: T)",
        "pub fn f<T>(x: T) where T: Clone",
        "pub fn f<const N: usize>()",
        "pub fn f(x: &str)",
        "pub fn f() -> Unknown",
        "pub fn f<T, T>(x: T)",
        "pub fn f(foo_bar: i64, fooBar: i64)",
    ] {
        for attr in [
            r#"#[morphir::native(hint="arithmetic")]"#,
            r#"#[morphir::external(target="rust", name="host::f")]"#,
        ] {
            let source = format!("{attr} {signature} {{ unimplemented!() }}");
            for version in ["3", "4"] {
                for types_only in [false, true] {
                    reject(&source, version, types_only);
                }
            }
        }
    }
}

#[test]
fn annotated_values_are_v4_only_but_validated_and_omitted_in_types_only_mode() {
    for attr in [
        r#"#[morphir::native(hint="arithmetic")]"#,
        r#"#[morphir::external(target="rust", name="f")]"#,
    ] {
        let source = format!("pub struct Model; {attr} pub fn f() {{}}");
        let result = reject(&source, "3", false);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("RS_BINDING_VERSION"))
        );
        for version in ["3", "4"] {
            let result = compile(&source, version, true);
            assert!(result.success, "{:?}", result.diagnostics);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|d| d.code.as_deref() == Some("RS_VALUES_UNSUPPORTED")
                        && d.severity == DiagnosticSeverity::Warning)
            );
            let ir = result.ir.unwrap();
            if version == "4" {
                let _: morphir_core::ir::v4::IRFile = serde_json::from_value(ir.clone()).unwrap();
                assert_eq!(
                    ir["distribution"]["Library"]["def"]["modules"]["models"]["Public"]["values"],
                    json!({})
                );
            } else {
                assert_eq!(
                    ir["distribution"][3]["modules"][0][1]["value"]["values"],
                    json!([])
                );
            }
        }
    }
    for version in ["3", "4"] {
        reject("pub fn ordinary() {}", version, false);
        assert!(compile("pub fn ordinary() {}", version, true).success);
    }
}

#[test]
fn ordinary_types_only_functions_do_not_reserve_constructor_names() {
    for source in [
        "pub struct FooBar; pub fn foo_bar() {}",
        "pub fn foo_bar() {} pub struct FooBar;",
        "pub enum E { FooBar } pub fn foo_bar() {}",
    ] {
        for version in ["3", "4"] {
            let result = compile(source, version, true);
            assert!(result.success, "{source}: {:?}", result.diagnostics);
            assert!(result.diagnostics.iter().any(|d| {
                d.code.as_deref() == Some("RS_VALUES_UNSUPPORTED")
                    && d.severity == DiagnosticSeverity::Warning
            }));
            let ir = result.ir.unwrap();
            if version == "4" {
                assert_eq!(
                    ir["distribution"]["Library"]["def"]["modules"]["models"]["Public"]["values"],
                    json!({})
                );
            } else {
                assert_eq!(
                    ir["distribution"][3]["modules"][0][1]["value"]["values"],
                    json!([])
                );
            }
        }
    }
}

#[test]
fn normalized_binding_names_do_not_collide_with_values_or_constructors() {
    for prefix in [
        "pub struct FooBar;",
        "pub struct FooBar(pub i64);",
        "pub struct FooBar { pub x: i64 }",
        "pub enum E { FooBar }",
        r#"#[morphir::native(hint="arithmetic")] pub fn foo_bar() {}"#,
        r#"#[morphir::external(target="rust", name="other")] pub fn foo_bar() {}"#,
    ] {
        let binding = r#"#[morphir::native(hint="arithmetic")] pub fn fooBar() {}"#;
        for source in [format!("{prefix} {binding}"), format!("{binding} {prefix}")] {
            reject(&source, "4", false);
        }
    }
}

#[test]
fn ignored_binding_bodies_must_still_parse_and_attributes_are_function_only() {
    reject(
        r#"#[morphir::native(hint="arithmetic")] pub fn f() { let = ; }"#,
        "4",
        false,
    );
    for item in ["pub struct S;", "pub type S = i64;", "pub enum E { V }"] {
        reject(
            &format!(r#"#[morphir::native(hint="arithmetic")] {item}"#),
            "4",
            false,
        );
    }
}
