//! Typed migration between concrete Morphir IR versions.

mod classic_to_v4;
mod diagnostic;

pub use classic_to_v4::{
    MigrationContext, migrate_definition, migrate_literal, migrate_pattern, migrate_type,
    migrate_value, migrate_value_definition,
};
pub use diagnostic::{
    MigrationDiagnostic, MigrationOptions, MigrationReport, Severity, V4Encoding,
};
