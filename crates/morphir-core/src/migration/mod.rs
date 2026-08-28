//! Typed migration between concrete Morphir IR versions.

mod diagnostic;

pub use diagnostic::{
    MigrationDiagnostic, MigrationOptions, MigrationReport, Severity, V4Encoding,
};
