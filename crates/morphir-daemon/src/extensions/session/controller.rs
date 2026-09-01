//! Typestate session controller and validated negotiation data.

use super::transport::{MepTransport, TransportError, TransportState};
use super::validation::{
    ResponseFailure, validate_method_result_async, validate_negotiation, validate_response,
};
use crate::DaemonError;
use crate::extensions::protocol::{
    ExtensionRequest, InitializeParams, InitializeResult, error_codes, methods,
};
use morphir_extension_sdk::{ExtensionCapabilities, ExtensionInfo, ExtensionType};
use morphir_workspace::{DiscoveryRequest, WORKSPACE_DISCOVERY_PROTOCOL};
use serde::{Serialize, de::DeserializeOwned};
use std::marker::PhantomData;

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
    pub(super) protocol_version: String,
    pub(super) extension: ExtensionInfo,
    pub(super) capabilities: ExtensionCapabilities,
    pub(super) legacy_backend: bool,
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
    pub fn capabilities(&self) -> &ExtensionCapabilities {
        &self.capabilities
    }

    pub(crate) fn supports_method(&self, method: &str) -> bool {
        match method {
            methods::COMPILE => {
                self.extension.types.contains(&ExtensionType::Frontend)
                    && self
                        .capabilities
                        .frontend
                        .as_ref()
                        .is_some_and(|frontend| frontend.compile)
            }
            methods::GENERATE => {
                self.extension.types.contains(&ExtensionType::Backend)
                    && (self.legacy_backend
                        || self
                            .capabilities
                            .backend
                            .as_ref()
                            .is_some_and(|backend| backend.generate))
            }
            methods::VALIDATE => self.extension.types.contains(&ExtensionType::Validator),
            methods::TRANSFORM => self.extension.types.contains(&ExtensionType::Transform),
            methods::WORKSPACE_DISCOVER => {
                self.extension.types.contains(&ExtensionType::Workspace)
                    && self
                        .capabilities
                        .workspace
                        .as_ref()
                        .is_some_and(|workspace| {
                            workspace.discover && workspace.protocol_versions.contains(&1)
                        })
            }
            _ => true,
        }
    }

    pub(crate) fn supports_invocation(&self, method: &str, params: &serde_json::Value) -> bool {
        if method != methods::WORKSPACE_DISCOVER {
            return true;
        }
        let Some(workspace) = self.capabilities.workspace.as_ref() else {
            return false;
        };
        serde_json::from_value::<DiscoveryRequest>(params.clone())
            .ok()
            .is_some_and(|request| {
                request.protocol_version == WORKSPACE_DISCOVERY_PROTOCOL
                    && workspace
                        .protocol_versions
                        .contains(&request.protocol_version)
            })
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
            CallOutcome::RpcError(error)
            | CallOutcome::Invalid(error)
            | CallOutcome::Local(error) => return Err(self.fail_after_abort(error).await),
            CallOutcome::Transport(error) => return Err(self.failed(error)),
        };
        let negotiated = match validate_negotiation(expected, &offered, result) {
            Ok(value) => value,
            Err(error) => return Err(self.fail_after_abort(error).await),
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
        if matches!(
            method,
            methods::INITIALIZE | methods::SHUTDOWN | methods::EXIT
        ) {
            return InvokeOutcome::Rejected(
                self,
                DaemonError::Extension(format!(
                    "Protocol lifecycle method '{method}' must use its dedicated session operation"
                )),
            );
        }
        if !self.negotiated().supports_method(method) {
            return InvokeOutcome::Rejected(
                self,
                DaemonError::Extension(format!(
                    "RPC error {}: Extension does not support capability '{method}'",
                    error_codes::CAPABILITY_UNAVAILABLE
                )),
            );
        }
        let params = match serde_json::to_value(params) {
            Ok(params) => params,
            Err(error) => return InvokeOutcome::Rejected(self, error.into()),
        };
        if !self.negotiated().supports_invocation(method, &params) {
            return InvokeOutcome::Rejected(
                self,
                DaemonError::Extension(format!(
                    "RPC error {}: Extension does not support capability '{method}' for the requested protocol",
                    error_codes::CAPABILITY_UNAVAILABLE
                )),
            );
        }
        match self.call(method, params).await {
            CallOutcome::Success(value) => InvokeOutcome::Success(self, value),
            CallOutcome::RpcError(error) | CallOutcome::Local(error) => {
                InvokeOutcome::Rejected(self, error)
            }
            CallOutcome::Invalid(error) => {
                InvokeOutcome::Failed(self.fail_after_abort(error).await)
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
                    Box::new(self.transition(None)),
                    DaemonError::Extension("Extension shutdown outcome is indeterminate".into()),
                )),
                Err(error) => Err(self.failed(error)),
            },
            CallOutcome::RpcError(error)
            | CallOutcome::Invalid(error)
            | CallOutcome::Local(error) => Err(self.fail_after_abort(error).await),
            CallOutcome::Transport(error) => Err(self.failed(error)),
        }
    }
}

impl<T, S> Session<T, S> {
    pub(crate) fn transport_internal(&self) -> &T {
        &self.transport
    }

    pub(crate) fn transport_mut_internal(&mut self) -> &mut T {
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
                return CallOutcome::Local(DaemonError::Extension(
                    "Extension request identifier overflowed".into(),
                ));
            }
        };
        let request = match ExtensionRequest::new(method, params, id) {
            Ok(request) => request,
            Err(error) => return CallOutcome::Local(error.into()),
        };
        let request_params = request.params.clone();
        let response = match self.transport.exchange(request).await {
            Ok(response) => response,
            Err(error) => return CallOutcome::Transport(error),
        };
        match validate_response(response, id) {
            Ok(value) => match validate_method_result_async(method, request_params, value)
                .await
                .and_then(|value| serde_json::from_value(value).map_err(Into::into))
            {
                Ok(value) => CallOutcome::Success(value),
                Err(error) => CallOutcome::Invalid(error),
            },
            Err(ResponseFailure::Rpc(error)) => CallOutcome::RpcError(error),
            Err(ResponseFailure::Invalid(error)) => CallOutcome::Invalid(error),
        }
    }

    async fn fail_after_abort(mut self, error: DaemonError) -> FailedSession<T> {
        match self.transport.abort().await {
            Ok(TransportState::Stopped) => {
                FailedSession::Stopped(Box::new(self.transition(None)), error)
            }
            Ok(TransportState::Indeterminate) => {
                FailedSession::Indeterminate(Box::new(self.transition(None)), error)
            }
            Err(abort) => {
                let state = abort.state;
                self.failed(TransportError::new(
                    DaemonError::Extension(format!(
                        "{error}; transport abort also failed: {}",
                        abort.error
                    )),
                    state,
                ))
            }
        }
    }

    fn failed(self, failure: TransportError) -> FailedSession<T> {
        match failure.state {
            TransportState::Stopped => {
                FailedSession::Stopped(Box::new(self.transition(None)), failure.error)
            }
            TransportState::Indeterminate => {
                FailedSession::Indeterminate(Box::new(self.transition(None)), failure.error)
            }
        }
    }
}

/// A failed transition paired with the only state the host can prove.
pub enum FailedSession<T> {
    /// The transport proved that the peer stopped.
    Stopped(Box<Session<T, Stopped>>, DaemonError),
    /// The transport could not prove the peer's state.
    Indeterminate(Box<Session<T, Indeterminate>>, DaemonError),
}

impl<T> FailedSession<T> {
    /// Return the failure that caused the state transition.
    pub fn error(&self) -> &DaemonError {
        match self {
            Self::Stopped(_, error) | Self::Indeterminate(_, error) => error,
        }
    }

    /// Consume the failed session and take ownership of its cause.
    ///
    /// [`Self::error`] can only lend the cause, which forces callers that need
    /// to propagate it to stringify it and lose its variant.
    pub fn into_error(self) -> DaemonError {
        match self {
            Self::Stopped(_, error) | Self::Indeterminate(_, error) => error,
        }
    }
}

enum CallOutcome<R> {
    Success(R),
    RpcError(DaemonError),
    Local(DaemonError),
    Invalid(DaemonError),
    Transport(TransportError),
}
