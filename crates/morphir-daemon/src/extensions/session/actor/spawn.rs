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
