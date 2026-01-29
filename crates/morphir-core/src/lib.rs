// TODO: These modules have pre-existing issues with undefined types
// They need to be updated to use the refactored classic IR types
// pub mod converter;
// pub mod traversal;

pub mod error;
pub mod ir;
pub mod naming;

pub use naming::{intern, resolve, Word};

// Re-export commonly used items for convenience
// pub mod visitor {
//     pub use crate::traversal::*;
// }

