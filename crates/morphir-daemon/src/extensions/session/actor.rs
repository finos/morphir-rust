//! One Kameo actor per MEP session, owning the session across invocations.
//!
//! A [`Session`] is a typestate value: [`Session::invoke`] consumes it and hands
//! it back inside an [`InvokeOutcome`]. Sharing one behind a lock would mean
//! taking it out and putting it back on every call, with an empty slot as the
//! cost of every missed failure path. An actor instead owns the session as its
//! own state and rebinds it after each message, so the typestate is respected
//! and no lock is involved.
//!
//! Kameo does not escape this module. [`SessionHandle`] erases both the
//! transport type and the actor framework, so callers hold one handle type and
//! never depend on `kameo`.

use kameo::Actor;
use kameo::actor::{ActorRef, Spawn as _};
use kameo::error::SendError;
use kameo::message::{Context, Message};
use serde::{Serialize, de::DeserializeOwned};

use super::{InvokeOutcome, MepTransport, Ready, Session};
use crate::DaemonError;

/// Invoke one MEP operation on the owned session.
struct Invoke {
    method: String,
    params: serde_json::Value,
}

/// Complete MEP shutdown and stop the actor.
struct Shutdown;

/// An actor owning one ready MEP session.
///
/// The session is held as an `Option` only because [`Session::invoke`] takes it
/// by value. That `Option` is private to a single-threaded actor rather than
/// shared state, so `None` means exactly one thing: the session is gone and the
/// actor is on its way out.
#[derive(Actor)]
struct SessionActor<T: MepTransport + Send + 'static> {
    session: Option<Session<T, Ready>>,
}

/// The one error reported once the session can no longer serve requests.
fn gone(detail: impl std::fmt::Display) -> DaemonError {
    DaemonError::Extension(format!(
        "Extension session is no longer available: {detail}"
    ))
}

impl<T: MepTransport + Send + 'static> Message<Invoke> for SessionActor<T> {
    type Reply = Result<serde_json::Value, DaemonError>;

    async fn handle(&mut self, msg: Invoke, ctx: &mut Context<Self, Self::Reply>) -> Self::Reply {
        let Some(session) = self.session.take() else {
            ctx.stop();
            return Err(gone("the session was already released"));
        };
        match session
            .invoke::<serde_json::Value>(&msg.method, msg.params)
            .await
        {
            InvokeOutcome::Success(session, value) => {
                self.session = Some(session);
                Ok(value)
            }
            InvokeOutcome::Rejected(session, error) => {
                self.session = Some(session);
                Err(error)
            }
            InvokeOutcome::Failed(failure) => {
                // The session is unrecoverable, so reply with the failure and
                // stop once this message is done. `self.session` stays `None`,
                // which keeps any message that races the stop deterministic.
                ctx.stop();
                Err(DaemonError::Extension(failure.error().to_string()))
            }
        }
    }
}

impl<T: MepTransport + Send + 'static> Message<Shutdown> for SessionActor<T> {
    type Reply = Result<(), DaemonError>;

    async fn handle(
        &mut self,
        _msg: Shutdown,
        ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        ctx.stop();
        let Some(session) = self.session.take() else {
            return Err(gone("the session was already released"));
        };
        session
            .shutdown()
            .await
            .map(|_stopped| ())
            .map_err(|failure| DaemonError::Extension(failure.error().to_string()))
    }
}

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
fn delivered<M, R>(result: Result<R, SendError<M, DaemonError>>) -> Result<R, DaemonError> {
    match result {
        Ok(value) => Ok(value),
        Err(SendError::HandlerError(error)) => Err(error),
        Err(undeliverable) => Err(gone(undeliverable)),
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
    pub async fn shutdown(&self) -> Result<(), DaemonError> {
        self.dispatch.shutdown().await
    }
}

/// Spawn one actor owning the given ready session and return its handle.
///
/// Must be called from within a Tokio runtime; the actor runs on its own task.
pub fn spawn_session<T: MepTransport + Send + 'static>(
    session: Session<T, Ready>,
) -> SessionHandle {
    let actor_ref = SessionActor::spawn(SessionActor {
        session: Some(session),
    });
    SessionHandle {
        dispatch: std::sync::Arc::new(actor_ref),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DaemonError;
    use crate::extensions::protocol::{ExtensionResponse, RpcError};
    use crate::extensions::session::tests::{FakeTransport, backend_initialization, params};
    use crate::extensions::session::{
        ExpectedExtension, Ready, Session, TransportError, TransportState,
    };
    use std::collections::VecDeque;

    /// A response envelope the fake transport hands back, or a transport failure.
    type Exchange = std::result::Result<ExtensionResponse, TransportError>;

    /// A well-formed `GenerateResult` distinguished by its single artifact path.
    fn generate_result(path: &str) -> serde_json::Value {
        serde_json::json!({
            "success": true,
            "artifacts": [{"path": path, "content": "{}"}],
            "diagnostics": []
        })
    }

    fn generated_paths(value: &serde_json::Value) -> Vec<String> {
        value["artifacts"]
            .as_array()
            .expect("a generate result carries artifacts")
            .iter()
            .map(|artifact| artifact["path"].as_str().expect("a path").to_owned())
            .collect()
    }

    /// Negotiate a backend session whose transport replays `exchanges` afterwards.
    ///
    /// Negotiation consumes response id 1, so the first replayed exchange must
    /// answer id 2. Response ids are validated against the session's own request
    /// counter, which makes the id sequence a proof that one session was reused.
    async fn ready_backend_session(
        exchanges: impl IntoIterator<Item = Exchange>,
    ) -> Session<FakeTransport, Ready> {
        let mut responses: VecDeque<Exchange> = VecDeque::new();
        responses.push_back(Ok(ExtensionResponse::success(
            1,
            backend_initialization(true),
        )
        .expect("a valid envelope")));
        responses.extend(exchanges);
        let transport = FakeTransport {
            expected: ExpectedExtension::identified("example"),
            responses,
            termination: TransportState::Stopped,
        };
        match Session::loaded(transport).initialize(params()).await {
            Ok(session) => session,
            Err(failure) => panic!("negotiation should succeed: {}", failure.error()),
        }
    }

    async fn ready_session_answering(
        results: &[serde_json::Value],
    ) -> Session<FakeTransport, Ready> {
        let exchanges: Vec<Exchange> = results
            .iter()
            .enumerate()
            .map(|(index, result)| {
                Ok(ExtensionResponse::success(index as u64 + 2, result).expect("a valid envelope"))
            })
            .collect();
        ready_backend_session(exchanges).await
    }

    async fn ready_session_rejecting_then_answering(
        result: serde_json::Value,
    ) -> Session<FakeTransport, Ready> {
        ready_backend_session([
            Ok(ExtensionResponse::error(
                2,
                RpcError::extension_error("the extension refused this request"),
            )),
            Ok(ExtensionResponse::success(3, result).expect("a valid envelope")),
        ])
        .await
    }

    async fn ready_session_failing_transport() -> Session<FakeTransport, Ready> {
        ready_backend_session([Err(TransportError::new(
            DaemonError::Extension("the transport pipe broke".into()),
            TransportState::Stopped,
        ))])
        .await
    }

    #[tokio::test]
    async fn sequential_invocations_reuse_one_session() {
        let handle = spawn_session(
            ready_session_answering(&[
                generate_result("first.avro"),
                generate_result("second.avro"),
            ])
            .await,
        );

        let first: serde_json::Value = handle
            .invoke("morphir.backend.generate", serde_json::json!({}))
            .await
            .unwrap();
        let second: serde_json::Value = handle
            .invoke("morphir.backend.generate", serde_json::json!({}))
            .await
            .unwrap();

        assert_eq!(generated_paths(&first), ["first.avro"]);
        assert_eq!(generated_paths(&second), ["second.avro"]);
    }

    #[tokio::test]
    async fn a_rejected_invocation_keeps_the_session_usable() {
        let handle = spawn_session(
            ready_session_rejecting_then_answering(generate_result("recovered.avro")).await,
        );

        let rejected = handle
            .invoke::<serde_json::Value>("morphir.backend.generate", serde_json::json!({}))
            .await;
        assert!(rejected.is_err(), "unexpected result: {rejected:?}");

        let recovered: serde_json::Value = handle
            .invoke("morphir.backend.generate", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(generated_paths(&recovered), ["recovered.avro"]);
    }

    #[tokio::test]
    async fn a_failed_invocation_stops_the_actor() {
        let handle = spawn_session(ready_session_failing_transport().await);

        let failed = handle
            .invoke::<serde_json::Value>("morphir.backend.generate", serde_json::json!({}))
            .await;
        assert!(failed.is_err(), "unexpected result: {failed:?}");

        let after = handle
            .invoke::<serde_json::Value>("morphir.backend.generate", serde_json::json!({}))
            .await;
        assert!(
            matches!(after, Err(DaemonError::Extension(ref message)) if message.contains("session is no longer available")),
            "unexpected result: {after:?}"
        );
        // An actor that merely dropped its session would still accept the
        // message and answer with the released-session detail. Undeliverable
        // means the actor itself is gone.
        assert!(
            matches!(after, Err(DaemonError::Extension(ref message)) if !message.contains("already released")),
            "the actor kept accepting messages: {after:?}"
        );
    }

    #[tokio::test]
    async fn shutdown_stops_the_actor_and_later_calls_report_it() {
        let handle = spawn_session(
            ready_backend_session([Ok(
                ExtensionResponse::success(2, serde_json::json!({})).expect("a valid envelope")
            )])
            .await,
        );

        handle.shutdown().await.unwrap();

        let after = handle
            .invoke::<serde_json::Value>("morphir.backend.generate", serde_json::json!({}))
            .await;
        assert!(
            matches!(after, Err(DaemonError::Extension(ref message)) if message.contains("session is no longer available")),
            "unexpected result: {after:?}"
        );
        assert!(
            matches!(after, Err(DaemonError::Extension(ref message)) if !message.contains("already released")),
            "the actor kept accepting messages: {after:?}"
        );
    }
}
