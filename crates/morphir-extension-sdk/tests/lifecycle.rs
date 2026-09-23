use morphir_extension_sdk::{
    __ProtocolSession, __dispatch_frontend, __dispatch_request, ExtensionType, NativeExtension,
    native::doc_fixtures::DocFrontend,
    protocol::{ExtensionRequest, ExtensionResponse, error_codes, methods},
};
use serde_json::{Value, json};

enum ProtocolTestDriver {
    Native(Box<NativeExtension>),
    Guest(__ProtocolSession),
}

impl ProtocolTestDriver {
    fn handle(&self, request: ExtensionRequest) -> ExtensionResponse {
        match self {
            Self::Native(extension) => extension.protocol().handle(request),
            Self::Guest(session) => session.dispatch(&request, || {
                __dispatch_request::<DocFrontend>(
                    &request,
                    &[__dispatch_frontend::<DocFrontend>],
                    &[ExtensionType::Frontend],
                )
            }),
        }
    }
}

fn fresh_protocol_drivers() -> [ProtocolTestDriver; 2] {
    [
        ProtocolTestDriver::Native(Box::new(
            NativeExtension::frontend_only(DocFrontend).unwrap(),
        )),
        ProtocolTestDriver::Guest(__ProtocolSession::new()),
    ]
}

fn request(method: &str, params: Value) -> ExtensionRequest {
    ExtensionRequest::new(method, params, 42).unwrap()
}

fn initialize(extension: &ProtocolTestDriver) {
    let response = extension.handle(request(
        methods::INITIALIZE,
        json!({"protocolVersions":["0.1"],"host":{"name":"test","version":"1"}}),
    ));
    assert!(response.error.is_none(), "{:?}", response.error);
}

fn assert_not_initialized(response: ExtensionResponse) {
    assert_eq!(response.id, 42);
    assert!(response.result.is_none());
    assert_eq!(response.error.unwrap().code, error_codes::NOT_INITIALIZED);
}

#[test]
fn protocol_refuses_work_before_initialize() {
    for extension in fresh_protocol_drivers() {
        for method in [
            methods::INFO,
            methods::CAPABILITIES,
            methods::COMPILE,
            methods::SHUTDOWN,
            "morphir.unknown",
        ] {
            assert_not_initialized(extension.handle(request(method, json!({}))));
        }
    }
}

#[test]
fn protocol_allows_describe_ping_and_exit_before_initialize() {
    for extension in fresh_protocol_drivers() {
        for (method, params) in [
            (methods::DESCRIBE, json!({"protocolVersions":["0.1"]})),
            (methods::PING, json!({})),
            (methods::EXIT, json!({})),
        ] {
            let response = extension.handle(request(method, params));
            assert!(response.error.is_none(), "{method}: {:?}", response.error);
            assert_not_initialized(extension.handle(request(methods::INFO, json!({}))));
        }
    }
}

#[test]
fn failed_initialize_does_not_open_a_session() {
    for extension in fresh_protocol_drivers() {
        for params in [
            json!({}),
            json!({"protocolVersions":["9.0"],"host":{"name":"test","version":"1"}}),
        ] {
            assert!(
                extension
                    .handle(request(methods::INITIALIZE, params))
                    .error
                    .is_some()
            );
            assert_not_initialized(extension.handle(request(methods::INFO, json!({}))));
        }
        initialize(&extension);
        assert!(
            extension
                .handle(request(methods::INFO, json!({})))
                .error
                .is_none()
        );
    }
}

#[test]
fn protocol_refuses_every_request_except_exit_after_shutdown() {
    for extension in fresh_protocol_drivers() {
        initialize(&extension);
        assert!(
            extension
                .handle(request(methods::SHUTDOWN, json!({})))
                .error
                .is_none()
        );
        for method in [
            methods::INITIALIZE,
            methods::DESCRIBE,
            methods::PING,
            methods::INFO,
            methods::CAPABILITIES,
            methods::COMPILE,
            methods::SHUTDOWN,
            "morphir.unknown",
        ] {
            assert_not_initialized(extension.handle(request(method, json!({}))));
        }
        assert!(
            extension
                .handle(request(methods::EXIT, json!({})))
                .error
                .is_none()
        );
    }
}

#[test]
fn protocol_sessions_are_isolated_per_provider() {
    for (first, second) in fresh_protocol_drivers()
        .into_iter()
        .zip(fresh_protocol_drivers())
    {
        initialize(&first);
        assert_not_initialized(second.handle(request(methods::INFO, json!({}))));
        assert!(
            first
                .handle(request(methods::INFO, json!({})))
                .error
                .is_none()
        );
    }
}

#[test]
fn protocol_refuses_a_second_initialize() {
    for extension in fresh_protocol_drivers() {
        initialize(&extension);
        let again = extension.handle(request(
            methods::INITIALIZE,
            json!({"protocolVersions":["0.1"],"host":{"name":"test","version":"1"}}),
        ));
        assert_eq!(again.error.unwrap().code, error_codes::INVALID_REQUEST);
        assert!(
            extension
                .handle(request(methods::INFO, json!({})))
                .error
                .is_none()
        );
    }
}

/// Each session a host opens over one provider has its own lifecycle: one
/// session's shutdown does not close the next, and two sessions can be open
/// at once.
#[test]
fn every_opened_protocol_has_its_own_lifecycle() {
    let extension = NativeExtension::frontend_only(DocFrontend).unwrap();
    let init = || {
        request(
            methods::INITIALIZE,
            json!({"protocolVersions":["0.1"],"host":{"name":"test","version":"1"}}),
        )
    };

    let first = extension.open_protocol();
    assert!(first.handle(init()).error.is_none());
    assert!(
        first
            .handle(request(methods::SHUTDOWN, json!({})))
            .error
            .is_none()
    );
    assert_not_initialized(first.handle(request(methods::INFO, json!({}))));

    let second = extension.open_protocol();
    assert_not_initialized(second.handle(request(methods::INFO, json!({}))));
    assert!(second.handle(init()).error.is_none());
    let third = extension.clone().open_protocol();
    assert!(third.handle(init()).error.is_none());
    assert!(
        second
            .handle(request(methods::INFO, json!({})))
            .error
            .is_none()
    );
    assert!(
        third
            .handle(request(methods::INFO, json!({})))
            .error
            .is_none()
    );
}
