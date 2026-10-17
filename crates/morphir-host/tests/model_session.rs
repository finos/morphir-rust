//! Rich model admission runs before transport, including generic session calls.

use morphir_extension_sdk::model::*;
use morphir_extension_sdk::protocol::{
    ExtensionResponse, InitializeResult, PeerInfo, PeerKind, methods,
};
use morphir_extension_sdk::{
    BackendCapability, ExtensionCapabilities, ExtensionInfo, ExtensionType, FrontendCapability,
    LanguageCapability,
};
use morphir_host::testing::MemoryChannel;
use morphir_host::{
    BasicChecks, CallError, ExpectedChecks, ExpectedExtension, HostConfig, HostError,
    JsonRpcConnection, Negotiated, Session, SessionChecks,
};
use serde_json::{Value, json};

fn a_host() -> HostConfig {
    HostConfig::new(PeerInfo {
        kind: PeerKind::Unspecified,
        name: "fixture".into(),
        version: "1".into(),
    })
}

fn a_support() -> ModelTransportSupport {
    ModelTransportSupport {
        tuples: [ModelOperation::CompileModel, ModelOperation::GenerateModel]
            .into_iter()
            .map(|operation| ModelTransportTuple {
                operation,
                profile: "morphir-ir".into(),
                codec: ModelCodec::MorphirJson,
                envelope_version: "0.1.0-draft.1".into(),
                ir_version: "4.0.0".into(),
            })
            .collect(),
        limits: ModelTransportLimits {
            max_encoded_bytes: 8192,
            max_decoded_bytes: 4096,
            max_depth: 64,
            max_nodes: 256,
        },
        guarantees: vec![],
    }
}

fn a_model() -> MorphirModel {
    serde_json::from_value(json!({
        "envelopeVersion": "0.1.0-draft.1", "modelId": "urn:fixture:model", "revision": "1",
        "irVersion": "4.0.0", "profile": "morphir-ir",
        "payload": {"codec":"morphir-json", "layout":"single-document", "decodedByteLength":130,
            "sha256":"fd49ad8b0cab49216323beb969c08f91a888925dbb995838c57a05b48a80bb2f",
            "content":"eyJmb3JtYXRWZXJzaW9uIjoiNC4wLjAiLCJkaXN0cmlidXRpb24iOnsiTGlicmFyeSI6eyJwYWNrYWdlTmFtZSI6ImV4YW1wbGUvbWVwLWZpeHR1cmUiLCJkZXBlbmRlbmNpZXMiOnt9LCJkZWYiOnsibW9kdWxlcyI6e319fX19Cg=="}
    })).unwrap()
}

fn a_generate() -> GenerateModelRequest {
    GenerateModelRequest {
        model: a_model(),
        input: a_support().tuples[1].clone(),
        target: "moonbit".into(),
        context_digest: "1".repeat(64),
        required_guarantees: vec![],
        options: Default::default(),
    }
}

fn a_compile() -> CompileModelRequest {
    serde_json::from_value(json!({
        "languageId":"moonbit", "sources":{"documents":[{"uri":"file:///fixture/main.mbt","languageId":"moonbit","version":1,"text":"pub fn is_ready() -> Bool { true }"}]},
        "package":{"name":"example/mep-fixture"}, "options":{"typesOnly":true,"irVersion":"4.0.0"},
        "contextDigest":"1".repeat(64), "output": a_support().tuples[0]
    })).unwrap()
}

fn an_initialization(support: ModelTransportSupport) -> InitializeResult {
    InitializeResult {
        protocol_version: "0.2.0-draft.1".into(),
        extension: ExtensionInfo {
            id: "guest".into(),
            name: "Guest".into(),
            types: vec![ExtensionType::Frontend, ExtensionType::Backend],
            ..Default::default()
        },
        capabilities: ExtensionCapabilities {
            frontend: Some(FrontendCapability {
                languages: vec![LanguageCapability {
                    id: "moonbit".into(),
                    file_extensions: vec!["mbt".into()],
                }],
                ir_versions: vec!["4.0.0".into()],
                ..Default::default()
            }),
            backend: Some(BackendCapability {
                targets: vec!["moonbit".into()],
                ir_versions: vec!["4.0.0".into()],
                generate: false,
            }),
            model_transport: Some(support),
            ..Default::default()
        },
    }
}

fn ok(id: u64, value: impl serde::Serialize) -> ExtensionResponse {
    ExtensionResponse::success(id, value).unwrap()
}

#[test]
fn model_offers_are_explicit_without_changing_legacy_default() {
    assert_eq!(a_host().protocol_versions(), ["0.1"]);
    assert!(a_host().initialize_params().model_transport.is_none());
    let host = a_host().with_model_transport(a_support());
    assert_eq!(host.protocol_versions(), ["0.2.0-draft.1", "0.1"]);
    assert_eq!(host.initialize_params().model_transport, Some(a_support()));
}

#[tokio::test]
async fn rich_selection_requires_the_original_host_offer() {
    let host = HostConfig::with_versions(
        a_host().peer().clone(),
        vec!["0.2.0-draft.1".into(), "0.1".into()],
    );
    let channel = MemoryChannel::new().respond(ok(1, an_initialization(a_support())));
    let log = channel.log();
    let connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    assert!(Session::open(connection, &host).await.is_err());
    assert_eq!(log.methods(), [methods::INITIALIZE]);
    assert_eq!(log.aborts(), 1);
}

struct PermissiveChecks;

impl SessionChecks for PermissiveChecks {
    type Error = HostError;

    fn negotiate(
        &mut self,
        _: &[String],
        result: InitializeResult,
    ) -> Result<Negotiated, HostError> {
        Ok(Negotiated::new(
            result.protocol_version,
            result.extension,
            result.capabilities,
            false,
        ))
    }
}

#[tokio::test]
async fn custom_checks_cannot_skip_bilateral_selection_validation() {
    let mut selection = a_support();
    selection.limits.max_encoded_bytes += 1;
    let channel = MemoryChannel::new().respond(ok(1, an_initialization(selection)));
    let log = channel.log();
    let connection = JsonRpcConnection::new(channel, PermissiveChecks);
    assert!(
        Session::open(connection, &a_host().with_model_transport(a_support()))
            .await
            .is_err()
    );
    assert_eq!(log.methods(), [methods::INITIALIZE]);
    assert_eq!(log.aborts(), 1);
}

#[tokio::test]
async fn unsupported_tuples_and_raised_limits_fail_the_handshake() {
    let mut wrong_ir = a_support();
    wrong_ir.tuples[0].ir_version = "3.0.0".into();
    let mut crossed_codec = a_support();
    crossed_codec.tuples[0].codec = ModelCodec::IonBinary;
    for selected in [wrong_ir, crossed_codec] {
        let channel = MemoryChannel::new().respond(ok(1, an_initialization(selected)));
        let log = channel.log();
        let connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
        assert!(
            Session::open(connection, &a_host().with_model_transport(a_support()))
                .await
                .is_err()
        );
        assert_eq!(log.methods(), [methods::INITIALIZE]);
    }
    let mut selected = a_support();
    selected.limits.max_decoded_bytes += 1;
    let channel = MemoryChannel::new().respond(ok(1, an_initialization(selected)));
    let log = channel.log();
    let connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    assert!(
        Session::open(connection, &a_host().with_model_transport(a_support()))
            .await
            .is_err()
    );
    assert_eq!(log.methods(), [methods::INITIALIZE]);
}

#[tokio::test]
async fn rich_helpers_use_tuples_without_requiring_legacy_booleans() {
    let channel = MemoryChannel::new()
        .respond(ok(1, an_initialization(a_support())))
        .respond(ok(2, json!({"success":true,"model":a_model()})))
        .respond(ok(
            3,
            json!({"success":true,"artifacts":[],"diagnostics":[]}),
        ))
        .respond(ok(4, json!({})));
    let log = channel.log();
    let connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    let mut session = Session::open(connection, &a_host().with_model_transport(a_support()))
        .await
        .unwrap();
    let CompileModelResult::Compiled { model, .. } =
        session.compile_model(a_compile()).await.unwrap()
    else {
        panic!("successful compile must retain the complete model")
    };
    assert_eq!(*model, a_model());
    let mut generate = a_generate();
    generate.model = *model;
    assert!(session.generate_model(generate).await.unwrap().success);
    session.close().await.unwrap();
    assert_eq!(
        log.methods(),
        [
            methods::INITIALIZE,
            "morphir.frontend.compileModel",
            "morphir.backend.generateModel",
            methods::SHUTDOWN,
            methods::EXIT
        ]
    );
}

#[tokio::test]
async fn legacy_ir_aliases_admit_rich_tuples_and_keep_json_operations_available() {
    let mut initialized = an_initialization(a_support());
    let frontend = initialized.capabilities.frontend.as_mut().unwrap();
    frontend.ir_versions = vec!["4".into()];
    frontend.compile = true;
    let backend = initialized.capabilities.backend.as_mut().unwrap();
    backend.ir_versions = vec!["4".into()];
    backend.generate = true;
    let legacy_ir = json!({"formatVersion":4,"distribution":{"Library":{
        "packageName":"example/mep-fixture","dependencies":{},"def":{"modules":{}}
    }}});
    let channel = MemoryChannel::new()
        .respond(ok(1, initialized))
        .respond(ok(2, json!({"success":true,"model":a_model()})))
        .respond(ok(3, json!({"success":true})))
        .respond(ok(
            4,
            json!({"success":true,"irVersion":"4","ir":legacy_ir}),
        ))
        .respond(ok(5, json!({"success":true})))
        .respond(ok(6, json!({})));
    let log = channel.log();
    let connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    let mut session = Session::open(connection, &a_host().with_model_transport(a_support()))
        .await
        .unwrap();
    assert!(matches!(
        session.compile_model(a_compile()).await.unwrap(),
        CompileModelResult::Compiled { .. }
    ));
    assert!(session.generate_model(a_generate()).await.unwrap().success);

    let mut legacy_compile = serde_json::to_value(a_compile()).unwrap();
    legacy_compile["options"]["irVersion"] = json!("4");
    let compiled = session
        .compile(serde_json::from_value(legacy_compile).unwrap())
        .await
        .unwrap();
    assert_eq!(compiled.ir.as_ref().unwrap()["formatVersion"], 4);
    assert!(
        session
            .generate(morphir_extension_sdk::GenerateRequest {
                ir: compiled.ir.unwrap(),
                target: "moonbit".into(),
                options: Default::default()
            })
            .await
            .unwrap()
            .success
    );
    session.close().await.unwrap();
    assert_eq!(
        log.methods(),
        [
            methods::INITIALIZE,
            methods::COMPILE_MODEL,
            methods::GENERATE_MODEL,
            methods::COMPILE,
            methods::GENERATE,
            methods::SHUTDOWN,
            methods::EXIT
        ]
    );
}

#[tokio::test]
async fn generic_calls_cannot_bypass_integrity_admission_and_rejection_keeps_session_ready() {
    let channel = MemoryChannel::new()
        .respond(ok(1, an_initialization(a_support())))
        .respond(ok(2, json!({"ok":true})))
        .respond(ok(3, json!({})));
    let log = channel.log();
    let connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    let mut session = Session::open(connection, &a_host().with_model_transport(a_support()))
        .await
        .unwrap();
    let mut request = a_generate();
    request.model.payload.sha256 = "0".repeat(64);
    assert!(matches!(
        session
            .call::<_, Value>("morphir.backend.generateModel", request)
            .await,
        Err(CallError::Rejected(_))
    ));
    assert_eq!(log.methods(), [methods::INITIALIZE]);
    let _: Value = session.call(methods::PING, json!({})).await.unwrap();
    session.close().await.unwrap();
    assert_eq!(
        log.methods(),
        [
            methods::INITIALIZE,
            methods::PING,
            methods::SHUTDOWN,
            methods::EXIT
        ]
    );
}

#[tokio::test]
async fn malformed_rich_results_cannot_escape_through_generic_calls() {
    let channel = MemoryChannel::new()
        .respond(ok(1, an_initialization(a_support())))
        .respond(ok(2, json!({"success":true})))
        .respond(ok(3, json!({})));
    let log = channel.log();
    let connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    let mut session = Session::open(connection, &a_host().with_model_transport(a_support()))
        .await
        .unwrap();
    assert!(matches!(
        session
            .call::<_, Value>("morphir.frontend.compileModel", a_compile())
            .await,
        Err(CallError::Invalid(_))
    ));
    assert_eq!(log.aborts(), 0);
    assert_eq!(log.closes(), 1);
    assert_eq!(
        log.methods(),
        [
            methods::INITIALIZE,
            methods::COMPILE_MODEL,
            methods::SHUTDOWN,
            methods::EXIT
        ]
    );
}

#[tokio::test]
async fn a_pool_does_not_retry_invalid_rich_results() {
    use morphir_host::{GuestConnection, Pool};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let opens = Arc::new(AtomicUsize::new(0));
    let counted = opens.clone();
    let open = move || {
        counted.fetch_add(1, Ordering::SeqCst);
        let channel = MemoryChannel::new()
            .respond(ok(1, an_initialization(a_support())))
            .respond(ok(2, json!({"success":true})))
            .respond(ok(3, json!({})));
        let connection: Box<dyn GuestConnection> =
            Box::new(JsonRpcConnection::new(channel, BasicChecks::new("guest")));
        std::future::ready(Ok(connection))
    };
    let pool = Pool::<String>::new(a_host().with_model_transport(a_support()));
    let result: Result<Value, CallError> = pool
        .call(
            &"guest".into(),
            "fingerprint",
            open,
            methods::COMPILE_MODEL,
            &a_compile(),
        )
        .await;
    assert!(matches!(result, Err(CallError::Invalid(_))));
    assert_eq!(opens.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn exact_discovery_allows_only_model_transport_narrowing() {
    let declared = an_initialization(a_support());
    let mut selected = declared.clone();
    let model = selected.capabilities.model_transport.as_mut().unwrap();
    model.tuples.remove(0);
    model.limits.max_nodes -= 1;
    let checks = ExpectedChecks::new(ExpectedExtension::discovered_with_capabilities(
        declared.extension.clone(),
        declared.capabilities.clone(),
    ));
    let channel = MemoryChannel::new()
        .respond(ok(1, selected.clone()))
        .respond(ok(2, json!({})));
    let connection = JsonRpcConnection::new(channel, checks);
    Session::open(connection, &a_host().with_model_transport(a_support()))
        .await
        .unwrap()
        .close()
        .await
        .unwrap();
    selected.capabilities.streaming = true;
    let channel = MemoryChannel::new().respond(ok(1, selected));
    let checks = ExpectedChecks::new(ExpectedExtension::discovered_with_capabilities(
        declared.extension,
        declared.capabilities,
    ));
    assert!(
        Session::open(
            JsonRpcConnection::new(channel, checks),
            &a_host().with_model_transport(a_support())
        )
        .await
        .is_err()
    );
}

#[test]
fn model_capable_discovery_can_select_legacy_without_model_support() {
    let declared = an_initialization(a_support());
    let mut selected = declared.clone();
    selected.protocol_version = "0.1".into();
    selected.capabilities.model_transport = None;
    let expected = || {
        ExpectedExtension::discovered_with_capabilities(
            declared.extension.clone(),
            declared.capabilities.clone(),
        )
    };
    assert!(
        morphir_host::validate_negotiation(expected(), &["0.1".into()], selected.clone()).is_ok()
    );
    selected
        .capabilities
        .backend
        .as_mut()
        .unwrap()
        .targets
        .push("unclaimed".into());
    assert!(morphir_host::validate_negotiation(expected(), &["0.1".into()], selected).is_err());
}
