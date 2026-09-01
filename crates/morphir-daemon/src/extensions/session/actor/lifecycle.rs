//! The actor that owns a session, and what it does with each message.

use kameo::Actor;
use kameo::actor::{ActorRef, WeakActorRef};
use kameo::error::{ActorStopReason, Infallible};
use kameo::message::{Context, Message};

use super::gone;
use super::messages::{Invoke, Shutdown};
use super::watchdog::SessionActivity;
use crate::DaemonError;
use crate::extensions::session::{InvokeOutcome, MepTransport, Ready, Session};

/// An actor owning one ready MEP session.
///
/// The session is held as an `Option` only because [`Session::invoke`] takes it
/// by value. That `Option` is private to a single-threaded actor rather than
/// shared state, so `None` means exactly one thing: the session is gone and the
/// actor is on its way out.
///
/// `activity` implements the idle timeout: it publishes
/// [`SessionActivity::Busy`] for the whole of every handled [`Invoke`] and
/// [`SessionActivity::Idle`] once it is answered. The watchdog task spawned
/// alongside the actor (see
/// [`spawn_session_with_idle_timeout`](super::spawn_session_with_idle_timeout))
/// resets its
/// sleep on every change and refuses to stop a busy actor, so the actor is
/// stopped only after a full idle period during which it did no work at all.
pub(super) struct SessionActor<T: MepTransport + Send + 'static> {
    pub(super) session: Option<Session<T, Ready>>,
    pub(super) activity: tokio::sync::watch::Sender<SessionActivity>,
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

impl<T: MepTransport + Send + 'static> SessionActor<T> {
    /// Invoke one operation, leaving the activity state to the caller.
    async fn invoke(
        &mut self,
        msg: Invoke,
        ctx: &mut Context<Self, Result<serde_json::Value, DaemonError>>,
    ) -> Result<serde_json::Value, DaemonError> {
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

impl<T: MepTransport + Send + 'static> Message<Invoke> for SessionActor<T> {
    type Reply = Result<serde_json::Value, DaemonError>;

    async fn handle(&mut self, msg: Invoke, ctx: &mut Context<Self, Self::Reply>) -> Self::Reply {
        // Busy for the whole handler, not just its first instant. The idle
        // deadline is meant to reclaim a session nobody is using; a transport
        // operation that runs longer than the deadline is the session being
        // used, and the fresh idle period starts when it finishes.
        let _ = self.activity.send(SessionActivity::Busy);
        let reply = self.invoke(msg, ctx).await;
        let _ = self.activity.send(SessionActivity::Idle);
        reply
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
