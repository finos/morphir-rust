//! Native child-process plumbing shared by process-backed MEP channels.
//!
//! This module holds the parts of a process-backed channel that do not
//! depend on session or transport state: the launch description, wire
//! framing over standard streams, and staging a verified executable to disk
//! before it runs.

mod frame;
mod launch;
mod stage;

pub use frame::{MAX_MEP_PAYLOAD_BYTES, read_frame, write_frame};
pub use launch::{ProcessLaunch, ProcessProgram};
pub use stage::prepare_program;
