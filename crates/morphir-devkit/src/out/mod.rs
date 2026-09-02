//! Mill-style out directory: one workspace-level root, one scratch directory
//! and one result record per task.
//!
//! ```
//! use morphir_devkit::{TaskId, TaskPaths};
//! use std::path::Path;
//!
//! let paths = TaskPaths::new(
//!     Path::new("/ws/.morphir/out"),
//!     Path::new("packages/orders"),
//!     &TaskId::generate("scala"),
//! );
//! assert!(paths.dest.ends_with("packages/orders/generate/scala.dest"));
//! assert!(paths.result.ends_with("packages/orders/generate/scala.json"));
//! ```

pub mod paths;
pub mod result;
pub mod root;

pub use paths::{OutError, TaskId, TaskPaths, sanitize_segment};
pub use result::{IrDescriptor, IrLayout, RESULT_SCHEMA, TaskResult, now_rfc3339};
pub use root::{DEFAULT_OUT_DIR, module_path, resolve_out_root};
