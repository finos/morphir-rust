use morphir_extension_sdk::{
    prelude::*,
    protocol::{ExtensionRequest, methods},
};
use morphir_rust_binding::RustExtension;

#[path = "support/functions.rs"]
mod functions;

#[test]
fn functions_and_lambdas_execute_through_both_ir_versions_and_native_mep() {
    functions::assert_executable(&format!("mod models {{ {} }}", functions::SOURCE));
    for version in ["3", "4"] {
        let extension = NativeExtension::frontend_backend(RustExtension).unwrap();
        let request = CompileRequest {
            language_id: "rust".into(),
            sources: SourceSet {
                root: None,
                documents: vec![SourceDocument {
                    uri: "models.rs".into(),
                    language_id: "rust".into(),
                    version: 1,
                    text: functions::SOURCE.into(),
                }],
            },
            package: CompilePackage {
                name: "acme/example".into(),
                exposed_modules: Some(vec!["Models".into()]),
            },
            options: CompileOptions {
                types_only: false,
                ir_version: version.into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let response = extension
            .protocol()
            .handle(ExtensionRequest::new(methods::COMPILE, request, 1).unwrap());
        assert!(response.error.is_none());
        let compiled: CompileResult = serde_json::from_value(response.result.unwrap()).unwrap();
        assert!(compiled.success, "{version}: {:?}", compiled.diagnostics);
        assert!(
            compiled.diagnostics.is_empty(),
            "{:?}",
            compiled.diagnostics
        );
        let ir = compiled.ir.unwrap();
        assert_eq!(ir["formatVersion"], version.parse::<u64>().unwrap());
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
        assert!(response.error.is_none());
        let generated: GenerateResult = serde_json::from_value(response.result.unwrap()).unwrap();
        assert!(generated.success, "{version}: {:?}", generated.diagnostics);
        assert!(
            generated.diagnostics.is_empty(),
            "{:?}",
            generated.diagnostics
        );
        assert_eq!(generated.artifacts.len(), 1);
        functions::assert_executable(&generated.artifacts[0].content);
    }
}
