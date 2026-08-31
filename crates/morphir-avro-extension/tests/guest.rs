mod support;

use std::collections::HashMap;

use morphir_avro_extension::AvroExtension;
use morphir_extension_sdk::{
    Backend, DiagnosticSeverity, Extension, ExtensionType, GenerateRequest,
};
use serde_json::{Value, json};
use support::mothers;

fn options(entries: impl IntoIterator<Item = (&'static str, Value)>) -> HashMap<String, Value> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn generate(ir: Value, options: HashMap<String, Value>) -> morphir_extension_sdk::GenerateResult {
    AvroExtension
        .generate(GenerateRequest {
            ir,
            target: "avro".into(),
            options,
        })
        .expect("backend-domain failures should remain successful MEP calls")
}

#[test]
fn extension_metadata_and_backend_capabilities_match_mep() {
    let info = AvroExtension::info();
    assert_eq!(info.id, "morphir-avro");
    assert_eq!(info.name, "Morphir Avro");
    assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(
        info.description.as_deref(),
        Some("Projects Morphir specifications into Apache Avro")
    );
    assert_eq!(info.types, [ExtensionType::Backend]);
    assert_eq!(info.author.as_deref(), Some("FINOS"));
    assert_eq!(info.license.as_deref(), Some("Apache-2.0"));
    assert_eq!(
        info.homepage.as_deref(),
        Some("https://github.com/finos/morphir-rust")
    );
    assert_eq!(info.min_sdk_version.as_deref(), Some("0.2.0"));

    let capabilities = AvroExtension::capabilities();
    let backend = capabilities
        .backend
        .expect("the extension should advertise typed backend capabilities");
    assert_eq!(backend.targets, ["avro"]);
    assert_eq!(backend.ir_versions, ["3", "4"]);
    assert!(backend.generate);
    assert_eq!(AvroExtension::target_languages(), ["avro"]);
}

#[test]
fn v3_and_v4_generate_json_and_idl_text_artifacts() {
    for (ir, version) in [
        (mothers::classic_customer_library(), "3"),
        (mothers::v4_customer_library(), "4"),
    ] {
        for (representation, extension) in [("json", ".avsc"), ("idl", ".avdl")] {
            let result = generate(
                ir.clone(),
                options([
                    ("representation", json!(representation)),
                    ("unsupported", json!("warn-and-skip")),
                ]),
            );
            assert!(
                result.success,
                "IR {version} {representation}: {:?}",
                result.diagnostics
            );
            assert!(!result.artifacts.is_empty());
            assert!(result.artifacts.iter().all(|artifact| {
                artifact.path.ends_with(extension)
                    && !artifact.binary
                    && !artifact.content.is_empty()
            }));
        }
    }
}

#[test]
fn invalid_options_are_reported_before_malformed_ir() {
    let result = generate(
        json!({ "distribution": "not Morphir IR" }),
        options([("decimal_precision", json!(0))]),
    );

    assert!(!result.success);
    assert!(result.artifacts.is_empty());
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].code.as_deref(), Some("AVRO004"));
    assert_eq!(result.diagnostics[0].severity, DiagnosticSeverity::Error);
}

#[test]
fn malformed_and_unsupported_ir_versions_are_domain_diagnostics() {
    for (ir, code) in [
        (json!({ "distribution": null }), "missing_format_version"),
        (
            json!({ "formatVersion": 4, "distribution": null }),
            "invalid_ir",
        ),
        (
            json!({ "formatVersion": 5, "distribution": null }),
            "unsupported_format_version_major",
        ),
    ] {
        let result = generate(ir, HashMap::new());
        assert!(!result.success);
        assert!(result.artifacts.is_empty());
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].code.as_deref(), Some(code));
        assert_eq!(result.diagnostics[0].severity, DiagnosticSeverity::Error);
    }
}

#[test]
fn strict_projection_errors_emit_no_artifacts() {
    let result = generate(
        mothers::v4_customer_application(),
        options([("projection", json!("protocol-entry-points"))]),
    );

    assert!(!result.success);
    assert!(result.artifacts.is_empty());
    assert!(!result.diagnostics.is_empty());
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    );
    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_deref() == Some("AVRO001")
            && diagnostic
                .location
                .as_ref()
                .map(|location| location.uri.as_str())
                == Some("morphir-fqname:acme/customer:domain#unfinished")
    }));
}

#[test]
fn warn_and_skip_returns_valid_artifacts_and_source_warnings() {
    let result = generate(
        mothers::v4_customer_application(),
        options([
            ("projection", json!("protocol-entry-points")),
            ("unsupported", json!("warn-and-skip")),
        ]),
    );

    assert!(result.success, "{:?}", result.diagnostics);
    assert!(!result.artifacts.is_empty());
    assert!(result.artifacts.iter().all(|artifact| !artifact.binary));
    assert!(!result.diagnostics.is_empty());
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
    );
    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_deref() == Some("AVRO001")
            && diagnostic
                .location
                .as_ref()
                .map(|location| location.uri.as_str())
                == Some("morphir-fqname:acme/customer:domain#unfinished")
    }));
}

#[test]
fn constants_are_zero_argument_messages_without_evaluated_values() {
    let result = generate(
        mothers::v4_customer_library(),
        options([
            ("projection", json!("protocol-public")),
            ("unsupported", json!("warn-and-skip")),
        ]),
    );

    assert!(result.success, "{:?}", result.diagnostics);
    let protocol = result
        .artifacts
        .iter()
        .find(|artifact| artifact.path.ends_with(".avpr"))
        .expect("public protocol projection should emit an Avro protocol");
    let protocol: Value = serde_json::from_str(&protocol.content).unwrap();
    let constant = &protocol["messages"]["defaultCustomer"];
    assert_eq!(constant["request"], json!([]));
    assert_eq!(constant["morphir.value-kind"], "constant");
    assert!(constant.get("morphir.constant-value").is_none());
    assert!(constant.get("body").is_none());
    assert!(constant.get("value").is_none());
}

#[test]
fn option_insertion_order_does_not_change_generation() {
    let first = options([
        ("representation", json!("idl")),
        ("projection", json!("protocol-public")),
        ("dependencies", json!("self-contained")),
    ]);
    let second = options([
        ("dependencies", json!("self-contained")),
        ("projection", json!("protocol-public")),
        ("representation", json!("idl")),
    ]);

    let first = generate(mothers::v4_customer_library(), first);
    let second = generate(mothers::v4_customer_library(), second);
    assert_eq!(
        first
            .artifacts
            .iter()
            .map(|artifact| (&artifact.path, &artifact.content, artifact.binary))
            .collect::<Vec<_>>(),
        second
            .artifacts
            .iter()
            .map(|artifact| (&artifact.path, &artifact.content, artifact.binary))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        first
            .diagnostics
            .iter()
            .map(|diagnostic| (
                diagnostic.severity,
                diagnostic.code.as_deref(),
                diagnostic.message.as_str(),
                diagnostic
                    .location
                    .as_ref()
                    .map(|location| location.uri.as_str()),
            ))
            .collect::<Vec<_>>(),
        second
            .diagnostics
            .iter()
            .map(|diagnostic| (
                diagnostic.severity,
                diagnostic.code.as_deref(),
                diagnostic.message.as_str(),
                diagnostic
                    .location
                    .as_ref()
                    .map(|location| location.uri.as_str()),
            ))
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_single_target_backend_ignores_the_requested_target() {
    let ir = mothers::classic_customer_library();

    let stated = AvroExtension
        .generate(GenerateRequest {
            ir: ir.clone(),
            target: "avro".into(),
            options: options([("unsupported", json!("warn-and-skip"))]),
        })
        .expect("generation succeeds");
    let unexpected = AvroExtension
        .generate(GenerateRequest {
            ir,
            target: "not-a-target".into(),
            options: options([("unsupported", json!("warn-and-skip"))]),
        })
        .expect("generation succeeds");

    assert!(stated.success);
    assert_eq!(stated.artifacts, unexpected.artifacts);
}
