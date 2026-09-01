//! The task that reclaims a session once it stops being used.

use kameo::actor::WeakActorRef;

use super::lifecycle::SessionActor;
use crate::extensions::session::MepTransport;

/// What the session is doing, as far as the idle watchdog can see.
///
/// "Idle" is a claim about the session, not about the clock, so the watchdog
/// needs more than a timestamp: a session can be well past its deadline and
/// still be working. Publishing the state itself lets the watchdog answer
/// "is this session idle?" instead of "has it been a while since one started?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionActivity {
    /// No invocation is in flight; the idle period runs from the last change.
    Idle,
    /// An invocation is being handled and the session cannot be idle-stopped.
    Busy,
}

/// Spawn the task that stops `actor_ref` after `idle` passes with the session
/// reported [`SessionActivity::Idle`] on `receiver` throughout.
///
/// The watchdog reads the published state rather than only reacting to changes,
/// because a deadline can pass in the middle of a single long invocation: with
/// no state to consult it would stop a session that had been working without
/// pause since the moment it was created.
///
/// The reference is deliberately weak. Kameo stops an actor when its last
/// strong reference drops, so a watchdog holding a strong one would keep the
/// session — and its extension subprocess — running for a full idle period
/// after the last [`SessionHandle`](super::SessionHandle) was dropped. Weak
/// also means the watchdog can outlive the actor, which is why every use of it
/// upgrades first.
///
/// Returns the watchdog's own `JoinHandle` so tests can observe when it
/// exits; production callers have no need for it; the watchdog either fires
/// (stopping the actor), finds the actor already gone, or notices `receiver`
/// closed (the actor already stopped some other way), and exits on its own in
/// every case.
pub(super) fn spawn_idle_watchdog<T: MepTransport + Send + 'static>(
    actor_ref: WeakActorRef<SessionActor<T>>,
    mut receiver: tokio::sync::watch::Receiver<SessionActivity>,
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
                    if *receiver.borrow_and_update() == SessionActivity::Busy {
                        // The deadline passed while an invocation was still
                        // running. A working session is not an idle one, and a
                        // graceful stop here would answer this invocation and
                        // then refuse every call after it -- the caller would
                        // lose a session that was never once idle. Wait for
                        // the invocation to finish and start a fresh idle
                        // period from there.
                        if receiver.changed().await.is_err() {
                            return;
                        }
                        sleep.as_mut().reset(tokio::time::Instant::now() + idle);
                        continue;
                    }
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
