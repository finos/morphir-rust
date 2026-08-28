//! Knowledgebase operations over OKF bundles.
//!
//! This crate ports the morphir-scala `kb` CLI's operational layer to Rust, on top of
//! the format model in `morphir-okf`. It provides the check catalogue, scaffolding,
//! the SQLite index, upstream sync vendoring, the intent and decision registers, and
//! the derived-state refresh, plus the text/JSON rendering shared by the CLI.
//!
//! The reference implementation lives in
//! `morphir-scala/.claude/skills/kb/` (`KbCheck.scala`, `KbScaffold.scala`,
//! `KbIndex.scala`, `KbSync.scala`, `KbIntent.scala`, `KbIntentEdit.scala`,
//! `KbDecision.scala`, `KbRefresh.scala`, `KbRender.scala`).

pub mod check;
pub mod decision;
pub mod error;
pub mod index;
pub mod intent;
pub mod refresh;
pub mod render;
pub mod scaffold;
pub mod sync;
pub mod util;

pub use error::{Error, Result};
