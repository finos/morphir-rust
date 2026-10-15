//! Explicit consumer projections from freshly restored native Library bytes.
//! The pure projection is separate from the explicit one-shot service boundary.

mod compatibility;
mod projection;
pub use projection::{CORE_PROJECTION_CODEC, LibraryProjection, ProjectionError, project_library};

/// Versioned, bounded binary Ion requests around fresh shared restoration.
pub mod service;
