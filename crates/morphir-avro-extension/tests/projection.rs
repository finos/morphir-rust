mod support;

use morphir_avro_extension::{
    Aliases, AvroOptions, AvroType, AvroUnion, Constructor, Dependencies, DistributionKind,
    EntryPointKind, EntryPointMetadata, IncompletenessKind, NamedSchema, Projection,
    ProjectionDependency, ProjectionModule, TypeDeclaration, TypeExpr, TypeMapping, UnionError,
    Unsupported, ValueKind, ValueSpecification, escape_idl_identifier, project, render_idl,
    render_json,
};
use pretty_assertions::assert_eq;
use serde_json::json;
use support::projection::{alias, customer_record, field, package, reference, value_specification};

include!("projection/protocols.rs");
include!("projection/dependencies.rs");
include!("projection/type_mappings.rs");
include!("projection/naming.rs");
include!("projection/advanced_types.rs");
