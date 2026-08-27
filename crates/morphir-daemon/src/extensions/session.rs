//! Validated Morphir Extension Protocol sessions.
//!
//! Wire messages are untrusted data. A [`Session`] validates JSON-RPC envelopes,
//! negotiation, capabilities, and lifecycle transitions once for every transport.

use crate::extensions::ExtensionContainer;
use crate::extensions::protocol::{
    ExtensionRequest, ExtensionResponse, InitializeParams, InitializeResult, JSONRPC_VERSION,
    RpcError, error_codes, methods,
};
use crate::{DaemonError, Result};
use async_trait::async_trait;
use morphir_extension_sdk::{ExtensionInfo, ExtensionType};
use serde::{Serialize, de::DeserializeOwned};
use std::{collections::HashSet, marker::PhantomData};

/// The transport's knowledge after an exchange or termination attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportState {
    /// The transport proved that the peer cannot accept more requests.
    Stopped,
    /// The transport cannot prove whether the peer accepted the last request.
    Indeterminate,
}

/// A transport failure together with the resulting lifecycle knowledge.
#[derive(Debug)]
pub struct TransportError {
    error: DaemonError,
    state: TransportState,
}

impl TransportError {
    /// Record a transport failure and what is known about the peer afterwards.
    pub fn new(error: DaemonError, state: TransportState) -> Self {
        Self { error, state }
    }
}

/// Identity known before protocol negotiation.
#[derive(Debug, Clone)]
pub struct ExpectedExtension {
    id: String,
    discovered: Option<ExtensionInfo>,
}

impl ExpectedExtension {
    /// Expect only a stable extension identifier.
    pub fn identified(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            discovered: None,
        }
    }
    /// Require initialization metadata to agree with discovery metadata.
    pub fn discovered(info: ExtensionInfo) -> Self {
        Self {
            id: info.id.clone(),
            discovered: Some(info),
        }
    }
}

/// Object-safe I/O boundary used by the MEP session controller.
#[async_trait]
pub trait MepTransport: Send {
    /// Return the identity or discovery record used to load the extension.
    fn expected_extension(&self) -> ExpectedExtension;
    /// Exchange one untrusted JSON-RPC request and response.
    async fn exchange(
        &mut self,
        request: ExtensionRequest,
    ) -> std::result::Result<ExtensionResponse, TransportError>;
    /// Stop the transport without sending another MEP lifecycle request.
    async fn terminate(&mut self) -> std::result::Result<TransportState, TransportError>;
}

#[async_trait]
impl<T: MepTransport + ?Sized> MepTransport for Box<T> {
    fn expected_extension(&self) -> ExpectedExtension {
        (**self).expected_extension()
    }
    async fn exchange(
        &mut self,
        request: ExtensionRequest,
    ) -> std::result::Result<ExtensionResponse, TransportError> {
        (**self).exchange(request).await
    }
    async fn terminate(&mut self) -> std::result::Result<TransportState, TransportError> {
        (**self).terminate().await
    }
}

/// A loaded extension that has not negotiated MEP.
pub struct Loaded;
/// A validated MEP session that accepts operations.
pub struct Ready;
/// A session whose peer has proved that it stopped.
pub struct Stopped;
/// A session whose peer state cannot be proved after a transport failure.
pub struct Indeterminate;

/// Validated application data produced by MEP negotiation.
#[derive(Debug, Clone)]
pub struct NegotiatedSession {
    protocol_version: String,
    extension: ExtensionInfo,
    capabilities: morphir_extension_sdk::ExtensionCapabilities,
}

impl NegotiatedSession {
    /// Selected MEP version.
    pub fn protocol_version(&self) -> &str {
        &self.protocol_version
    }
    /// Validated extension identity and capability kinds.
    pub fn extension(&self) -> &ExtensionInfo {
        &self.extension
    }
    /// Features negotiated for this session.
    pub fn capabilities(&self) -> &morphir_extension_sdk::ExtensionCapabilities {
        &self.capabilities
    }
    fn supports(&self, capability: ExtensionType) -> bool {
        self.extension.types.contains(&capability)
    }
}

/// A MEP session whose legal operations depend on its state parameter.
pub struct Session<T, S> {
    transport: T,
    next_request_id: u64,
    negotiated: Option<NegotiatedSession>,
    marker: PhantomData<S>,
}

impl<T> Session<T, Loaded> {
    /// Wrap a loaded transport before negotiation.
    pub fn loaded(transport: T) -> Self {
        Self {
            transport,
            next_request_id: 1,
            negotiated: None,
            marker: PhantomData,
        }
    }
}

impl<T: MepTransport> Session<T, Loaded> {
    /// Negotiate MEP and return a session that can invoke operations.
    pub async fn initialize(
        mut self,
        params: InitializeParams,
    ) -> std::result::Result<Session<T, Ready>, FailedSession<T>> {
        let offered = params.protocol_versions.clone();
        let expected = self.transport.expected_extension();
        let result: InitializeResult = match self.call(methods::INITIALIZE, params).await {
            CallOutcome::Success(value) => value,
            CallOutcome::RpcError(error) | CallOutcome::Invalid(error) => {
                return Err(self.fail_after_termination(error).await);
            }
            CallOutcome::Transport(error) => return Err(self.failed(error)),
        };
        let negotiated = match validate_negotiation(expected, &offered, result) {
            Ok(value) => value,
            Err(error) => return Err(self.fail_after_termination(error).await),
        };
        Ok(self.transition(Some(negotiated)))
    }
}

/// Result of invoking an operation on a ready session.
pub enum InvokeOutcome<T, R> {
    /// The operation succeeded and the session remains ready.
    Success(Session<T, Ready>, R),
    /// The extension rejected the operation and the session remains ready.
    Rejected(Session<T, Ready>, DaemonError),
    /// A protocol or transport failure changed the session state.
    Failed(FailedSession<T>),
}

impl<T: MepTransport> Session<T, Ready> {
    /// Return the validated negotiation data.
    pub fn negotiated(&self) -> &NegotiatedSession {
        self.negotiated
            .as_ref()
            .expect("ready sessions are negotiated")
    }

    /// Invoke one non-lifecycle operation.
    ///
    /// ```compile_fail
    /// use morphir_daemon::extensions::{Loaded, MepTransport, Session};
    /// async fn invalid<T: MepTransport>(session: Session<T, Loaded>) {
    ///     let _ = session.invoke::<serde_json::Value>("morphir.backend.generate", serde_json::json!({})).await;
    /// }
    /// ```
    pub async fn invoke<R: DeserializeOwned>(
        mut self,
        method: &str,
        params: impl Serialize,
    ) -> InvokeOutcome<T, R> {
        if matches!(method, methods::INITIALIZE | methods::SHUTDOWN) {
            return InvokeOutcome::Rejected(
                self,
                DaemonError::Extension(format!(
                    "Protocol lifecycle method '{method}' must use its dedicated session operation"
                )),
            );
        }
        if let Some(required) = required_capability(method)
            && !self.negotiated().supports(required)
        {
            return InvokeOutcome::Rejected(
                self,
                DaemonError::Extension(format!(
                    "RPC error {}: Extension does not support capability '{method}'",
                    error_codes::CAPABILITY_UNAVAILABLE
                )),
            );
        }
        match self.call(method, params).await {
            CallOutcome::Success(value) => InvokeOutcome::Success(self, value),
            CallOutcome::RpcError(error) => InvokeOutcome::Rejected(self, error),
            CallOutcome::Invalid(error) => {
                InvokeOutcome::Failed(self.fail_after_termination(error).await)
            }
            CallOutcome::Transport(error) => InvokeOutcome::Failed(self.failed(error)),
        }
    }

    /// Complete MEP shutdown and prove the resulting lifecycle state.
    pub async fn shutdown(mut self) -> std::result::Result<Session<T, Stopped>, FailedSession<T>> {
        match self
            .call::<_, serde_json::Value>(methods::SHUTDOWN, serde_json::json!({}))
            .await
        {
            CallOutcome::Success(_) => match self.transport.terminate().await {
                Ok(TransportState::Stopped) => Ok(self.transition(None)),
                Ok(TransportState::Indeterminate) => Err(FailedSession::Indeterminate(
                    self.transition(None),
                    DaemonError::Extension("Extension shutdown outcome is indeterminate".into()),
                )),
                Err(error) => Err(self.failed(error)),
            },
            CallOutcome::RpcError(error) | CallOutcome::Invalid(error) => {
                Err(self.fail_after_termination(error).await)
            }
            CallOutcome::Transport(error) => Err(self.failed(error)),
        }
    }
}

impl<T, S> Session<T, S> {
    /// Borrow the underlying transport for transport-specific observations.
    pub fn transport(&self) -> &T {
        &self.transport
    }
    /// Mutably borrow the underlying transport for transport-specific observations.
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }
    fn transition<N>(self, negotiated: Option<NegotiatedSession>) -> Session<T, N> {
        Session {
            transport: self.transport,
            next_request_id: self.next_request_id,
            negotiated,
            marker: PhantomData,
        }
    }
}

impl<T: MepTransport, S> Session<T, S> {
    async fn call<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: P,
    ) -> CallOutcome<R> {
        let id = self.next_request_id;
        self.next_request_id = match id.checked_add(1) {
            Some(next) => next,
            None => {
                return CallOutcome::Invalid(DaemonError::Extension(
                    "Extension request identifier overflowed".into(),
                ));
            }
        };
        let request = match ExtensionRequest::new(method, params, id) {
            Ok(request) => request,
            Err(error) => return CallOutcome::Invalid(error.into()),
        };
        let response = match self.transport.exchange(request).await {
            Ok(response) => response,
            Err(error) => return CallOutcome::Transport(error),
        };
        match validate_response(response, id) {
            Ok(value) => match serde_json::from_value(value) {
                Ok(value) => CallOutcome::Success(value),
                Err(error) => CallOutcome::Invalid(error.into()),
            },
            Err(ResponseFailure::Rpc(error)) => CallOutcome::RpcError(error),
            Err(ResponseFailure::Invalid(error)) => CallOutcome::Invalid(error),
        }
    }

    async fn fail_after_termination(mut self, error: DaemonError) -> FailedSession<T> {
        match self.transport.terminate().await {
            Ok(TransportState::Stopped) => FailedSession::Stopped(self.transition(None), error),
            Ok(TransportState::Indeterminate) => {
                FailedSession::Indeterminate(self.transition(None), error)
            }
            Err(termination) => {
                let state = termination.state;
                self.failed(TransportError::new(
                    DaemonError::Extension(format!(
                        "{error}; transport termination also failed: {}",
                        termination.error
                    )),
                    state,
                ))
            }
        }
    }

    fn failed(self, failure: TransportError) -> FailedSession<T> {
        match failure.state {
            TransportState::Stopped => FailedSession::Stopped(self.transition(None), failure.error),
            TransportState::Indeterminate => {
                FailedSession::Indeterminate(self.transition(None), failure.error)
            }
        }
    }
}

/// A failed transition paired with the only state the host can prove.
pub enum FailedSession<T> {
    /// The transport proved that the peer stopped.
    Stopped(Session<T, Stopped>, DaemonError),
    /// The transport could not prove the peer's state.
    Indeterminate(Session<T, Indeterminate>, DaemonError),
}

impl<T> FailedSession<T> {
    /// Return the failure that caused the state transition.
    pub fn error(&self) -> &DaemonError {
        match self {
            Self::Stopped(_, error) | Self::Indeterminate(_, error) => error,
        }
    }
}

enum CallOutcome<R> {
    Success(R),
    RpcError(DaemonError),
    Invalid(DaemonError),
    Transport(TransportError),
}
enum ResponseFailure {
    Rpc(DaemonError),
    Invalid(DaemonError),
}

fn validate_response(
    response: ExtensionResponse,
    expected_id: u64,
) -> std::result::Result<serde_json::Value, ResponseFailure> {
    if response.jsonrpc != JSONRPC_VERSION {
        return Err(ResponseFailure::Invalid(DaemonError::Extension(format!(
            "Extension response used unsupported JSON-RPC version '{}'",
            response.jsonrpc
        ))));
    }
    if response.id != expected_id {
        return Err(ResponseFailure::Invalid(DaemonError::Extension(format!(
            "Extension response ID {} did not match request ID {expected_id}",
            response.id
        ))));
    }
    match (response.result, response.error) {
        (Some(value), None) => Ok(value),
        (None, Some(RpcError { code, message, .. })) => Err(ResponseFailure::Rpc(
            DaemonError::Extension(format!("RPC error {code}: {message}")),
        )),
        _ => Err(ResponseFailure::Invalid(DaemonError::Extension(
            "Extension response must contain exactly one of result or error".into(),
        ))),
    }
}

fn validate_negotiation(
    expected: ExpectedExtension,
    offered_versions: &[String],
    result: InitializeResult,
) -> Result<NegotiatedSession> {
    if !offered_versions.contains(&result.protocol_version) {
        return Err(DaemonError::Extension(format!(
            "Extension selected protocol version '{}' that the host did not offer",
            result.protocol_version
        )));
    }
    if result.extension.id != expected.id {
        return Err(DaemonError::Extension(format!(
            "Extension identity changed during initialization: expected '{}', initialized '{}'",
            expected.id, result.extension.id
        )));
    }
    let unique: HashSet<_> = result.extension.types.iter().copied().collect();
    if unique.len() != result.extension.types.len() {
        return Err(DaemonError::Extension(
            "Extension initialization repeated a capability kind".into(),
        ));
    }
    if let Some(discovered) = expected.discovered
        && (result.extension.version != discovered.version
            || result.extension.name != discovered.name
            || unique != discovered.types.iter().copied().collect())
    {
        return Err(DaemonError::Extension(format!(
            "Extension '{}' initialization metadata disagreed with discovery",
            expected.id
        )));
    }
    Ok(NegotiatedSession {
        protocol_version: result.protocol_version,
        extension: result.extension,
        capabilities: result.capabilities,
    })
}

fn required_capability(method: &str) -> Option<ExtensionType> {
    match method {
        methods::COMPILE => Some(ExtensionType::Frontend),
        methods::GENERATE => Some(ExtensionType::Backend),
        methods::VALIDATE => Some(ExtensionType::Validator),
        methods::TRANSFORM => Some(ExtensionType::Transform),
        _ => None,
    }
}

/// Factory for Extism-backed typestate sessions.
pub struct ExtismSession;
impl ExtismSession {
    /// Create a loaded MEP session around an Extism container.
    pub fn connect(container: ExtensionContainer) -> Session<ExtismTransport, Loaded> {
        Session::loaded(ExtismTransport { container })
    }
}

/// Extism implementation of the object-safe MEP transport.
pub struct ExtismTransport {
    container: ExtensionContainer,
}

#[async_trait]
impl MepTransport for ExtismTransport {
    fn expected_extension(&self) -> ExpectedExtension {
        ExpectedExtension::discovered(self.container.info().clone())
    }
    async fn exchange(
        &mut self,
        request: ExtensionRequest,
    ) -> std::result::Result<ExtensionResponse, TransportError> {
        let bytes = serde_json::to_vec(&request)
            .map_err(|error| TransportError::new(error.into(), TransportState::Indeterminate))?;
        let output = self
            .container
            .call_raw("handle", &bytes)
            .await
            .map_err(|error| TransportError::new(error, TransportState::Indeterminate))?;
        serde_json::from_slice(&output)
            .map_err(|error| TransportError::new(error.into(), TransportState::Indeterminate))
    }
    async fn terminate(&mut self) -> std::result::Result<TransportState, TransportError> {
        Ok(TransportState::Stopped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_extension_sdk::ExtensionCapabilities;
    use std::collections::VecDeque;

    struct FakeTransport {
        expected: ExpectedExtension,
        responses: VecDeque<std::result::Result<ExtensionResponse, TransportError>>,
        termination: TransportState,
    }

    #[async_trait]
    impl MepTransport for FakeTransport {
        fn expected_extension(&self) -> ExpectedExtension {
            self.expected.clone()
        }
        async fn exchange(
            &mut self,
            _: ExtensionRequest,
        ) -> std::result::Result<ExtensionResponse, TransportError> {
            self.responses
                .pop_front()
                .expect("a response should be arranged")
        }
        async fn terminate(&mut self) -> std::result::Result<TransportState, TransportError> {
            Ok(self.termination)
        }
    }

    fn extension(types: Vec<ExtensionType>) -> ExtensionInfo {
        ExtensionInfo {
            id: "example".into(),
            name: "Example".into(),
            version: "1.0.0".into(),
            types,
            ..Default::default()
        }
    }

    fn initialization(info: ExtensionInfo) -> InitializeResult {
        InitializeResult {
            protocol_version: "0.1".into(),
            extension: info,
            capabilities: ExtensionCapabilities::default(),
        }
    }

    fn params() -> InitializeParams {
        InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: crate::extensions::protocol::PeerInfo {
                name: "test".into(),
                version: "1".into(),
            },
        }
    }

    fn transport(expected: ExpectedExtension, response: ExtensionResponse) -> FakeTransport {
        FakeTransport {
            expected,
            responses: VecDeque::from([Ok(response)]),
            termination: TransportState::Stopped,
        }
    }

    #[tokio::test]
    async fn rejects_an_invalid_response_envelope_before_negotiation() {
        let mut response =
            ExtensionResponse::success(1, initialization(extension(vec![ExtensionType::Backend])))
                .unwrap();
        response.jsonrpc = "1.0".into();
        let failure = Session::loaded(transport(
            ExpectedExtension::identified("example"),
            response,
        ))
        .initialize(params())
        .await
        .err()
        .expect("the envelope should fail");
        assert!(matches!(failure, FailedSession::Stopped(_, _)));
        assert!(failure.error().to_string().contains("JSON-RPC version"));
    }

    #[tokio::test]
    async fn rejects_capability_drift_from_discovery() {
        let discovered = extension(vec![ExtensionType::Backend]);
        let initialized = extension(vec![ExtensionType::Frontend]);
        let response = ExtensionResponse::success(1, initialization(initialized)).unwrap();
        let failure = Session::loaded(transport(
            ExpectedExtension::discovered(discovered),
            response,
        ))
        .initialize(params())
        .await
        .err()
        .expect("capability drift should fail");
        assert!(
            failure
                .error()
                .to_string()
                .contains("disagreed with discovery")
        );
    }

    #[tokio::test]
    async fn rejects_duplicate_capability_kinds() {
        let response = ExtensionResponse::success(
            1,
            initialization(extension(vec![
                ExtensionType::Backend,
                ExtensionType::Backend,
            ])),
        )
        .unwrap();
        let failure = Session::loaded(transport(
            ExpectedExtension::identified("example"),
            response,
        ))
        .initialize(params())
        .await
        .err()
        .expect("duplicates should fail");
        assert!(
            failure
                .error()
                .to_string()
                .contains("repeated a capability")
        );
    }

    #[tokio::test]
    async fn retains_an_indeterminate_state_after_an_uncertain_exchange_failure() {
        let transport = FakeTransport {
            expected: ExpectedExtension::identified("example"),
            responses: VecDeque::from([Err(TransportError::new(
                DaemonError::Extension("connection lost".into()),
                TransportState::Indeterminate,
            ))]),
            termination: TransportState::Indeterminate,
        };
        let failure = Session::loaded(transport)
            .initialize(params())
            .await
            .err()
            .expect("the exchange should fail");
        assert!(matches!(failure, FailedSession::Indeterminate(_, _)));
    }

    #[test]
    fn transport_trait_remains_object_safe() {
        fn accepts(_: Box<dyn MepTransport>) {}
        let response = ExtensionResponse::success(1, serde_json::json!({})).unwrap();
        accepts(Box::new(transport(
            ExpectedExtension::identified("example"),
            response,
        )));
    }
}

/// Observable lifecycle state for the compatibility session interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionSessionState {
    /// The extension has loaded but has not negotiated MEP.
    Starting,
    /// The extension has negotiated MEP and accepts operations.
    Ready,
    /// The extension completed shutdown.
    Stopped,
}

/// Compatibility interface for callers that erase typestate at runtime.
#[async_trait]
pub trait ExtensionSession {
    /// Return the current runtime-erased state.
    fn state(&self) -> ExtensionSessionState;
    /// Negotiate a protocol version and capabilities.
    async fn initialize(&mut self, params: InitializeParams) -> Result<InitializeResult>;
    /// Invoke one operation with JSON values.
    async fn invoke(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value>;
    /// Complete the session lifecycle.
    async fn shutdown(&mut self) -> Result<()>;
}
