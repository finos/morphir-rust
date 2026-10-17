//! Rich MorphirModel messages and validated transport boundaries.
//!
//! The wire contract is governed by finos/morphir's canonical MEP model.
//! IR 4.0.0 remains draft; its spelling is independent of the MEP draft.

mod wire;
pub use wire::*;

mod negotiation;
pub use negotiation::*;

mod admission;
#[cfg(not(target_arch = "wasm32"))]
mod ion;
mod json;
mod operation;
mod wire_budget;
pub use admission::{ModelAdmissionError, RetainedModel, retain_model};
pub use operation::{validate_model_invocation, validate_model_result};

pub(crate) mod guest;
