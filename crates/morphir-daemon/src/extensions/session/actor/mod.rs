//! One Kameo actor per MEP session, owning the session across invocations.
//!
//! A [`Session`](super::Session) is a typestate value:
//! [`Session::invoke`](super::Session::invoke) consumes it and hands it back
//! inside an [`InvokeOutcome`](super::InvokeOutcome). Sharing one behind a lock
//! would mean taking it out and putting it back on every call, with an empty
//! slot as the cost of every missed failure path. An actor instead owns the
//! session as its own state and rebinds it after each message, so the typestate
//! is respected and no lock is involved.
//!
//! Kameo does not escape this module. [`SessionHandle`] erases both the
//! transport type and the actor framework, so callers hold one handle type and
//! never depend on `kameo`.
//!
//! [`handle`] is the only part of this a caller sees; [`lifecycle`],
//! [`messages`] and [`watchdog`] are the actor, the two messages it answers,
//! and the task that reclaims it when it goes idle.

mod handle;
mod lifecycle;
mod messages;
mod spawn;
mod watchdog;

#[cfg(test)]
mod tests;

pub use handle::SessionHandle;
pub use spawn::{spawn_session, spawn_session_with_idle_timeout};

use crate::DaemonError;

/// Report that the session is gone, wrapping the cause that ended it.
///
/// [`DaemonError::SessionLost`] is deliberately a different variant from the
/// [`DaemonError::Extension`] an extension returns when it refuses one
/// operation. A caller caching a handle needs to evict and respawn on the first
/// but not the second, and matching on message text is not a contract.
fn gone(cause: DaemonError) -> DaemonError {
    DaemonError::SessionLost(Box::new(cause))
}
