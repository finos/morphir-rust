//! Cursor-aware visitors for the concrete Classic and v4 IR models.

pub mod classic;
pub mod cursor;
pub mod v4;

pub use cursor::{CursorSegment, IrCursor};
