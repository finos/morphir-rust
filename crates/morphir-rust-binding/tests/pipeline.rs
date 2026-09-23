#[path = "support/native.rs"]
mod native;

use morphir_extension_sdk::{
    prelude::*,
    protocol::{ExtensionRequest, methods},
};
use morphir_rust_binding::RustExtension;
use quote::ToTokens;

#[test]
fn binding_declarations_compile_through_native_mep() {
    let extension = native::an_initialized_extension();
    let request = CompileRequest {
        language_id: "rust".into(),
        sources: SourceSet {
            root: None,
            documents: vec![SourceDocument {
                uri: "models.rs".into(),
                language_id: "rust".into(),
                version: 1,
                text: r#"
                #[morphir::native(hint = "arithmetic")]
                pub fn add(a: i64, b: i64) -> i64 { panic!("must not execute") }
                #[morphir::external(target = "rust", name = "vendor::identity")]
                pub fn identity<T>(value: T) -> T { value }
            "#
                .into(),
            }],
        },
        package: CompilePackage {
            name: "acme/example".into(),
            exposed_modules: Some(vec!["Models".into()]),
        },
        options: CompileOptions {
            types_only: false,
            ir_version: "4".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let response = extension
        .protocol()
        .handle(ExtensionRequest::new(methods::COMPILE, request, 1).unwrap());
    assert!(response.error.is_none());
    let compiled: CompileResult = serde_json::from_value(response.result.unwrap()).unwrap();
    assert!(compiled.success, "{:?}", compiled.diagnostics);
    let ir = compiled.ir.unwrap();
    let _: morphir_core::ir::v4::IRFile = serde_json::from_value(ir.clone()).unwrap();
    let values = &ir["distribution"]["Library"]["def"]["modules"]["models"]["Public"]["values"];
    assert!(values["add"]["Public"]["NativeBody"].is_object(), "{ir:#}");
    assert!(
        values["identity"]["Public"]["ExternalBody"].is_object(),
        "{ir:#}"
    );
}

#[test]
fn types_compile_and_generate_through_native_mep_in_both_versions() {
    for version in ["3", "4"] {
        let extension = native::an_initialized_extension();
        let request = CompileRequest {
            language_id: "rust".into(),
            sources: SourceSet {
                root: None,
                documents: vec![SourceDocument { uri: "models.rs".into(), language_id: "rust".into(), version: 1, text: "pub struct Person { pub name: String, pub age: i64 }\npub enum Decision { Pending, Accepted(Person) }\npub type Pair = (String, i64);".into() }],
            },
            package: CompilePackage { name: "acme/example".into(), exposed_modules: Some(vec!["Models".into()]) },
            dependencies: vec![],
            options: CompileOptions { types_only: true, ir_version: version.into(), ..Default::default() },
            baseline: None,
        };
        let response = extension
            .protocol()
            .handle(ExtensionRequest::new(methods::COMPILE, request, 1).unwrap());
        let compiled: CompileResult = serde_json::from_value(response.result.unwrap()).unwrap();
        assert!(compiled.success, "{:?}", compiled.diagnostics);
        let ir = compiled.ir.unwrap();
        let response = extension.protocol().handle(
            ExtensionRequest::new(
                methods::GENERATE,
                GenerateRequest {
                    ir,
                    target: "rust".into(),
                    options: Default::default(),
                },
                2,
            )
            .unwrap(),
        );
        let generated: GenerateResult = serde_json::from_value(response.result.unwrap()).unwrap();
        assert!(generated.success, "{:?}", generated.diagnostics);
        assert_eq!(generated.artifacts[0].path, "lib.rs");
        assert!(generated.artifacts[0].content.contains("Person"));
    }
}

#[test]
fn generic_module_declarations_roundtrip_in_both_versions() {
    for version in ["3", "4"] {
        let mut request = CompileRequest {
            language_id: "rust".into(),
            sources: SourceSet {
                root: None,
                documents: vec![SourceDocument {
                    uri: "models.rs".into(), language_id: "rust".into(), version: 1,
                    text: "pub struct Record<T> { pub value: T } pub enum Choice<T> { Empty, Full(T) } pub type Pair<T> = (T, ());".into(),
                }],
            },
            package: CompilePackage { name: "acme/example".into(), exposed_modules: Some(vec!["Models".into()]) },
            dependencies: vec![], options: CompileOptions { types_only: true, ir_version: version.into(), ..Default::default() },
            baseline: None,
        };
        let compiled = RustExtension.compile(request.clone()).unwrap();
        assert!(compiled.success, "{:?}", compiled.diagnostics);
        let ir = compiled.ir.unwrap();
        let generated = RustExtension
            .generate(GenerateRequest {
                ir: ir.clone(),
                target: "rust".into(),
                options: Default::default(),
            })
            .unwrap();
        assert!(generated.success, "{:?}", generated.diagnostics);
        let file = syn::parse_file(&generated.artifacts[0].content).unwrap();
        let syn::Item::Mod(module) = &file.items[0] else {
            panic!("expected module wrapper")
        };
        request.sources.documents[0].text = module
            .content
            .as_ref()
            .unwrap()
            .1
            .iter()
            .map(|item| item.to_token_stream().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let recompiled = RustExtension.compile(request).unwrap();
        assert!(recompiled.success, "{:?}", recompiled.diagnostics);
        // Map ordering is not a semantic difference; v3 lists retain declaration order.
        let normalize = |value| {
            if version == "3" {
                let typed = serde_json::from_value(value).unwrap();
                serde_json::to_value(
                    morphir_core::migration::migrate_distribution(&typed, Default::default())
                        .unwrap()
                        .value,
                )
                .unwrap()
            } else {
                value
            }
        };
        assert_eq!(normalize(ir), normalize(recompiled.ir.unwrap()));
    }
}
