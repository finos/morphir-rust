use serde::Serialize;

/// Byte offsets into the source text, end exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}
