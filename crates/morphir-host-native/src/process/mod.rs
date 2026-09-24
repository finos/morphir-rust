//! Native child processes as MEP channels.
//!
//! This module holds the launch description, wire framing over standard
//! streams, staging a verified executable to disk before it runs, the
//! running child with its streams and diagnostics, and the channel that
//! carries MEP messages over it.

mod channel;
mod child;
mod frame;
mod launch;
mod stage;

pub use channel::ProcessChannel;
pub use child::ProcessChild;
pub use frame::{MAX_MEP_PAYLOAD_BYTES, read_frame, write_frame};
pub use launch::{ProcessLaunch, ProcessProgram};
pub use stage::prepare_program;
