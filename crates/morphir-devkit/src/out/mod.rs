//! Mill-style out directory: one workspace-level root, one scratch directory
//! and one result record per task.

pub mod paths;
pub mod result;

pub use paths::{OutError, TaskId, TaskPaths, sanitize_segment};
pub use result::{IrDescriptor, IrLayout, RESULT_SCHEMA, TaskResult, now_rfc3339};
