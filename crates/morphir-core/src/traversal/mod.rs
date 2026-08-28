//! Cursor-aware visitors for the concrete Classic and v4 IR models.

pub mod classic;
pub mod cursor;
pub mod event;
pub mod v4;

pub use cursor::{CursorSegment, IrCursor};
pub use event::{
    ClassicV3Module, DependencyEvent, DistributionHeader, ModuleEvent, SemanticEvent,
    SemanticEventKind,
};
