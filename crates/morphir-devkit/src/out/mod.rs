//! Mill-style out directory: one workspace-level root, one scratch directory
//! and one result record per task.

pub mod paths;
pub mod result;
pub mod root;

pub use paths::{OutError, TaskId, TaskPaths, sanitize_segment};
pub use result::{IrDescriptor, IrLayout, RESULT_SCHEMA, TaskResult, now_rfc3339};
pub use root::{DEFAULT_OUT_DIR, module_path, resolve_out_root};
