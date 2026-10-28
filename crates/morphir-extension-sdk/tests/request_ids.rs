//! Canonical request identity across wire decoding and shared dispatch paths.
use morphir_extension_sdk::{
    __ProtocolSession, __dispatch_request, ExtensionType, NativeExtension,
    native::doc_fixtures::DocFrontend,
    protocol::{ExtensionRequest, methods},
};
use serde_json::{Value, json};

fn request(method: &str, id: Value) -> ExtensionRequest {
    serde_json::from_value(json!({
        "jsonrpc": "2.0", "method": method, "params": {}, "id": id,
    }))
    .expect("canonical ID should decode")
}

#[test]
fn wire_ids_are_primitive_and_preserve_type_and_value() {
    for id in [json!(0), json!(u64::MAX), json!("python-λ-1"), json!("007")] {
        let decoded = request(methods::PING, id.clone());
        assert_eq!(serde_json::to_value(decoded).unwrap()["id"], id);
    }
    for id in [
        Value::Null,
        json!(true),
        json!({}),
        json!([]),
        json!(-1),
        json!(1.5),
    ] {
        assert!(
            serde_json::from_value::<ExtensionRequest>(json!({
                "jsonrpc": "2.0", "method": methods::PING, "params": {}, "id": id,
            }))
            .is_err()
        );
    }
    assert!(
        serde_json::from_str::<ExtensionRequest>(
            r#"{"jsonrpc":"2.0","method":"morphir.ping","params":{},"id":18446744073709551616}"#
        )
        .is_err()
    );
}

#[test]
fn native_and_generated_guest_paths_preserve_success_and_error_ids() {
    let native = NativeExtension::frontend_only(DocFrontend).unwrap();
    let guest = __ProtocolSession::new();
    for method in [methods::PING, methods::INFO, "missing.method"] {
        for id in [json!(u64::MAX), json!("python-λ-1"), json!("007")] {
            let request = request(method, id.clone());
            let expected_error = method != methods::PING;
            let native_response = native.protocol().handle(request.clone());
            let guest_response = guest.dispatch(&request, || {
                __dispatch_request::<DocFrontend>(&request, &[], &[ExtensionType::Frontend])
            });
            for response in [native_response, guest_response] {
                assert_eq!(response.error.is_some(), expected_error);
                assert_eq!(serde_json::to_value(response).unwrap()["id"], id);
            }
        }
    }
}

#[test]
fn initialized_sessions_preserve_ids_on_success_shutdown_and_errors() {
    for id in [json!(u64::MAX), json!("python-λ-1"), json!("007")] {
        let native = NativeExtension::frontend_only(DocFrontend).unwrap();
        let guest = __ProtocolSession::new();
        for (method, error) in [
            (methods::INITIALIZE, false),
            (methods::INFO, false),
            (methods::INITIALIZE, true),
            ("missing.method", true),
            (methods::SHUTDOWN, false),
            (methods::INFO, true),
        ] {
            let mut request = request(method, id.clone());
            if method == methods::INITIALIZE {
                request.params = json!({
                    "protocolVersions": ["0.1"], "host": {"name":"test", "version":"1"}
                });
            }
            let native_response = native.protocol().handle(request.clone());
            let guest_response = guest.dispatch(&request, || {
                __dispatch_request::<DocFrontend>(&request, &[], &[ExtensionType::Frontend])
            });
            for response in [native_response, guest_response] {
                assert_eq!(response.error.is_some(), error);
                assert_eq!(serde_json::to_value(response).unwrap()["id"], id);
            }
        }
    }
}

#[test]
fn canonical_parameter_free_requests_use_empty_named_parameters() {
    let native = NativeExtension::frontend_only(DocFrontend).unwrap();
    let guest = __ProtocolSession::new();
    for id in [json!(u64::MAX), json!("python-λ-1")] {
        let request: ExtensionRequest = serde_json::from_value(json!({
            "jsonrpc": "2.0", "id": id, "method": methods::PING,
        }))
        .expect("canonical transcripts omit parameters for unit methods");
        assert_eq!(request.params, json!({}));
        let native_response = native.protocol().handle(request.clone());
        let guest_response = guest.dispatch(&request, || {
            __dispatch_request::<DocFrontend>(&request, &[], &[ExtensionType::Frontend])
        });
        for response in [native_response, guest_response] {
            assert!(response.error.is_none());
            assert_eq!(serde_json::to_value(response).unwrap()["id"], id);
        }
    }
}
