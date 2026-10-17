//! The SDK keeps the canonical rich-model wire shapes distinct from legacy IR.

use morphir_extension_sdk::model::*;
use serde_json::{Value, json};

fn a_model() -> Value {
    json!({
        "envelopeVersion": "0.1.0-draft.1",
        "modelId": "urn:fixture:model",
        "revision": "revision-1",
        "irVersion": "4.0.0",
        "profile": "morphir-model",
        "payload": {
            "codec": "ion-binary",
            "layout": "single-document",
            "decodedByteLength": 4,
            "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
            "content": "4AEA6g=="
        }
    })
}

fn a_tuple() -> Value {
    json!({
        "operation": "morphir.frontend.compileModel",
        "profile": "morphir-model",
        "codec": "ion-binary",
        "envelopeVersion": "0.1.0-draft.1",
        "irVersion": "4.0.0"
    })
}

#[test]
fn tuples_and_support_use_exact_method_and_format_spellings() {
    let tuple: ModelTransportTuple = serde_json::from_value(a_tuple()).unwrap();
    assert_eq!(tuple.operation, ModelOperation::CompileModel);
    assert_eq!(tuple.codec, ModelCodec::IonBinary);
    assert_eq!(serde_json::to_value(&tuple).unwrap(), a_tuple());
    assert_eq!(
        serde_json::to_value(ModelOperation::GenerateModel).unwrap(),
        "morphir.backend.generateModel"
    );
    assert_eq!(
        serde_json::to_value(ModelCodec::MorphirJson).unwrap(),
        "morphir-json"
    );
    let support: ModelTransportSupport = serde_json::from_value(json!({
        "tuples": [a_tuple()],
        "limits": {
            "maxEncodedBytes": 4096,
            "maxDecodedBytes": 2048,
            "maxDepth": 32,
            "maxNodes": 128
        }
    }))
    .unwrap();
    assert!(support.guarantees.is_empty());
    assert_eq!(support.limits.max_encoded_bytes, 4096);
    assert_eq!(support.limits.max_decoded_bytes, 2048);
    assert_eq!(support.limits.max_depth, 32);
    assert_eq!(support.limits.max_nodes, 128);
    assert_eq!(support.tuples, vec![tuple]);
    for invalid in ["compileModel", "morphir.frontend.compile", "unknown"] {
        assert!(serde_json::from_value::<ModelOperation>(json!(invalid)).is_err());
    }
    assert!(serde_json::from_value::<ModelCodec>(json!("IonBinary")).is_err());
    assert!(serde_json::from_value::<ModelLayout>(json!("document-tree")).is_err());
}

#[test]
fn model_defaults_keep_opaque_payload_and_scoped_evidence() {
    let mut value = a_model();
    value["futureOptional"] = json!({"ignored": true});
    let model: MorphirModel = serde_json::from_value(value).unwrap();
    assert_eq!(model.payload.layout, ModelLayout::SingleDocument);
    assert_eq!(model.payload.content, "4AEA6g==");
    assert!(model.evidence.is_empty());
    assert!(model.critical.is_empty());
    let written = serde_json::to_value(&model).unwrap();
    assert_eq!(written["payload"], a_model()["payload"]);
    assert!(written.get("critical").is_none());
    assert!(written.get("futureOptional").is_none());

    let evidence: ModelGuaranteeEvidence = serde_json::from_value(json!({
        "guarantee": "urn:fixture:typed",
        "modelId": "urn:fixture:model",
        "modelRevision": "revision-1",
        "contextDigest": "1111111111111111111111111111111111111111111111111111111111111111",
        "scope": "urn:fixture:scope",
        "producer": "urn:fixture:producer",
        "evidence": a_model()["payload"]
    }))
    .unwrap();
    assert_eq!(evidence.model_revision, model.revision);
    assert_eq!(evidence.model_id, model.model_id);
    assert_eq!(evidence.evidence.content, model.payload.content);
}

#[test]
fn compile_requests_reuse_existing_context_and_full_dependency_models() {
    let request: CompileModelRequest = serde_json::from_value(json!({
        "languageId": "moonbit",
        "sources": {"documents": []},
        "package": {"name": "example/model"},
        "dependencies": [{"packageName": "example/dependency", "model": a_model()}],
        "options": {"typesOnly": true, "irVersion": "4.0.0"},
        "contextDigest": "1111111111111111111111111111111111111111111111111111111111111111",
        "output": a_tuple()
    }))
    .unwrap();
    assert_eq!(request.language_id, "moonbit");
    assert!(request.sources.documents.is_empty());
    assert_eq!(request.dependencies[0].package_name, "example/dependency");
    assert_eq!(request.dependencies[0].model.payload.content, "4AEA6g==");
    let mut value = serde_json::to_value(request).unwrap();
    value.as_object_mut().unwrap().remove("dependencies");
    assert!(
        serde_json::from_value::<CompileModelRequest>(value)
            .unwrap()
            .dependencies
            .is_empty()
    );
}

#[test]
fn compile_success_and_failure_are_distinct_wire_shapes() {
    let success: CompileModelResult = serde_json::from_value(json!({
        "success": true,
        "model": a_model(),
        "futureOptional": 1
    }))
    .unwrap();
    let CompileModelResult::Compiled {
        model,
        diagnostics,
        modules,
    } = success
    else {
        panic!("success must construct the compiled variant")
    };
    assert!(diagnostics.is_empty());
    assert!(modules.is_empty());
    let compiled = CompileModelResult::Compiled {
        model,
        diagnostics,
        modules,
    };
    let written = serde_json::to_value(compiled).unwrap();
    assert_eq!(written["success"], true);
    assert_eq!(written["model"]["modelId"], "urn:fixture:model");
    assert_eq!(written["diagnostics"], json!([]));
    assert_eq!(written["modules"], json!([]));

    let failure: CompileModelResult = serde_json::from_value(json!({
        "success": false,
        "futureOptional": "ignored",
        "modules": {"notASuccessField": true}
    }))
    .unwrap();
    assert!(matches!(failure, CompileModelResult::Failed { .. }));
    assert_eq!(
        serde_json::to_value(failure).unwrap(),
        json!({"success": false, "diagnostics": []})
    );
}

#[test]
fn compile_outcomes_refuse_missing_null_or_inconsistent_model_members() {
    for invalid in [
        json!({"success": true}),
        json!({"success": true, "model": null}),
        json!({"success": false, "model": a_model()}),
        json!({"success": false, "model": null}),
        json!({"success": "true", "model": a_model()}),
        json!({"model": a_model()}),
    ] {
        assert!(
            serde_json::from_value::<CompileModelResult>(invalid.clone()).is_err(),
            "invalid result was accepted: {invalid}"
        );
    }
}

#[test]
fn generation_requests_require_an_explicit_input_tuple_and_default_optional_lists() {
    let mut input = a_tuple();
    input["operation"] = json!("morphir.backend.generateModel");
    let request: GenerateModelRequest = serde_json::from_value(json!({
        "model": a_model(),
        "input": input,
        "target": "moonbit",
        "contextDigest": "1111111111111111111111111111111111111111111111111111111111111111"
    }))
    .unwrap();
    assert_eq!(request.input.operation, ModelOperation::GenerateModel);
    assert!(request.required_guarantees.is_empty());
    assert!(request.options.is_empty());
    let requirement = ModelGuaranteeRequirement {
        guarantee: "urn:fixture:typed".into(),
        scope: "urn:fixture:scope".into(),
    };
    assert_eq!(
        serde_json::to_value(requirement).unwrap()["scope"],
        "urn:fixture:scope"
    );
}
