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

/// One cached guest: the fingerprint it was opened with, and its session.
struct SlotState {
    fingerprint: String,
    session: Session,
}

/// A key's guest, once one exists. `None` before the first call for the key.
type Slot = Mutex<Option<SlotState>>;

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
/// is opened with `open` before the call runs. An open failure is reported as
/// [`CallError::Failed`] without caching anything.
///
/// # Failure and retry
///
/// [`CallError::Rejected`] is the guest answering; the session stays cached.
/// [`CallError::Failed`] means the session broke, so the slot is evicted, a
/// new guest is opened, and the call is retried exactly once on it. A second
/// failure, from the retry's call or from opening its replacement, evicts the
/// slot again (leaving it empty) and returns [`CallError::Failed`].
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
/// [`Pool::abandon`] removes the slot from the map immediately. When no call
/// is using that slot, the removed session is closed right away. When a call
/// is in flight, `abandon` only drops its own reference to the slot: the call
/// in flight finishes against the session it already holds, and the session
/// is dropped once that call releases the last reference. A caller already
/// holding the removed `Arc` (in flight, or already waiting on its lock) thus
/// still finishes against the abandoned session, while any new call for the
/// same key finds no entry and opens a fresh guest.
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
        let slot = self.slot_for(key).await;
        let mut guard = slot.lock().await;

        Self::ensure_matching(&mut guard, fingerprint, &open, &self.config).await?;
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
            Err(CallError::Failed(_)) => {
                // The connection already tore its own transport down on this
                // failure, so there is nothing left to close: just drop the
                // broken session. The slot is already empty since it was
                // taken above.
                drop(state);
                let mut session = open_session(&open, &self.config)
                    .await
                    .map_err(CallError::Failed)?;
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
                    Err(CallError::Failed(error)) => Err(CallError::Failed(error)),
                }
            }
        }
    }

    /// Forget the guest for `key`.
    ///
    /// Closes the removed session right away when no call is using it.
    /// Otherwise only drops the slot reference: the call in flight finishes
    /// against the session it already holds, and the session is dropped once
    /// that call releases the last reference to the slot.
    pub async fn abandon(&self, key: &K) {
        let slot = {
            let mut slots = self.slots.lock().await;
            slots.remove(key)
        };
        let Some(slot) = slot else {
            return;
        };
        if let Some(mut guard) = slot.try_lock()
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
        let slot = Arc::new(Mutex::new(None));
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
            let session = open_session(open, config)
                .await
                .map_err(CallError::Failed)?;
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

/// Run `open`, then negotiate a [`Session`] over the connection it returns.
async fn open_session<F, Fut>(open: &F, config: &HostConfig) -> Result<Session, HostError>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Box<dyn GuestConnection>, HostError>>,
{
    let connection = open().await?;
    Session::open(connection, config).await
}
