//! Where a node came from in the source text.

use serde::Serialize;

/// Byte offsets into the source text, end exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Span {
    /// The first byte of the node.
    pub start: usize,
    /// One past the last byte of the node.
    pub end: usize,
}
