//! Warm guest reuse, one session per key.
//!
//! See [`Pool`] for the locking, fingerprint, retry and abandon rules.

use crate::connection::{CallError, GuestConnection};
use crate::send::MaybeSend;
use crate::session::Session;
use crate::{HostConfig, HostError};
use async_lock::Mutex;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// One cached guest: the fingerprint it was opened with, and its session.
struct SlotState {
    fingerprint: String,
    session: Session,
}

/// A key's guest, once one exists.
struct Slot {
    /// Set by [`Pool::abandon`], under the map lock, when it removes this
    /// slot from the map. A call that gets the slot's lock after that looks
    /// the key up again instead of using this slot.
    abandoned: AtomicBool,
    /// `None` before the first call for the key, and while a call holds the
    /// session.
    state: Mutex<Option<SlotState>>,
}

/// Warm guests, one per key, reused across calls.
///
/// [`Pool::call`] replaces opening a fresh guest for every call with opening
/// one guest per key and reusing it, the same behavior the CLI's session
/// reuse invoker gave the playground, but portable.
///
/// # Locking
///
/// One [`async_lock::Mutex`] guards the key-to-slot map. Each slot then holds
/// its own `async_lock::Mutex` around its [`Session`], so a call for key `A`
/// never waits on a call for key `B`: the map lock is only held long enough
/// to find or insert the slot, never across a call. Holding a slot's lock
/// across a call serializes calls to the same provider, the same guarantee
/// the actor gave the CLI.
///
/// # Fingerprints
///
/// A caller names which incarnation of a provider it wants with
/// `fingerprint`. A slot whose cached fingerprint differs from the one a call
/// supplies is stale: the old session is closed (its close errors are
/// ignored, since the old guest is being discarded either way) and a new one
/// is opened with `open` before the call runs.
///
/// # Failure and retry
///
/// [`CallError::Rejected`] is the guest answering; the session stays cached.
/// [`CallError::Failed`] means the session broke, so the slot is evicted, a
/// new guest is opened, and the call is retried exactly once on it. A failure
/// of the retried call leaves the slot empty and returns
/// [`CallError::Failed`].
///
/// Because of that retry, a pooled call must be safe to send twice: when a
/// session breaks mid-call the guest may already have run the first attempt.
///
/// [`CallError::Decode`] means the guest answered but its result did not
/// decode. The session already closed itself in order, so the slot is left
/// empty and the next call opens fresh. It is not retried: the same guest
/// build would give the same bad answer.
///
/// [`CallError::Connect`] means `open` failed, so no guest was reached.
/// [`CallError::Handshake`] means `open` returned a connection but the MEP
/// handshake on it failed. Each carries that error as it was. Neither is
/// retried and nothing is cached, whether it came from the first open or
/// from opening the replacement for a broken session. Only in the first case
/// is it certain that the call never reached a guest.
///
/// # Cancellation
///
/// A session is held out of its slot for the duration of a call, not merely
/// borrowed, and is put back only once the call completes (successfully or
/// [`CallError::Rejected`]). Dropping a call's future before it completes,
/// such as a caller's own timeout, therefore drops the session with it
/// instead of leaving a cached session with a request permanently in flight:
/// the next call for that key opens fresh.
///
/// # Abandon
///
/// [`Pool::abandon`] removes the slot from the map immediately and marks it
/// abandoned. When no call holds the slot's lock, the removed session is
/// closed right away. When a call holds it, `abandon` only drops its own
/// reference to the slot: that call finishes against the session it already
/// holds (including its one retry, if the session breaks), and whatever
/// session it leaves behind is dropped, without an orderly close, once the
/// last reference to the detached slot goes. A call still waiting on the
/// slot's lock does not use the abandoned slot: once it gets the lock it
/// sees the mark, looks the key up again, and runs against the key's new
/// slot, like any new call for the key.
pub struct Pool<K> {
    config: HostConfig,
    slots: Mutex<HashMap<K, Arc<Slot>>>,
}

impl<K: Eq + Hash + Clone + MaybeSend + Sync> Pool<K> {
    /// A pool that introduces itself to every guest it opens with `config`.
    pub fn new(config: HostConfig) -> Self {
        Self {
            config,
            slots: Mutex::new(HashMap::new()),
        }
    }

    /// Call `method` on the guest for `key`.
    ///
    /// Opens the guest with `open` when there is none cached for `key`, or
    /// when `fingerprint` names a different incarnation than the cached
    /// session. See the module documentation for what happens on failure.
    pub async fn call<P, R, F, Fut>(
        &self,
        key: &K,
        fingerprint: &str,
        open: F,
        method: &str,
        params: &P,
    ) -> Result<R, CallError>
    where
        P: Serialize + Sync,
        R: DeserializeOwned,
        F: Fn() -> Fut,
        Fut: Future<Output = Result<Box<dyn GuestConnection>, HostError>>,
    {
        loop {
            let slot = self.slot_for(key).await;
            let mut guard = slot.state.lock().await;
            if slot.abandoned.load(Ordering::Acquire) {
                // `abandon` removed this slot while this call waited on its
                // lock: look the key up again.
                continue;
            }
            return self
                .call_in(&mut guard, fingerprint, &open, method, params)
                .await;
        }
    }

    /// [`Pool::call`] against one slot's state, with its lock held.
    async fn call_in<P, R, F, Fut>(
        &self,
        guard: &mut Option<SlotState>,
        fingerprint: &str,
        open: &F,
        method: &str,
        params: &P,
    ) -> Result<R, CallError>
    where
        P: Serialize + Sync,
        R: DeserializeOwned,
        F: Fn() -> Fut,
        Fut: Future<Output = Result<Box<dyn GuestConnection>, HostError>>,
    {
        Self::ensure_matching(guard, fingerprint, open, &self.config).await?;
        // Taken out of the slot, not just borrowed: if this call's own future
        // is dropped before it finishes (a caller's timeout), `state` drops
        // with it mid-call, taking its session with it, and the slot is left
        // empty rather than caching a session with a request permanently in
        // flight. It is put back only once the call actually completes.
        let mut state = guard
            .take()
            .expect("ensure_matching leaves the slot occupied");

        match call_on(&mut state.session, method, params).await {
            Ok(value) => {
                *guard = Some(state);
                Ok(value)
            }
            Err(CallError::Rejected(error)) => {
                *guard = Some(state);
                Err(CallError::Rejected(error))
            }
            Err(CallError::Decode(error)) => {
                // The session closed itself in order; the slot stays empty.
                // The same build would give the same answer, so this is not
                // retried.
                drop(state);
                Err(CallError::Decode(error))
            }
            Err(CallError::Failed(_)) => {
                // The connection already tore its own transport down on this
                // failure, so there is nothing left to close: just drop the
                // broken session. The slot is already empty since it was
                // taken above.
                drop(state);
                let mut session = open_session(open, &self.config)
                    .await
                    .map_err(CallError::from)?;
                match call_on(&mut session, method, params).await {
                    Ok(value) => {
                        *guard = Some(SlotState {
                            fingerprint: fingerprint.to_owned(),
                            session,
                        });
                        Ok(value)
                    }
                    Err(CallError::Rejected(error)) => {
                        *guard = Some(SlotState {
                            fingerprint: fingerprint.to_owned(),
                            session,
                        });
                        Err(CallError::Rejected(error))
                    }
                    // The slot is already empty: the retry's own failure
                    // leaves it evicted for the next caller to open fresh.
                    Err(error) => Err(error),
                }
            }
            // A session never reports a connect or handshake failure; pass
            // anything else through with the session left out of the slot.
            Err(error) => Err(error),
        }
    }

    /// Forget the guest for `key`.
    ///
    /// Closes the removed session right away when no call holds the slot.
    /// Otherwise only drops the slot reference: the call holding the slot
    /// finishes against the session it already holds, and that session is
    /// dropped once the last reference to the slot goes. A call waiting on
    /// the slot runs against the key's new slot instead. See [`Pool`].
    pub async fn abandon(&self, key: &K) {
        let slot = {
            let mut slots = self.slots.lock().await;
            let slot = slots.remove(key);
            if let Some(slot) = &slot {
                slot.abandoned.store(true, Ordering::Release);
            }
            slot
        };
        let Some(slot) = slot else {
            return;
        };
        if let Some(mut guard) = slot.state.try_lock()
            && let Some(state) = guard.take()
        {
            let _ = state.session.close().await;
        }
    }

    /// The slot for `key`, inserting an empty one when there is none yet.
    async fn slot_for(&self, key: &K) -> Arc<Slot> {
        let mut slots = self.slots.lock().await;
        if let Some(slot) = slots.get(key) {
            return Arc::clone(slot);
        }
        let slot = Arc::new(Slot {
            abandoned: AtomicBool::new(false),
            state: Mutex::new(None),
        });
        slots.insert(key.clone(), Arc::clone(&slot));
        slot
    }

    /// Make `*guard` hold a session opened for `fingerprint`, replacing
    /// whatever is there when it is empty or stamped with a different
    /// fingerprint. A replaced session's close errors are ignored: it is
    /// being discarded either way.
    async fn ensure_matching<F, Fut>(
        guard: &mut Option<SlotState>,
        fingerprint: &str,
        open: &F,
        config: &HostConfig,
    ) -> Result<(), CallError>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<Box<dyn GuestConnection>, HostError>>,
    {
        let matches = matches!(guard, Some(state) if state.fingerprint == fingerprint);
        if !matches {
            if let Some(state) = guard.take() {
                let _ = state.session.close().await;
            }
            let session = open_session(open, config).await.map_err(CallError::from)?;
            *guard = Some(SlotState {
                fingerprint: fingerprint.to_owned(),
                session,
            });
        }
        Ok(())
    }
}

/// Invoke `method` on `session`, forwarding to [`Session::call`].
///
/// A free function so it borrows only the session, not a whole [`Pool`].
async fn call_on<P, R>(session: &mut Session, method: &str, params: &P) -> Result<R, CallError>
where
    P: Serialize + Sync,
    R: DeserializeOwned,
{
    session.call(method, params).await
}

/// Which step of opening a pooled guest failed.
enum OpenFailure {
    /// `open` failed: no guest was reached.
    Connect(HostError),
    /// The guest started but the MEP handshake failed.
    Handshake(HostError),
}

impl From<OpenFailure> for CallError {
    fn from(failure: OpenFailure) -> Self {
        match failure {
            OpenFailure::Connect(error) => CallError::Connect(error),
            OpenFailure::Handshake(error) => CallError::Handshake(error),
        }
    }
}

/// Run `open`, then negotiate a [`Session`] over the connection it returns.
async fn open_session<F, Fut>(open: &F, config: &HostConfig) -> Result<Session, OpenFailure>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Box<dyn GuestConnection>, HostError>>,
{
    let connection = open().await.map_err(OpenFailure::Connect)?;
    Session::open(connection, config)
        .await
        .map_err(OpenFailure::Handshake)
}
