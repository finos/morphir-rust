pub mod error;
pub mod ir;
pub mod naming;
pub mod traversal;
pub mod converter;

// Re-export commonly used traversal items for convenience
pub mod visitor {
    pub use crate::traversal::*;
}
