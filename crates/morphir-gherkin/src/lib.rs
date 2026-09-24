//! A Gherkin document model for Morphir.
//!
//! It reads `.feature` files and `.feature.md` files (Markdown with Gherkin) into one model. Every
//! node has a source span. Descriptions keep their prose and fenced blocks as parsed Markdown.
//! Extensions read tags, fences and prose into a typed context. This crate does not run scenarios.

pub mod error;
pub mod markdown;
pub mod model;
pub mod path;
pub mod span;

pub use error::ReadError;
pub use model::*;
pub use path::{NodePath, NodePathError, Segment};
pub use span::{LineCol, SourceText, Span};
