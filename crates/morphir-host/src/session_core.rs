//! Sans-IO MEP session state machine.

use crate::envelope::{EnvelopeError, validate_envelope};
use crate::{HostError, Negotiated};
use morphir_extension_sdk::ExtensionType;
use morphir_extension_sdk::protocol::{
    ExtensionRequest, ExtensionResponse, InitializeParams, InitializeResult, error_codes, methods,
};
use serde_json::Value;
use std::collections::HashSet;

/// Negotiation rules that a client adds to the MEP handshake.
pub trait SessionChecks {
    /// The error type the client reports. Core errors convert into it.
    type Error: From<HostError>;

    /// Validate the guest's answer to `morphir.initialize`.
    fn negotiate(
        &mut self,
        offered: &[String],
        result: InitializeResult,
    ) -> Result<Negotiated, Self::Error>;
}

/// The checks every host applies: an offered version and the expected identity.
#[derive(Debug, Clone)]
pub struct BasicChecks {
    expected_id: String,
}

impl BasicChecks {
    /// Expect the guest to identify itself as `expected_id`.
    pub fn new(expected_id: impl Into<String>) -> Self {
        Self {
            expected_id: expected_id.into(),
        }
    }
}

impl SessionChecks for BasicChecks {
    type Error = HostError;

    fn negotiate(
        &mut self,
        offered: &[String],
        result: InitializeResult,
    ) -> Result<Negotiated, HostError> {
        if !offered.contains(&result.protocol_version) {
            return Err(HostError::VersionNotOffered {
                selected: result.protocol_version,
                offered: offered.to_vec(),
            });
        }
        if result.extension.id != self.expected_id {
            return Err(HostError::Invalid(format!(
                "Extension identity changed during initialization: expected '{}', initialized '{}'",
                self.expected_id, result.extension.id
            )));
        }
        let unique: HashSet<_> = result.extension.types.iter().copied().collect();
        if unique.len() != result.extension.types.len() {
            return Err(HostError::Invalid(
                "Extension initialization repeated a capability kind".into(),
            ));
        }
        if unique.contains(&ExtensionType::Frontend) && result.capabilities.frontend.is_none() {
            return Err(HostError::Invalid(
                "Extension declared Frontend without frontend capabilities".into(),
            ));
        }
        if !unique.contains(&ExtensionType::Frontend) && result.capabilities.frontend.is_some() {
            return Err(HostError::Invalid(
                "Extension advertised frontend capabilities without declaring Frontend".into(),
            ));
        }
        if unique.contains(&ExtensionType::Backend) && result.capabilities.backend.is_none() {
            return Err(HostError::Invalid(
                "Extension declared Backend without backend capabilities".into(),
            ));
        }
        if !unique.contains(&ExtensionType::Backend) && result.capabilities.backend.is_some() {
            return Err(HostError::Invalid(
                "Extension advertised backend capabilities without declaring Backend".into(),
            ));
        }
        if unique.contains(&ExtensionType::Workspace) && result.capabilities.workspace.is_none() {
            return Err(HostError::Invalid(
                "Extension declared Workspace without workspace capabilities".into(),
            ));
        }
        if !unique.contains(&ExtensionType::Workspace) && result.capabilities.workspace.is_some() {
            return Err(HostError::Invalid(
                "Extension advertised workspace capabilities without declaring Workspace".into(),
            ));
        }
        Ok(Negotiated::new(
            result.protocol_version,
            result.extension,
            result.capabilities,
            false,
        ))
    }
}

/// Something that happened to the session.
#[derive(Debug)]
pub enum Event {
    /// Start the handshake with these parameters.
    Open(InitializeParams),
    /// Invoke one non-lifecycle method.
    Call {
        /// The MEP method name.
        method: String,
        /// The request parameters.
        params: Value,
    },
    /// The guest answered the request in flight.
    Received(ExtensionResponse),
    /// Begin MEP shutdown.
    Close,
    /// The transport failed, so the session is over.
    TransportFailed,
}

/// What the client must do next.
#[derive(Debug)]
pub enum Action<E> {
    /// Deliver this request, then pass the answer back as [`Event::Received`].
    Send(ExtensionRequest),
    /// Negotiation succeeded. [`SessionCore::negotiated`] now returns the result.
    Ready,
    /// The call succeeded with this result.
    Completed(Value),
    /// The call did not succeed, and the session is still ready.
    Rejected(E),
    /// The session broke. Abort the transport.
    Failed(E),
    /// The guest acknowledged shutdown. Terminate the transport.
    ShutDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Initialize,
    Call,
    Shutdown,
}

#[derive(Debug)]
struct Pending {
    id: u64,
    purpose: Purpose,
}

#[derive(Debug)]
enum State {
    Loaded,
    Initializing { offered: Vec<String> },
    Ready,
    ShuttingDown,
    Stopped,
    Failed,
}

/// The MEP session state, driven by events and free of I/O.
///
/// One request is in flight at a time. The first request has id 1.
#[derive(Debug)]
pub struct SessionCore<C: SessionChecks> {
    checks: C,
    state: State,
    negotiated: Option<Negotiated>,
    next_id: u64,
    pending: Option<Pending>,
}

impl<C: SessionChecks> SessionCore<C> {
    /// A session that has not started the handshake.
    pub fn new(checks: C) -> Self {
        Self {
            checks,
            state: State::Loaded,
            negotiated: None,
            next_id: 1,
            pending: None,
        }
    }

    /// The negotiation result, once the handshake succeeded.
    pub fn negotiated(&self) -> Option<&Negotiated> {
        self.negotiated.as_ref()
    }

    /// Whether the session accepts a call now.
    pub fn is_ready(&self) -> bool {
        matches!(self.state, State::Ready) && self.pending.is_none()
    }

    /// Advance the session by one event.
    pub fn handle(&mut self, event: Event) -> Action<C::Error> {
        match event {
            Event::Open(params) => self.open(params),
            Event::Call { method, params } => self.call(method, params),
            Event::Received(response) => self.received(response),
            Event::Close => self.close(),
            Event::TransportFailed => {
                self.pending = None;
                self.state = State::Failed;
                Action::Failed(
                    HostError::State {
                        action: "continue",
                        state: "failed",
                    }
                    .into(),
                )
            }
        }
    }

    fn open(&mut self, params: InitializeParams) -> Action<C::Error> {
        if !matches!(self.state, State::Loaded) {
            let error = self.wrong_state("open");
            self.state = State::Failed;
            return Action::Failed(error);
        }
        let offered = params.protocol_versions.clone();
        match self.request(methods::INITIALIZE, params, Purpose::Initialize) {
            Ok(request) => {
                self.state = State::Initializing { offered };
                Action::Send(request)
            }
            Err(error) => {
                self.state = State::Failed;
                Action::Failed(error)
            }
        }
    }

    fn call(&mut self, method: String, params: Value) -> Action<C::Error> {
        if !self.is_ready() {
            let error = self.wrong_state("call");
            if !matches!(self.state, State::Stopped) {
                self.state = State::Failed;
            }
            return Action::Failed(error);
        }
        if matches!(
            method.as_str(),
            methods::INITIALIZE | methods::SHUTDOWN | methods::EXIT
        ) {
            return Action::Rejected(
                HostError::Rejected(format!(
                    "Protocol lifecycle method '{method}' must use its dedicated session operation"
                ))
                .into(),
            );
        }
        let negotiated = self
            .negotiated
            .as_ref()
            .expect("a ready session is negotiated");
        if !negotiated.supports_method(&method) {
            return Action::Rejected(
                HostError::Rejected(format!(
                    "RPC error {}: Extension does not support capability '{method}'",
                    error_codes::CAPABILITY_UNAVAILABLE
                ))
                .into(),
            );
        }
        if !negotiated.supports_invocation(&method, &params) {
            return Action::Rejected(
                HostError::Rejected(format!(
                    "RPC error {}: Extension does not support capability '{method}' for the requested protocol",
                    error_codes::CAPABILITY_UNAVAILABLE
                ))
                .into(),
            );
        }
        match self.request(&method, params, Purpose::Call) {
            Ok(request) => Action::Send(request),
            Err(error) => Action::Rejected(error),
        }
    }

    fn close(&mut self) -> Action<C::Error> {
        if !self.is_ready() {
            let error = self.wrong_state("close");
            self.state = State::Failed;
            return Action::Failed(error);
        }
        match self.request(methods::SHUTDOWN, serde_json::json!({}), Purpose::Shutdown) {
            Ok(request) => {
                self.state = State::ShuttingDown;
                Action::Send(request)
            }
            Err(error) => {
                self.state = State::Failed;
                Action::Failed(error)
            }
        }
    }

    fn received(&mut self, response: ExtensionResponse) -> Action<C::Error> {
        let Some(pending) = self.pending.take() else {
            self.state = State::Failed;
            return Action::Failed(
                HostError::Invalid(format!(
                    "Extension sent response ID {} with no request in flight",
                    response.id
                ))
                .into(),
            );
        };
        let outcome = validate_envelope(response, pending.id);
        match pending.purpose {
            Purpose::Initialize => self.initialized(outcome),
            Purpose::Call => match outcome {
                Ok(value) => Action::Completed(value),
                Err(EnvelopeError::Rpc(error)) => Action::Rejected(error.into()),
                Err(EnvelopeError::Invalid(error)) => {
                    self.state = State::Failed;
                    Action::Failed(error.into())
                }
            },
            Purpose::Shutdown => match outcome {
                Ok(_) => {
                    self.state = State::Stopped;
                    Action::ShutDown
                }
                Err(EnvelopeError::Rpc(error) | EnvelopeError::Invalid(error)) => {
                    self.state = State::Failed;
                    Action::Failed(error.into())
                }
            },
        }
    }

    fn initialized(&mut self, outcome: Result<Value, EnvelopeError>) -> Action<C::Error> {
        let State::Initializing { offered } = std::mem::replace(&mut self.state, State::Failed)
        else {
            return Action::Failed(
                HostError::State {
                    action: "initialize",
                    state: "not initializing",
                }
                .into(),
            );
        };
        let value = match outcome {
            Ok(value) => value,
            Err(EnvelopeError::Rpc(error) | EnvelopeError::Invalid(error)) => {
                return Action::Failed(error.into());
            }
        };
        let result: InitializeResult = match serde_json::from_value(value) {
            Ok(result) => result,
            Err(error) => return Action::Failed(HostError::Json(error).into()),
        };
        match self.checks.negotiate(&offered, result) {
            Ok(negotiated) => {
                self.negotiated = Some(negotiated);
                self.state = State::Ready;
                Action::Ready
            }
            Err(error) => Action::Failed(error),
        }
    }

    fn request(
        &mut self,
        method: &str,
        params: impl serde::Serialize,
        purpose: Purpose,
    ) -> Result<ExtensionRequest, C::Error> {
        let id = self.next_id;
        let next = id.checked_add(1).ok_or_else(|| {
            C::Error::from(HostError::Rejected(
                "Extension request identifier overflowed".into(),
            ))
        })?;
        let request = ExtensionRequest::new(method, params, id)
            .map_err(|error| C::Error::from(HostError::Json(error)))?;
        self.next_id = next;
        self.pending = Some(Pending { id, purpose });
        Ok(request)
    }

    fn wrong_state(&self, action: &'static str) -> C::Error {
        let state = match self.state {
            State::Ready if self.pending.is_some() => "waiting for a response",
            State::Loaded => "loaded",
            State::Initializing { .. } => "initializing",
            State::Ready => "ready",
            State::ShuttingDown => "shutting down",
            State::Stopped => "stopped",
            State::Failed => "failed",
        };
        HostError::State { action, state }.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_extension_sdk::protocol::{
        ExtensionResponse, PeerInfo, PeerKind, RpcError, methods,
    };
    use morphir_extension_sdk::{
        ExtensionCapabilities, ExtensionInfo, ExtensionType, FrontendCapability,
    };
    use serde_json::json;

    fn params() -> InitializeParams {
        InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                kind: PeerKind::Unspecified,
                name: "test".into(),
                version: "1.0.0".into(),
            },
        }
    }

    fn frontend_result(protocol_version: &str) -> InitializeResult {
        InitializeResult {
            protocol_version: protocol_version.into(),
            extension: ExtensionInfo {
                id: "guest".into(),
                name: "Guest".into(),
                types: vec![ExtensionType::Frontend],
                ..ExtensionInfo::default()
            },
            capabilities: ExtensionCapabilities {
                frontend: Some(FrontendCapability {
                    compile: true,
                    ..FrontendCapability::default()
                }),
                ..ExtensionCapabilities::default()
            },
        }
    }

    fn ok(id: u64, value: impl serde::Serialize) -> Event {
        Event::Received(ExtensionResponse::success(id, value).unwrap())
    }

    fn rpc_error(id: u64, code: i32, message: &str) -> Event {
        Event::Received(ExtensionResponse::error(
            id,
            RpcError {
                code,
                message: message.into(),
                data: None,
            },
        ))
    }

    fn sent(action: Action<HostError>) -> ExtensionRequest {
        match action {
            Action::Send(request) => request,
            other => panic!("expected a request to send, got {other:?}"),
        }
    }

    fn rejected(action: Action<HostError>) -> String {
        match action {
            Action::Rejected(error) => error.to_string(),
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    fn failed(action: Action<HostError>) -> String {
        match action {
            Action::Failed(error) => error.to_string(),
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    fn ready_core() -> SessionCore<BasicChecks> {
        let mut core = SessionCore::new(BasicChecks::new("guest"));
        sent(core.handle(Event::Open(params())));
        assert!(matches!(
            core.handle(ok(1, frontend_result("0.1"))),
            Action::Ready
        ));
        core
    }

    fn compile() -> Event {
        Event::Call {
            method: methods::COMPILE.into(),
            params: json!({}),
        }
    }

    #[test]
    fn open_sends_initialize_with_the_offered_versions() {
        let mut core = SessionCore::new(BasicChecks::new("guest"));
        let request = sent(core.handle(Event::Open(params())));
        assert_eq!(request.method, methods::INITIALIZE);
        assert_eq!(request.id, 1);
        assert_eq!(request.params["protocolVersions"], json!(["0.1"]));
    }

    #[test]
    fn a_valid_answer_makes_the_session_ready() {
        let core = ready_core();
        assert!(core.is_ready());
        assert_eq!(core.negotiated().unwrap().extension().id, "guest");
    }

    #[test]
    fn a_version_the_host_did_not_offer_fails_the_session() {
        let mut core = SessionCore::new(BasicChecks::new("guest"));
        sent(core.handle(Event::Open(params())));
        assert_eq!(
            failed(core.handle(ok(1, frontend_result("9.9")))),
            "Extension selected protocol version '9.9' that the host did not offer"
        );
        assert!(!core.is_ready());
    }

    #[test]
    fn a_changed_identity_fails_the_session() {
        let mut core = SessionCore::new(BasicChecks::new("someone-else"));
        sent(core.handle(Event::Open(params())));
        assert_eq!(
            failed(core.handle(ok(1, frontend_result("0.1")))),
            "Extension identity changed during initialization: expected 'someone-else', initialized 'guest'"
        );
    }

    #[test]
    fn a_declared_frontend_without_capabilities_fails_the_session() {
        let mut result = frontend_result("0.1");
        result.capabilities.frontend = None;
        let mut core = SessionCore::new(BasicChecks::new("guest"));
        sent(core.handle(Event::Open(params())));
        assert_eq!(
            failed(core.handle(ok(1, result))),
            "Extension declared Frontend without frontend capabilities"
        );
    }

    #[test]
    fn frontend_capabilities_without_the_kind_fail_the_session() {
        let mut result = frontend_result("0.1");
        result.extension.types = Vec::new();
        let mut core = SessionCore::new(BasicChecks::new("guest"));
        sent(core.handle(Event::Open(params())));
        assert_eq!(
            failed(core.handle(ok(1, result))),
            "Extension advertised frontend capabilities without declaring Frontend"
        );
    }

    #[test]
    fn a_repeated_capability_kind_fails_the_session() {
        let mut result = frontend_result("0.1");
        result.extension.types = vec![ExtensionType::Frontend, ExtensionType::Frontend];
        let mut core = SessionCore::new(BasicChecks::new("guest"));
        sent(core.handle(Event::Open(params())));
        assert_eq!(
            failed(core.handle(ok(1, result))),
            "Extension initialization repeated a capability kind"
        );
    }

    #[test]
    fn an_rpc_error_during_initialize_fails_the_session() {
        let mut core = SessionCore::new(BasicChecks::new("guest"));
        sent(core.handle(Event::Open(params())));
        assert_eq!(
            failed(core.handle(rpc_error(1, -32603, "boom"))),
            "RPC error -32603: boom"
        );
    }

    #[test]
    fn calls_use_the_next_request_id_and_complete() {
        let mut core = ready_core();
        let request = sent(core.handle(compile()));
        assert_eq!(request.method, methods::COMPILE);
        assert_eq!(request.id, 2);
        match core.handle(ok(2, json!({"answer": 42}))) {
            Action::Completed(value) => assert_eq!(value, json!({"answer": 42})),
            other => panic!("expected completion, got {other:?}"),
        }
        assert!(core.is_ready());
    }

    #[test]
    fn a_guest_rpc_error_rejects_the_call_and_keeps_the_session() {
        let mut core = ready_core();
        sent(core.handle(compile()));
        assert_eq!(
            rejected(core.handle(rpc_error(2, -32001, "does not compile"))),
            "RPC error -32001: does not compile"
        );
        assert!(core.is_ready());
        assert_eq!(sent(core.handle(compile())).id, 3);
    }

    #[test]
    fn an_invalid_envelope_fails_the_session() {
        let mut core = ready_core();
        sent(core.handle(compile()));
        assert_eq!(
            failed(core.handle(ok(99, json!({})))),
            "Extension response ID 99 did not match request ID 2"
        );
        assert_eq!(
            failed(core.handle(compile())),
            "Session cannot call while failed"
        );
    }

    #[test]
    fn lifecycle_methods_cannot_be_called_directly() {
        let mut core = ready_core();
        let action = core.handle(Event::Call {
            method: methods::SHUTDOWN.into(),
            params: json!({}),
        });
        assert_eq!(
            rejected(action),
            "Protocol lifecycle method 'morphir.shutdown' must use its dedicated session operation"
        );
        assert!(core.is_ready());
    }

    #[test]
    fn an_unadvertised_capability_is_rejected_without_sending() {
        let mut core = ready_core();
        let action = core.handle(Event::Call {
            method: methods::GENERATE.into(),
            params: json!({}),
        });
        assert_eq!(
            rejected(action),
            "RPC error -32013: Extension does not support capability 'morphir.backend.generate'"
        );
        assert_eq!(sent(core.handle(compile())).id, 2);
    }

    #[test]
    fn a_second_call_waits_for_the_first_answer() {
        let mut core = ready_core();
        sent(core.handle(compile()));
        assert_eq!(
            failed(core.handle(compile())),
            "Session cannot call while waiting for a response"
        );
        assert!(!core.is_ready());
    }

    #[test]
    fn a_call_after_shutdown_fails() {
        let mut core = ready_core();
        sent(core.handle(Event::Close));
        assert!(matches!(core.handle(ok(2, json!({}))), Action::ShutDown));
        assert_eq!(
            failed(core.handle(compile())),
            "Session cannot call while stopped"
        );
    }

    #[test]
    fn close_sends_shutdown_and_reports_shut_down() {
        let mut core = ready_core();
        let request = sent(core.handle(Event::Close));
        assert_eq!(request.method, methods::SHUTDOWN);
        assert_eq!(request.id, 2);
        assert!(matches!(core.handle(ok(2, json!({}))), Action::ShutDown));
        assert!(!core.is_ready());
    }

    #[test]
    fn a_response_with_no_request_in_flight_fails_the_session() {
        let mut core = ready_core();
        assert_eq!(
            failed(core.handle(ok(5, json!({})))),
            "Extension sent response ID 5 with no request in flight"
        );
    }

    #[test]
    fn a_transport_failure_ends_the_session() {
        let mut core = ready_core();
        sent(core.handle(compile()));
        failed(core.handle(Event::TransportFailed));
        assert!(!core.is_ready());
    }
}
