//! Shared normalization from Morphir IR into a body-free projection model.
//!
//! Backend extensions decode IR v3 or v4 with [`normalize()`] and then project
//! the resulting [`ProjectionPackage`] into their own target model. The model
//! keeps public declarations, documentation, source FQNames, dependencies, and
//! v4 entry-point metadata, and drops every value body.

mod model;
mod normalize;

pub use model::{
    Constructor, DistributionKind, EntryPointKind, EntryPointMetadata, IncompletenessKind,
    NamedType, ProjectionDependency, ProjectionModule, ProjectionPackage, TypeDeclaration,
    TypeExpr, ValueKind, ValueSpecification,
};
pub use normalize::{NormalizeError, normalize};

#[cfg(any(test, feature = "testing"))]
pub mod testing;
