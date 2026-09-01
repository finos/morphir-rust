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
use kameo::actor::{ActorRef, Spawn as _, WeakActorRef};
use kameo::error::{ActorStopReason, Infallible, SendError};
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
///
/// `activity` implements the idle timeout: it is bumped on every handled
/// [`Invoke`], and the watchdog task spawned alongside the actor (see
/// [`spawn_session_with_idle_timeout`]) resets its sleep every time it
/// observes a bump, stopping the actor only once a full idle period passes
/// with no activity at all.
struct SessionActor<T: MepTransport + Send + 'static> {
    session: Option<Session<T, Ready>>,
    activity: tokio::sync::watch::Sender<()>,
}

/// Hand-written instead of `#[derive(Actor)]` so `on_stop` can complete the
/// MEP shutdown handshake for a session that was never explicitly shut down
/// (dropped handle, idle timeout). `on_start` is otherwise exactly what the
/// derive macro generates: return the args unchanged.
impl<T: MepTransport + Send + 'static> Actor for SessionActor<T> {
    type Args = Self;
    type Error = Infallible;

    async fn on_start(state: Self::Args, _actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        Ok(state)
    }

    async fn on_stop(
        &mut self,
        _actor_ref: WeakActorRef<Self>,
        _reason: ActorStopReason,
    ) -> Result<(), Self::Error> {
        // The explicit `Shutdown` message already completes the handshake and
        // takes `self.session`. This hook also fires after that path (every
        // terminal stop runs it), so `None` here means "already handled,
        // nothing to do" rather than an error.
        let Some(session) = self.session.take() else {
            return Ok(());
        };
        // Best-effort: there is no caller left to report a failure to. A
        // remote extension daemon that never receives this handshake would
        // otherwise leak the session's state indefinitely.
        let _ = session.shutdown().await;
        Ok(())
    }
}

/// Report that the session is gone, wrapping the cause that ended it.
///
/// [`DaemonError::SessionLost`] is deliberately a different variant from the
/// [`DaemonError::Extension`] an extension returns when it refuses one
/// operation. A caller caching a handle needs to evict and respawn on the first
/// but not the second, and matching on message text is not a contract.
fn gone(cause: DaemonError) -> DaemonError {
    DaemonError::SessionLost(Box::new(cause))
}

impl<T: MepTransport + Send + 'static> Message<Invoke> for SessionActor<T> {
    type Reply = Result<serde_json::Value, DaemonError>;

    async fn handle(&mut self, msg: Invoke, ctx: &mut Context<Self, Self::Reply>) -> Self::Reply {
        // Bump the idle watchdog first: every handled message counts as
        // activity, whether or not the session is still there to use.
        let _ = self.activity.send(());
        let Some(session) = self.session.take() else {
            ctx.stop();
            return Err(gone(DaemonError::Extension(
                "the session was already released".into(),
            )));
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
                Err(gone(failure.into_error()))
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
            return Err(gone(DaemonError::Extension(
                "the session was already released".into(),
            )));
        };
        session
            .shutdown()
            .await
            .map(|_stopped| ())
            .map_err(|failure| gone(failure.into_error()))
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
        Err(undeliverable) => Err(gone(DaemonError::Extension(undeliverable.to_string()))),
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
/// The session stops itself after 300 seconds with no invocations; see
/// [`spawn_session_with_idle_timeout`] to choose a different duration.
///
/// Must be called from within a Tokio runtime; the actor runs on its own task.
pub fn spawn_session<T: MepTransport + Send + 'static>(
    session: Session<T, Ready>,
) -> SessionHandle {
    spawn_session_with_idle_timeout(session, std::time::Duration::from_secs(300))
}

/// Spawn one actor owning the given ready session, stopping it after `idle`
/// passes with no invocations.
///
/// Every [`SessionHandle::invoke`] resets the idle timer, so a busy session is
/// never stopped mid-use; only a session with no activity for a full `idle`
/// period is stopped. Stopping completes the MEP shutdown handshake via
/// [`Actor::on_stop`], the same as dropping the last handle would.
///
/// Must be called from within a Tokio runtime; the actor runs on its own task.
pub fn spawn_session_with_idle_timeout<T: MepTransport + Send + 'static>(
    session: Session<T, Ready>,
    idle: std::time::Duration,
) -> SessionHandle {
    let (activity, receiver) = tokio::sync::watch::channel(());
    let actor_ref = SessionActor::spawn(SessionActor {
        session: Some(session),
        activity,
    });
    // Downgraded on purpose: kameo stops an actor once the last *strong*
    // reference is dropped, so a watchdog holding one would keep the actor —
    // and the extension subprocess behind it — alive for the whole idle
    // window even after every caller has let go of its handle.
    spawn_idle_watchdog(actor_ref.downgrade(), receiver, idle);
    SessionHandle {
        dispatch: std::sync::Arc::new(actor_ref),
    }
}

/// Spawn the task that stops `actor_ref` after `idle` passes with no activity
/// on `receiver`.
///
/// The reference is deliberately weak. Kameo stops an actor when its last
/// strong reference drops, so a watchdog holding a strong one would keep the
/// session — and its extension subprocess — running for a full idle period
/// after the last [`SessionHandle`] was dropped. Weak also means the watchdog
/// can outlive the actor, which is why every use of it upgrades first.
///
/// Returns the watchdog's own `JoinHandle` so tests can observe when it
/// exits; production callers have no need for it; the watchdog either fires
/// (stopping the actor), finds the actor already gone, or notices `receiver`
/// closed (the actor already stopped some other way), and exits on its own in
/// every case.
fn spawn_idle_watchdog<T: MepTransport + Send + 'static>(
    actor_ref: WeakActorRef<SessionActor<T>>,
    mut receiver: tokio::sync::watch::Receiver<()>,
    idle: std::time::Duration,
) -> tokio::task::JoinHandle<()> {
    // Anchored synchronously, before this function returns and certainly
    // before any `.await` in it, so the deadline reflects actor-creation time
    // even though the watchdog task below won't itself run until the runtime
    // schedules it. `tokio::spawn` only enqueues the task; if the deadline
    // were instead computed lazily on the watchdog's first poll, a test that
    // advances a paused clock before the runtime takes a turn would compute
    // the deadline against the *already advanced* clock, requiring a second
    // full `idle` period to elapse before the timeout ever fired.
    let first_deadline = tokio::time::Instant::now() + idle;
    tokio::spawn(async move {
        let sleep = tokio::time::sleep_until(first_deadline);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => {
                    // No activity for a full `idle` period: stop gracefully
                    // rather than `kill`, so any message already in flight is
                    // answered before the actor goes away, and `on_stop` still
                    // gets to complete the MEP shutdown handshake.
                    //
                    // A failed upgrade means the actor stopped between the
                    // deadline firing and this poll; there is nothing left to
                    // stop, so the watchdog is done.
                    if let Some(actor_ref) = actor_ref.upgrade() {
                        let _ = actor_ref.stop_gracefully().await;
                    }
                    return;
                }
                changed = receiver.changed() => {
                    if changed.is_err() {
                        // The actor dropped its `activity` sender along with
                        // the rest of its state when it stopped for some
                        // other reason (explicit shutdown, a failed
                        // invocation). Nothing left for this watchdog to do.
                        return;
                    }
                    // Activity observed: re-arm the sleep instead of letting
                    // it fire on schedule.
                    sleep.as_mut().reset(tokio::time::Instant::now() + idle);
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DaemonError;
    use crate::extensions::protocol::{ExtensionResponse, RpcError, methods};
    use crate::extensions::session::tests::{
        FakeTransport, RequestLog, backend_initialization, params,
    };
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
    ///
    /// The returned [`RequestLog`] is shared with the transport, so it stays
    /// readable after the session is handed to an actor that never gives it back.
    async fn ready_backend_session(
        exchanges: impl IntoIterator<Item = Exchange>,
    ) -> (Session<FakeTransport, Ready>, RequestLog) {
        let mut responses: VecDeque<Exchange> = VecDeque::new();
        responses.push_back(Ok(ExtensionResponse::success(
            1,
            backend_initialization(true),
        )
        .expect("a valid envelope")));
        responses.extend(exchanges);
        let requests = RequestLog::default();
        let transport = FakeTransport {
            expected: ExpectedExtension::identified("example"),
            responses,
            termination: TransportState::Stopped,
            requests: requests.clone(),
        };
        match Session::loaded(transport).initialize(params()).await {
            Ok(session) => (session, requests),
            Err(failure) => panic!("negotiation should succeed: {}", failure.error()),
        }
    }

    async fn ready_session_answering(
        results: &[serde_json::Value],
    ) -> (Session<FakeTransport, Ready>, RequestLog) {
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
    ) -> (Session<FakeTransport, Ready>, RequestLog) {
        ready_backend_session([
            Ok(ExtensionResponse::error(
                2,
                RpcError::extension_error("the extension refused this request"),
            )),
            Ok(ExtensionResponse::success(3, result).expect("a valid envelope")),
        ])
        .await
    }

    /// A session whose next exchange fails at the transport with an `Io` cause.
    ///
    /// The cause is deliberately not an `Extension` error so that a test can
    /// prove the original variant survives being reported to the caller.
    async fn ready_session_failing_transport() -> (Session<FakeTransport, Ready>, RequestLog) {
        ready_backend_session([Err(TransportError::new(
            DaemonError::Io(std::io::Error::other("the transport pipe broke")),
            TransportState::Stopped,
        ))])
        .await
    }

    #[tokio::test]
    async fn sequential_invocations_reuse_one_session() {
        let (session, requests) = ready_session_answering(&[
            generate_result("first.avro"),
            generate_result("second.avro"),
        ])
        .await;
        let handle = spawn_session(session);

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
        assert_eq!(
            requests.methods(),
            [methods::INITIALIZE, methods::GENERATE, methods::GENERATE]
        );
    }

    #[tokio::test]
    async fn an_invocation_forwards_the_callers_method_and_params() {
        let (session, requests) = ready_session_answering(&[generate_result("out.avro")]).await;
        let handle = spawn_session(session);

        let _: serde_json::Value = handle
            .invoke(
                methods::GENERATE,
                serde_json::json!({"target": "avro", "options": {"pretty": true}}),
            )
            .await
            .unwrap();

        assert_eq!(requests.methods(), [methods::INITIALIZE, methods::GENERATE]);
        assert_eq!(
            requests.params(1),
            serde_json::json!({"target": "avro", "options": {"pretty": true}})
        );
    }

    #[tokio::test]
    async fn a_rejected_invocation_keeps_the_session_usable() {
        let (session, _requests) =
            ready_session_rejecting_then_answering(generate_result("recovered.avro")).await;
        let handle = spawn_session(session);

        let rejected = handle
            .invoke::<serde_json::Value>("morphir.backend.generate", serde_json::json!({}))
            .await;
        assert!(
            matches!(rejected, Err(DaemonError::Extension(ref message)) if message.contains("the extension refused this request")),
            "unexpected result: {rejected:?}"
        );

        let recovered: serde_json::Value = handle
            .invoke("morphir.backend.generate", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(generated_paths(&recovered), ["recovered.avro"]);
    }

    #[tokio::test]
    async fn a_caller_can_tell_a_rejection_from_a_dead_session() {
        let (rejecting, _) =
            ready_session_rejecting_then_answering(generate_result("unused.avro")).await;
        let rejecting = spawn_session(rejecting);
        let (failing, _) = ready_session_failing_transport().await;
        let failing = spawn_session(failing);

        let rejected = rejecting
            .invoke::<serde_json::Value>(methods::GENERATE, serde_json::json!({}))
            .await;
        let lost = failing
            .invoke::<serde_json::Value>(methods::GENERATE, serde_json::json!({}))
            .await;

        // A caller caching a handle evicts on one of these and retries on the
        // other, so the two must be different variants, not different strings.
        assert!(
            matches!(rejected, Err(DaemonError::Extension(_))),
            "a refused operation should not look like a lost session: {rejected:?}"
        );
        assert!(
            matches!(lost, Err(DaemonError::SessionLost(_))),
            "a dead session should be its own variant: {lost:?}"
        );
        // The cause keeps its original variant instead of being stringified.
        assert!(
            matches!(lost, Err(DaemonError::SessionLost(ref cause)) if matches!(**cause, DaemonError::Io(_))),
            "the transport failure lost its variant: {lost:?}"
        );
    }

    #[tokio::test]
    async fn a_failed_invocation_stops_the_actor() {
        let (session, _requests) = ready_session_failing_transport().await;
        let handle = spawn_session(session);

        let failed = handle
            .invoke::<serde_json::Value>("morphir.backend.generate", serde_json::json!({}))
            .await;
        assert!(
            matches!(failed, Err(DaemonError::SessionLost(_))),
            "unexpected result: {failed:?}"
        );

        let after = handle
            .invoke::<serde_json::Value>("morphir.backend.generate", serde_json::json!({}))
            .await;
        assert!(
            matches!(after, Err(DaemonError::SessionLost(_))),
            "unexpected result: {after:?}"
        );
        // An actor that merely dropped its session would still accept the
        // message and answer with the released-session cause. Undeliverable
        // means the actor itself is gone.
        assert!(
            !format!("{after:?}").contains("already released"),
            "the actor kept accepting messages: {after:?}"
        );
    }

    #[tokio::test]
    async fn shutdown_completes_the_mep_handshake() {
        let (session, requests) =
            ready_backend_session([Ok(
                ExtensionResponse::success(2, serde_json::json!({})).expect("a valid envelope")
            )])
            .await;
        let handle = spawn_session(session);

        handle.shutdown().await.unwrap();

        // Dropping the session instead of shutting it down would leave the
        // extension running and this request unsent.
        assert_eq!(requests.methods(), [methods::INITIALIZE, methods::SHUTDOWN]);
    }

    #[tokio::test]
    async fn shutdown_reports_a_failed_handshake_as_a_lost_session() {
        let (session, _requests) = ready_backend_session([Ok(ExtensionResponse::error(
            2,
            RpcError::extension_error("the extension could not stop cleanly"),
        ))])
        .await;
        let handle = spawn_session(session);

        let result = handle.shutdown().await;

        assert!(
            matches!(result, Err(DaemonError::SessionLost(_))),
            "unexpected result: {result:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_session_stops_itself() {
        let (session, _requests) = ready_session_answering(&[generate_result("out.avro")]).await;
        let handle = spawn_session_with_idle_timeout(session, std::time::Duration::from_secs(60));

        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;

        let after = handle
            .invoke::<serde_json::Value>("morphir.backend.generate", serde_json::json!({}))
            .await;
        assert!(after.is_err(), "an idle session should have stopped");
        assert!(
            matches!(after, Err(DaemonError::SessionLost(_))),
            "an idle stop should surface the same way as any other dead session: {after:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_stop_completes_the_mep_shutdown_handshake() {
        let (session, requests) =
            ready_backend_session([Ok(
                ExtensionResponse::success(2, serde_json::json!({})).expect("a valid envelope")
            )])
            .await;
        // Spawned directly (bypassing `spawn_session_with_idle_timeout`) so
        // this test can hold the concrete `ActorRef` and wait deterministically
        // for `on_stop` via `wait_for_shutdown_result`, rather than inferring
        // completion from a subsequent `invoke` failing (which only proves the
        // mailbox stopped accepting new messages, not that `on_stop` itself
        // has finished running).
        let (activity, receiver) = tokio::sync::watch::channel(());
        let actor_ref = SessionActor::spawn(SessionActor {
            session: Some(session),
            activity,
        });
        let idle = std::time::Duration::from_secs(60);
        let _watchdog = spawn_idle_watchdog(actor_ref.downgrade(), receiver, idle);

        tokio::time::advance(idle + std::time::Duration::from_secs(1)).await;
        actor_ref
            .wait_for_shutdown_result()
            .await
            .expect("on_stop should not error");

        // Dropping the session instead of shutting it down would leave the
        // extension running and this request unsent. Proving this here (and
        // not just for the explicit `Shutdown` message) is the point of
        // giving an idle-stopped actor its own `on_stop` hook.
        assert_eq!(requests.methods(), [methods::INITIALIZE, methods::SHUTDOWN]);
    }

    #[tokio::test(start_paused = true)]
    async fn activity_resets_the_idle_timer() {
        let (session, _requests) = ready_session_answering(&[
            generate_result("first.avro"),
            generate_result("second.avro"),
        ])
        .await;
        let idle = std::time::Duration::from_secs(10);
        let handle = spawn_session_with_idle_timeout(session, idle);

        // Advance less than the idle duration, then invoke: this should reset
        // the timer rather than letting the elapsed time accumulate toward it.
        tokio::time::advance(std::time::Duration::from_secs(7)).await;
        tokio::task::yield_now().await;
        let first: serde_json::Value = handle
            .invoke("morphir.backend.generate", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(generated_paths(&first), ["first.avro"]);

        // Advance less than the idle duration again. Without a reset, the two
        // advances together (7s + 7s = 14s) would exceed the 10s idle window
        // and this session would already be dead.
        tokio::time::advance(std::time::Duration::from_secs(7)).await;
        tokio::task::yield_now().await;
        let second: serde_json::Value = handle
            .invoke("morphir.backend.generate", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(generated_paths(&second), ["second.avro"]);
    }

    #[tokio::test]
    async fn explicit_shutdown_is_not_repeated_when_the_actor_stops() {
        let (session, requests) =
            ready_backend_session([Ok(
                ExtensionResponse::success(2, serde_json::json!({})).expect("a valid envelope")
            )])
            .await;
        // Spawned directly (bypassing `spawn_session`) so the test can hold
        // the concrete `ActorRef` and use `wait_for_shutdown_result`, which
        // resolves only once `on_stop` has actually returned. `SessionHandle`
        // erases the actor type on purpose, so it has no equivalent method;
        // polling with `yield_now` in a loop would only approximate this.
        let (activity, _receiver) = tokio::sync::watch::channel(());
        let actor_ref = SessionActor::spawn(SessionActor {
            session: Some(session),
            activity,
        });

        actor_ref.ask(Shutdown).await.unwrap();
        // The explicit `Shutdown` message already completed the MEP handshake
        // and took `self.session`. `on_stop` runs right after, as part of the
        // same terminal stop; waiting for it here (rather than guessing with
        // a fixed number of yields) is what makes the following assertion
        // deterministic instead of racy.
        actor_ref
            .wait_for_shutdown_result()
            .await
            .expect("on_stop should not error");

        assert_eq!(
            requests.methods(),
            [methods::INITIALIZE, methods::SHUTDOWN],
            "on_stop should not repeat the MEP shutdown handshake"
        );
    }

    #[tokio::test]
    async fn the_idle_watchdog_exits_when_the_actor_stops_for_another_reason() {
        let (session, _requests) =
            ready_backend_session([Ok(
                ExtensionResponse::success(2, serde_json::json!({})).expect("a valid envelope")
            )])
            .await;
        let (activity, receiver) = tokio::sync::watch::channel(());
        let actor_ref = SessionActor::spawn(SessionActor {
            session: Some(session),
            activity,
        });
        // An idle duration far longer than this test will take: if the
        // watchdog only exits by reaching its deadline (rather than noticing
        // the actor's `activity` sender dropped), the `timeout` below fails
        // fast instead of the test hanging for real minutes.
        let idle = std::time::Duration::from_secs(600);
        let watchdog = spawn_idle_watchdog(actor_ref.downgrade(), receiver, idle);

        actor_ref.ask(Shutdown).await.unwrap();
        actor_ref
            .wait_for_shutdown_result()
            .await
            .expect("on_stop should not error");

        tokio::time::timeout(std::time::Duration::from_secs(1), watchdog)
            .await
            .expect(
                "the watchdog should exit as soon as the actor stops, \
                 not linger until its idle deadline",
            )
            .expect("the watchdog task should not panic");
    }

    #[tokio::test]
    async fn dropping_the_last_handle_completes_the_mep_shutdown_handshake() {
        let (session, requests) =
            ready_backend_session([Ok(
                ExtensionResponse::success(2, serde_json::json!({})).expect("a valid envelope")
            )])
            .await;
        // Deliberately spawned through the production entry point: the point of
        // this test is that nothing spawned alongside the actor (the idle
        // watchdog in particular) keeps it alive once callers let go. An idle
        // duration far longer than this test can run makes the watchdog's own
        // deadline an impossible explanation for the stop.
        let handle = spawn_session_with_idle_timeout(session, std::time::Duration::from_secs(600));

        drop(handle);

        // `SessionHandle` erases the actor type, so there is no `ActorRef` left
        // to wait on once the last handle is gone; the shared request log is
        // the only observable. Polled with a sleep rather than `yield_now` so a
        // failing run waits idly instead of burning a worker thread next to the
        // rest of the suite, and bounded so that failure is a fast, clearly
        // attributed timeout rather than a hung suite.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while requests.methods().len() < 2 {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .expect(
            "dropping the last handle should stop the actor and complete the \
             MEP shutdown handshake",
        );
        assert_eq!(requests.methods(), [methods::INITIALIZE, methods::SHUTDOWN]);
    }

    #[tokio::test]
    async fn shutdown_stops_the_actor_and_later_calls_report_it() {
        let (session, _requests) =
            ready_backend_session([Ok(
                ExtensionResponse::success(2, serde_json::json!({})).expect("a valid envelope")
            )])
            .await;
        let handle = spawn_session(session);

        handle.shutdown().await.unwrap();

        let after = handle
            .invoke::<serde_json::Value>("morphir.backend.generate", serde_json::json!({}))
            .await;
        assert!(
            matches!(after, Err(DaemonError::SessionLost(_))),
            "unexpected result: {after:?}"
        );
        assert!(
            !format!("{after:?}").contains("already released"),
            "the actor kept accepting messages: {after:?}"
        );
    }
}
