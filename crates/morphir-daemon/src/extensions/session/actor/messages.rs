//! The two messages a session actor answers.
//!
//! Kept apart from the actor itself because both the actor (which handles them)
//! and the erased dispatch surface (which sends them) refer to these types, and
//! neither should have to reach through the other to name one.

/// Invoke one MEP operation on the owned session.
pub(super) struct Invoke {
    pub(super) method: String,
    pub(super) params: serde_json::Value,
}

/// Complete MEP shutdown and stop the actor.
pub(super) struct Shutdown;
