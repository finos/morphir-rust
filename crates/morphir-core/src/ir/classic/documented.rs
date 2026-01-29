//! Classic IR Documented wrapper
//!
//! Documentation wrapper for the Classic Morphir IR format.

use serde::{Deserialize, Serialize};

/// Type that represents a documented value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Documented<A> {
    pub doc: String,
    pub value: A,
}

impl<A> Documented<A> {
    /// Create a new documented value.
    pub fn new(doc: impl Into<String>, value: A) -> Self {
        Self {
            doc: doc.into(),
            value,
        }
    }

    /// Map over the value inside Documented.
    pub fn map<B, F: FnOnce(A) -> B>(self, f: F) -> Documented<B> {
        Documented {
            doc: self.doc,
            value: f(self.value),
        }
    }
}
