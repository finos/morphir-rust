//! Typed migration between concrete Morphir IR versions.

mod classic_to_v4;
mod diagnostic;

pub use classic_to_v4::{
    Migrated, MigrationContext, ValueAnnotation, migrate_access, migrate_distribution,
    migrate_fqname, migrate_literal, migrate_module_definition, migrate_name,
    migrate_package_specification, migrate_path, migrate_pattern, migrate_type,
    migrate_type_specification, migrate_value, migrate_value_definition,
};
pub use diagnostic::{
    MigrationDiagnostic, MigrationOptions, MigrationReport, Severity, V4Encoding,
};
