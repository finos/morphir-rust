//! The handle callers hold, and the erased dispatch beneath it.
//!
//! Everything kameo-shaped stops here: a [`SessionHandle`] names no actor type
//! and no transport type, and a delivery failure is retold in this module's own
//! vocabulary before it reaches a caller.

use kameo::actor::ActorRef;
use kameo::error::SendError;
use serde::{Serialize, de::DeserializeOwned};

use super::gone;
use super::lifecycle::SessionActor;
use super::messages::{Invoke, Shutdown};
use crate::DaemonError;
use crate::extensions::session::MepTransport;

/// A transport-erased, framework-erased handle to one session actor.
///
/// Cloning a handle shares the same actor and therefore the same session.
#[derive(Clone)]
pub struct SessionHandle {
    dispatch: std::sync::Arc<dyn SessionDispatch>,
}

impl std::fmt::Debug for SessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHandle").finish_non_exhaustive()
    }
}

/// The object-safe surface of a session actor, hiding its transport type.
#[async_trait::async_trait]
trait SessionDispatch: Send + Sync {
    async fn invoke(
        &self,
        method: String,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError>;

    async fn shutdown(&self) -> Result<(), DaemonError>;
}

/// Translate a Kameo delivery result, keeping handler errors verbatim.
///
/// A delivery failure is described in this module's own vocabulary rather than
/// by forwarding kameo's `Display`. This module promises the framework does not
/// escape, and "Extension session is no longer available: Extension error:
/// actor stopped" would break that promise in the one place a user reads.
fn delivered<M, R>(result: Result<R, SendError<M, DaemonError>>) -> Result<R, DaemonError> {
    match result {
        Ok(value) => Ok(value),
        Err(SendError::HandlerError(error)) => Err(error),
        Err(undeliverable) => Err(gone(DaemonError::Extension(
            undeliverable_cause(&undeliverable).to_owned(),
        ))),
    }
}

/// Describe why a request never reached the session, in domain terms.
fn undeliverable_cause<M, E>(error: &SendError<M, E>) -> &'static str {
    match error {
        SendError::ActorNotRunning(_) | SendError::ActorStopped | SendError::ActorRestarting(_) => {
            "the session ended before this request was handled"
        }
        SendError::MailboxFull(_) => "the session has too many requests in flight",
        SendError::Timeout(_) => "the session did not answer in time",
        // Handler errors are the extension's own reply and are returned
        // verbatim by `delivered`; this arm exists only for exhaustiveness.
        SendError::HandlerError(_) => "the session could not serve this request",
    }
}

#[async_trait::async_trait]
impl<T: MepTransport + Send + 'static> SessionDispatch for ActorRef<SessionActor<T>> {
    async fn invoke(
        &self,
        method: String,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        delivered(self.ask(Invoke { method, params }).await)
    }

    async fn shutdown(&self) -> Result<(), DaemonError> {
        delivered(self.ask(Shutdown).await)
    }
}

impl SessionHandle {
    /// Erase both the transport type and the actor framework behind a handle.
    pub(super) fn erasing<T: MepTransport + Send + 'static>(
        actor_ref: ActorRef<SessionActor<T>>,
    ) -> Self {
        Self {
            dispatch: std::sync::Arc::new(actor_ref),
        }
    }

    /// Invoke one MEP operation on the owned session and decode its result.
    pub async fn invoke<R: DeserializeOwned>(
        &self,
        method: &str,
        params: impl Serialize,
    ) -> Result<R, DaemonError> {
        let params = serde_json::to_value(params)?;
        let value = self.dispatch.invoke(method.to_owned(), params).await?;
        Ok(serde_json::from_value(value)?)
    }

    /// Complete MEP shutdown for the owned session and stop the actor.
    ///
    /// This is not a pre-emption: the request queues behind whatever is
    /// already in the actor's bounded mailbox (capacity 64), each entry of
    /// which can hold the actor for up to the transport's 30-second request
    /// timeout, so shutting down a hung extension can take far longer than one
    /// timeout to return.
    pub async fn shutdown(&self) -> Result<(), DaemonError> {
        self.dispatch.shutdown().await
    }
}
