use morphir_extension_sdk::model::*;
use morphir_extension_sdk::protocol::{ExtensionRequest, InitializeParams, PeerInfo, methods};
use morphir_extension_sdk::{
    Backend, BackendCapability, Extension, ExtensionCapabilities, ExtensionInfo, ExtensionType,
    GenerateRequest, GenerateResult, NativeExtension,
};
use serde_json::{Value, json};

#[derive(Default)]
struct ModelBackend {
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
fn support() -> ModelTransportSupport {
    let c: Value = serde_json::from_str(include_str!(
        "fixtures/model_transport/model-transport.cases.json"
    ))
    .unwrap();
    serde_json::from_value(c["cases"][0]["hostOffer"]["modelTransport"].clone()).unwrap()
}
impl Extension for ModelBackend {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "model-backend".into(),
            types: vec![ExtensionType::Backend],
            ..Default::default()
        }
    }
    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities {
            backend: Some(BackendCapability {
                targets: vec!["moonbit".into()],
                ir_versions: vec!["4.0.0".into()],
                generate: false,
            }),
            model_transport: Some(support()),
            ..Default::default()
        }
    }
}
impl Backend for ModelBackend {
    fn generate(&self, _: GenerateRequest) -> morphir_extension_sdk::Result<GenerateResult> {
        panic!("rich requests must never use a JSON legacy fallback")
    }
    fn target_languages() -> Vec<String> {
        vec!["moonbit".into()]
    }
    fn generate_model(
        &self,
        request: GenerateModelRequest,
    ) -> morphir_extension_sdk::Result<GenerateResult> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(GenerateResult {
            success: true,
            artifacts: vec![morphir_extension_sdk::Artifact {
                path: "retained.ionb".into(),
                content: request.model.payload.content,
                binary: true,
            }],
            diagnostics: vec![],
        })
    }
}

#[test]
fn native_guest_selects_exact_model_draft_and_freezes_bilateral_limits() {
    let guest = NativeExtension::backend_only(ModelBackend::default()).unwrap();
    let mut offer = support();
    offer.limits.max_decoded_bytes = 2048;
    let response = guest.protocol().handle(
        ExtensionRequest::new(
            methods::INITIALIZE,
            InitializeParams {
                protocol_versions: vec![MODEL_MEP_VERSION.into(), "0.1".into()],
                host: PeerInfo {
                    kind: Default::default(),
                    name: "test".into(),
                    version: "1".into(),
                },
                model_transport: Some(offer),
            },
            1,
        )
        .unwrap(),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    let initialized = response.result.unwrap();
    assert_eq!(initialized["protocolVersion"], MODEL_MEP_VERSION);
    assert_eq!(
        initialized["capabilities"]["modelTransport"]["limits"]["maxDecodedBytes"],
        2048
    );
    let capabilities = guest
        .protocol()
        .handle(ExtensionRequest::new(methods::CAPABILITIES, json!({}), 2).unwrap())
        .result
        .unwrap();
    assert_eq!(capabilities, initialized["capabilities"]);
}

#[test]
fn legacy_guest_session_never_grants_rich_transport() {
    let guest = NativeExtension::backend_only(ModelBackend::default()).unwrap();
    let response = guest.protocol().handle(
        ExtensionRequest::new(
            methods::INITIALIZE,
            json!({"protocolVersions":["0.1"],"host":{"name":"test","version":"1"}}),
            1,
        )
        .unwrap(),
    );
    assert_eq!(response.result.as_ref().unwrap()["protocolVersion"], "0.1");
    assert!(
        response.result.unwrap()["capabilities"]
            .get("modelTransport")
            .is_none()
    );
    let refused = guest
        .protocol()
        .handle(ExtensionRequest::new(methods::GENERATE_MODEL, json!({}), 2).unwrap());
    assert!(refused.error.is_some());
}

#[test]
fn native_guest_checks_before_calling_and_retains_opaque_payloads() {
    let backend = ModelBackend::default();
    let calls = backend.calls.clone();
    let guest = NativeExtension::backend_only(backend).unwrap();
    let initialized=guest.protocol().handle(ExtensionRequest::new(methods::INITIALIZE,json!({"protocolVersions":[MODEL_MEP_VERSION],"host":{"name":"test","version":"1"},"modelTransport":support()}),1).unwrap());
    assert!(initialized.error.is_none());
    let c: Value = serde_json::from_str(include_str!(
        "fixtures/model_transport/model-transport.cases.json"
    ))
    .unwrap();
    let params = c["cases"][0]["request"]["params"].clone();
    let mut malformed = params.clone();
    malformed["model"]["payload"]["sha256"] = json!("0".repeat(64));
    let refused = guest
        .protocol()
        .handle(ExtensionRequest::new(methods::GENERATE_MODEL, malformed, 2).unwrap());
    assert!(refused.error.is_some());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    let generated = guest
        .protocol()
        .handle(ExtensionRequest::new(methods::GENERATE_MODEL, &params, 3).unwrap());
    assert!(generated.error.is_none(), "{:?}", generated.error);
    assert_eq!(
        generated.result.unwrap()["artifacts"][0]["content"],
        params["model"]["payload"]["content"]
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let unsupported = guest.protocol().handle(
        ExtensionRequest::new(methods::GENERATE, json!({"ir":{},"target":"moonbit"}), 4).unwrap(),
    );
    assert!(unsupported.error.is_some());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn matching_evidence_bindings_do_not_establish_a_remote_guarantee() {
    let backend = ModelBackend::default();
    let calls = backend.calls.clone();
    let guest = NativeExtension::backend_only(backend).unwrap();
    assert!(guest.protocol().handle(ExtensionRequest::new(methods::INITIALIZE,json!({"protocolVersions":[MODEL_MEP_VERSION],"host":{"name":"test","version":"1"},"modelTransport":support()}),1).unwrap()).error.is_none());
    let c: Value = serde_json::from_str(include_str!(
        "fixtures/model_transport/model-transport.cases.json"
    ))
    .unwrap();
    let mut params = c["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "model-0021")
        .unwrap()["request"]["params"]
        .clone();
    params["model"]["evidence"][0]["modelRevision"] = params["model"]["revision"].clone();
    let refused = guest
        .protocol()
        .handle(ExtensionRequest::new(methods::GENERATE_MODEL, params, 2).unwrap());
    assert!(refused.error.is_some());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[test]
fn unselected_model_operation_is_capability_unavailable() {
    let guest = NativeExtension::backend_only(ModelBackend::default()).unwrap();
    let mut offer = support();
    offer
        .tuples
        .retain(|tuple| tuple.operation == ModelOperation::GenerateModel);
    let initialized = guest.protocol().handle(ExtensionRequest::new(methods::INITIALIZE, json!({"protocolVersions":[MODEL_MEP_VERSION],"host":{"name":"test","version":"1"},"modelTransport":offer}),1).unwrap());
    assert!(initialized.error.is_none());
    let refused = guest
        .protocol()
        .handle(ExtensionRequest::new(methods::COMPILE_MODEL, json!({}), 2).unwrap());
    assert_eq!(
        refused.error.unwrap().code,
        morphir_extension_sdk::protocol::error_codes::CAPABILITY_UNAVAILABLE
    );
}

#[test]
fn oversized_wire_payload_is_refused_before_typed_request_decoding() {
    let mut offer = support();
    offer.limits.max_encoded_bytes = 8;
    let transport = NegotiatedModelTransport::select(&offer, &offer).unwrap();
    // An unrelated missing target would fail typed deserialization. The bounded
    // raw-payload pass must run first, without cloning this content string.
    let params = json!({"model":{"payload":{"content":"x".repeat(100),"decodedByteLength":75}}});
    let error = validate_model_invocation(
        &transport,
        &ModelBackend::capabilities(),
        methods::GENERATE_MODEL,
        &params,
    )
    .unwrap_err();
    assert_eq!(error.code(), "encoded_limit");
}
