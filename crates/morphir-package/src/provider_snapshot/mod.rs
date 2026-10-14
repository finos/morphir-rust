//! Explicit consumer projections from freshly restored native Library bytes.
//! Process framing and service composition are separate from this pure step.

mod compatibility;
mod projection;
pub use projection::{CORE_PROJECTION_CODEC, LibraryProjection, ProjectionError, project_library};
