//! Typestate session controller and validated negotiation data.

use super::transport::{MepTransport, TransportError, TransportState};
use super::validation::{DaemonChecks, validate_method_result_async};
use crate::DaemonError;
use crate::extensions::protocol::InitializeParams;
use morphir_host::{Action, Event, SessionCore};
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
pub use morphir_host::Negotiated as NegotiatedSession;

/// A MEP session whose legal operations depend on its state parameter.
pub struct Session<T, S> {
    transport: T,
    core: Option<SessionCore<DaemonChecks>>,
    marker: PhantomData<S>,
}

impl<T> Session<T, Loaded> {
    /// Wrap a loaded transport before negotiation.
    pub fn loaded(transport: T) -> Self {
        Self {
            transport,
            core: None,
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
        self.core = Some(SessionCore::new(DaemonChecks::new(
            self.transport.expected_extension(),
        )));
        match self.step(Event::Open(params)).await {
            Step::Action(Action::Ready) => Ok(self.transition()),
            Step::Action(Action::Failed(error) | Action::Rejected(error)) => {
                Err(self.fail_after_abort(error).await)
            }
            Step::Action(other) => Err(self.fail_after_abort(unexpected(&other)).await),
            Step::Transport(error) => Err(self.failed(error)),
        }
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
        self.core
            .as_ref()
            .and_then(SessionCore::negotiated)
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
        let params = match serde_json::to_value(params) {
            Ok(params) => params,
            Err(error) => return InvokeOutcome::Rejected(self, error.into()),
        };
        let event = Event::Call {
            method: method.to_owned(),
            params: params.clone(),
        };
        match self.step(event).await {
            Step::Action(Action::Completed(value)) => {
                match validate_method_result_async(method, params, value)
                    .await
                    .and_then(|value| serde_json::from_value(value).map_err(Into::into))
                {
                    Ok(value) => InvokeOutcome::Success(self, value),
                    Err(error) => InvokeOutcome::Failed(self.fail_after_abort(error).await),
                }
            }
            Step::Action(Action::Rejected(error)) => InvokeOutcome::Rejected(self, error),
            Step::Action(Action::Failed(error)) => {
                InvokeOutcome::Failed(self.fail_after_abort(error).await)
            }
            Step::Action(other) => {
                InvokeOutcome::Failed(self.fail_after_abort(unexpected(&other)).await)
            }
            Step::Transport(error) => InvokeOutcome::Failed(self.failed(error)),
        }
    }

    /// Complete MEP shutdown and prove the resulting lifecycle state.
    pub async fn shutdown(mut self) -> std::result::Result<Session<T, Stopped>, FailedSession<T>> {
        match self.step(Event::Close).await {
            Step::Action(Action::ShutDown) => match self.transport.terminate().await {
                Ok(TransportState::Stopped) => Ok(self.transition()),
                Ok(TransportState::Indeterminate) => Err(FailedSession::Indeterminate(
                    Box::new(self.transition()),
                    DaemonError::Extension("Extension shutdown outcome is indeterminate".into()),
                )),
                Err(error) => Err(self.failed(error)),
            },
            Step::Action(Action::Failed(error) | Action::Rejected(error)) => {
                Err(self.fail_after_abort(error).await)
            }
            Step::Action(other) => Err(self.fail_after_abort(unexpected(&other)).await),
            Step::Transport(error) => Err(self.failed(error)),
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

    fn transition<N>(self) -> Session<T, N> {
        Session {
            transport: self.transport,
            core: self.core,
            marker: PhantomData,
        }
    }
}

impl<T: MepTransport, S> Session<T, S> {
    /// Feed one event to the core, and exchange requests until it needs the caller.
    async fn step(&mut self, event: Event) -> Step {
        let core = self
            .core
            .as_mut()
            .expect("the session core exists once initialization starts");
        let mut action = core.handle(event);
        while let Action::Send(request) = action {
            match self.transport.exchange(request).await {
                Ok(response) => action = core.handle(Event::Received(response)),
                Err(error) => {
                    core.handle(Event::TransportFailed);
                    return Step::Transport(error);
                }
            }
        }
        Step::Action(action)
    }

    async fn fail_after_abort(mut self, error: DaemonError) -> FailedSession<T> {
        match self.transport.abort().await {
            Ok(TransportState::Stopped) => {
                FailedSession::Stopped(Box::new(self.transition()), error)
            }
            Ok(TransportState::Indeterminate) => {
                FailedSession::Indeterminate(Box::new(self.transition()), error)
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
                FailedSession::Stopped(Box::new(self.transition()), failure.error)
            }
            TransportState::Indeterminate => {
                FailedSession::Indeterminate(Box::new(self.transition()), failure.error)
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

/// How one step of the session ended.
enum Step {
    /// The core produced an action for the caller.
    Action(Action<DaemonError>),
    /// The transport failed while the core waited for an answer.
    Transport(TransportError),
}

fn unexpected(action: &Action<DaemonError>) -> DaemonError {
    DaemonError::Extension(format!(
        "Session core returned an unexpected action: {action:?}"
    ))
}
