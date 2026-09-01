//! The entry points that put an actor, a watchdog and a handle together.

use kameo::actor::Spawn as _;

use super::handle::SessionHandle;
use super::lifecycle::SessionActor;
use super::watchdog::{SessionActivity, spawn_idle_watchdog};
use crate::extensions::session::{MepTransport, Ready, Session};

/// Spawn one actor owning the given ready session and return its handle.
///
/// The session stops itself after 300 seconds with no invocations; see
/// [`spawn_session_with_idle_timeout`] to choose a different duration.
///
/// Must be called from within a Tokio runtime; the actor runs on its own task.
///
/// # Examples
///
/// One handle serves many calls over the same negotiated session, and ends it
/// when the caller is done. The extension behind this example is a stand-in;
/// a real one arrives already negotiated from
/// [`activate_transport`](crate::extensions::activate_transport).
///
/// ```
/// # use async_trait::async_trait;
/// # use morphir_daemon::extensions::protocol::{
/// #     ExtensionRequest, ExtensionResponse, InitializeParams, InitializeResult, PeerInfo,
/// # };
/// # use morphir_daemon::extensions::session::{
/// #     ExpectedExtension, MepTransport, Session, TransportError, TransportState,
/// # };
/// # use morphir_extension_sdk::{
/// #     BackendCapability, ExtensionCapabilities, ExtensionInfo, ExtensionType,
/// # };
/// use morphir_daemon::extensions::session::spawn_session;
///
/// # /// Answers every request in place of a real extension subprocess.
/// # struct ScriptedExtension {
/// #     generated: usize,
/// # }
/// #
/// # #[async_trait]
/// # impl MepTransport for ScriptedExtension {
/// #     fn expected_extension(&self) -> ExpectedExtension {
/// #         ExpectedExtension::identified("example")
/// #     }
/// #
/// #     async fn exchange(
/// #         &mut self,
/// #         request: ExtensionRequest,
/// #     ) -> Result<ExtensionResponse, TransportError> {
/// #         let result = match request.method.as_str() {
/// #             "morphir.initialize" => serde_json::to_value(InitializeResult {
/// #                 protocol_version: "0.1".to_owned(),
/// #                 extension: ExtensionInfo {
/// #                     id: "example".to_owned(),
/// #                     name: "Example".to_owned(),
/// #                     version: "1.0.0".to_owned(),
/// #                     types: vec![ExtensionType::Backend],
/// #                     ..ExtensionInfo::default()
/// #                 },
/// #                 capabilities: ExtensionCapabilities {
/// #                     backend: Some(BackendCapability {
/// #                         targets: vec!["avro".to_owned()],
/// #                         ir_versions: vec!["3".to_owned()],
/// #                         generate: true,
/// #                     }),
/// #                     ..ExtensionCapabilities::default()
/// #                 },
/// #             })
/// #             .unwrap(),
/// #             "morphir.backend.generate" => {
/// #                 self.generated += 1;
/// #                 serde_json::json!({
/// #                     "success": true,
/// #                     "artifacts": [{
/// #                         "path": format!("schema-{}.avsc", self.generated),
/// #                         "content": "{}"
/// #                     }],
/// #                     "diagnostics": []
/// #                 })
/// #             }
/// #             _ => serde_json::json!({}),
/// #         };
/// #         Ok(ExtensionResponse::success(request.id, result).unwrap())
/// #     }
/// #
/// #     async fn terminate(&mut self) -> Result<TransportState, TransportError> {
/// #         Ok(TransportState::Stopped)
/// #     }
/// # }
/// #
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let session = Session::loaded(ScriptedExtension { generated: 0 })
/// #     .initialize(InitializeParams {
/// #         protocol_versions: vec!["0.1".to_owned()],
/// #         host: PeerInfo { name: "morphir".to_owned(), version: "1".to_owned() },
/// #     })
/// #     .await
/// #     .map_err(|failure| failure.into_error())?;
/// let handle = spawn_session(session);
///
/// // Both calls run on the same session: nothing is launched per invocation,
/// // and the extension keeps whatever state it built up for the first one.
/// let first: serde_json::Value = handle
///     .invoke("morphir.backend.generate", serde_json::json!({"target": "avro"}))
///     .await?;
/// let second: serde_json::Value = handle
///     .invoke("morphir.backend.generate", serde_json::json!({"target": "avro"}))
///     .await?;
/// assert_eq!(first["artifacts"][0]["path"], "schema-1.avsc");
/// assert_eq!(second["artifacts"][0]["path"], "schema-2.avsc");
///
/// // Completes the MEP shutdown handshake and stops the actor. Dropping the
/// // last handle, or leaving the session idle, ends it the same way.
/// handle.shutdown().await?;
/// # Ok(())
/// # }
/// ```
pub fn spawn_session<T: MepTransport + Send + 'static>(
    session: Session<T, Ready>,
) -> SessionHandle {
    spawn_session_with_idle_timeout(session, std::time::Duration::from_secs(300))
}

/// Spawn one actor owning the given ready session, stopping it after `idle`
/// passes with no invocations.
///
/// Only a session that did nothing for a full `idle` period is stopped. An
/// invocation in flight makes the session ineligible to stop no matter how long
/// it runs, and the next idle period begins when it is answered, so `idle` may
/// safely be shorter than the slowest operation the extension performs.
/// Stopping completes the MEP shutdown handshake via
/// [`Actor::on_stop`](kameo::Actor::on_stop), the same as dropping the last
/// handle would.
///
/// Must be called from within a Tokio runtime; the actor runs on its own task.
pub fn spawn_session_with_idle_timeout<T: MepTransport + Send + 'static>(
    session: Session<T, Ready>,
    idle: std::time::Duration,
) -> SessionHandle {
    let (activity, receiver) = tokio::sync::watch::channel(SessionActivity::Idle);
    let actor_ref = SessionActor::spawn(SessionActor {
        session: Some(session),
        activity,
    });
    // Downgraded on purpose: kameo stops an actor once the last *strong*
    // reference is dropped, so a watchdog holding one would keep the actor —
    // and the extension subprocess behind it — alive for the whole idle
    // window even after every caller has let go of its handle.
    spawn_idle_watchdog(actor_ref.downgrade(), receiver, idle);
    SessionHandle::erasing(actor_ref)
}
