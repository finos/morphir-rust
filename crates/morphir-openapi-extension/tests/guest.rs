use std::collections::HashMap;

use morphir_extension_sdk::{
    Backend, DiagnosticSeverity, Extension, ExtensionType, GenerateRequest,
};
use morphir_openapi_extension::OpenApiExtension;
use serde_json::{Value, json};

fn generate(
    target: &str,
    ir: Value,
    options: HashMap<String, Value>,
) -> morphir_extension_sdk::GenerateResult {
    OpenApiExtension
        .generate(GenerateRequest {
            ir,
            target: target.into(),
            options,
        })
        .expect("backend-domain failures remain successful MEP calls")
}

#[test]
fn advertises_both_targets_and_both_ir_versions() {
    let info = OpenApiExtension::info();
    assert_eq!(info.id, "morphir-openapi");
    assert_eq!(info.name, "Morphir OpenAPI");
    assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(info.types, [ExtensionType::Backend]);
    assert_eq!(info.license.as_deref(), Some("Apache-2.0"));

    let backend = OpenApiExtension::capabilities()
        .backend
        .expect("the extension advertises a backend capability");
    assert_eq!(backend.targets, ["openapi", "json-schema"]);
    assert_eq!(backend.ir_versions, ["3", "4"]);
    assert!(backend.generate);
}

#[test]
fn rejects_a_target_it_does_not_advertise() {
    let result = generate("avro", json!({"formatVersion": 4}), HashMap::new());

    assert!(!result.success);
    assert!(result.artifacts.is_empty());
    let diagnostic = result
        .diagnostics
        .first()
        .expect("an unadvertised target reports a diagnostic");
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Error);
    assert_eq!(diagnostic.code.as_deref(), Some("JSC001"));
    assert!(
        diagnostic.message.contains("avro"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn reports_an_ir_error_rather_than_panicking() {
    let result = generate("json-schema", json!({}), HashMap::new());

    assert!(!result.success);
    assert!(result.artifacts.is_empty());
    let diagnostic = result
        .diagnostics
        .first()
        .expect("malformed IR reports a diagnostic");
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Error);
    // The normalization error keeps its own stable code so a caller can tell
    // bad IR apart from a bad backend option (JSC002), rather than both
    // collapsing onto the same code.
    assert_eq!(diagnostic.code.as_deref(), Some("missing_format_version"));
    assert_ne!(diagnostic.code.as_deref(), Some("JSC002"));
}
