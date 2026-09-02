//! Mill-style out directory: one workspace-level root, one scratch directory
//! and one result record per task.

pub mod paths;

pub use paths::{OutError, TaskId, TaskPaths, sanitize_segment};
