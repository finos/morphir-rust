use std::collections::{BTreeMap, BTreeSet};

use morphir_extension_sdk::DiagnosticSeverity;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{
    AvroField, AvroFullName, AvroMessage, AvroPackage, AvroRequest, AvroRoot, AvroType, AvroUnion,
    EnumSchema, NamedSchema, Properties, Protocol, RecordSchema,
};
use crate::{
    Aliases, AvroDiagnostic, AvroGenerationError, AvroInternalError, AvroOptions, Constructor,
    Dependencies, DistributionKind, EntryPointKind, NamedType, ProjectedDiagnostic, Projection,
    ProjectionModule, ProjectionPackage, TypeDeclaration, TypeExpr, Unsupported, ValueKind,
    ValueSpecification,
    naming::{NameRegistry, full_name_from_source, lower_camel, namespace, upper_camel},
};

const SDK_BOOL: &str = "morphir/SDK:basics#bool";
const SDK_INT: &str = "morphir/SDK:basics#int";
const SDK_FLOAT: &str = "morphir/SDK:basics#float";
const SDK_STRING: &str = "morphir/SDK:string#string";
const SDK_CHAR: &str = "morphir/SDK:char#char";
const SDK_MAYBE: &str = "morphir/SDK:maybe#maybe";
const SDK_LIST: &str = "morphir/SDK:list#list";
const SDK_SET: &str = "morphir/SDK:set#set";
const SDK_DICT: &str = "morphir/SDK:dict#dict";
const SDK_RESULT: &str = "morphir/SDK:result#result";
const SDK_LOCAL_DATE: &str = "morphir/SDK:local-date#local-date";
const SDK_LOCAL_TIME: &str = "morphir/SDK:local-time#local-time";
const SDK_INSTANT: &str = "morphir/SDK:instant#instant";
const SDK_DATE_TIME: &str = "morphir/SDK:date-time#date-time";
const SDK_UUID: &str = "morphir/SDK:uuid#uuid";
const SDK_DECIMAL: &str = "morphir/SDK:decimal#decimal";

/// Project a normalized, body-free Morphir package into the semantic Avro model.
///
/// The returned roots retain every supported public type, including aliases
/// whose Avro representation is not itself a named schema.
///
/// # Examples
///
/// ```
/// use morphir_avro_extension::{
///     AvroOptions, DistributionKind, ProjectionPackage, project,
/// };
///
/// let package = ProjectionPackage {
///     kind: DistributionKind::Library,
///     package_name: "example".to_owned(),
///     dependencies: Vec::new(),
///     modules: Vec::new(),
/// };
/// let avro = project(&package, &AvroOptions::default())?;
/// assert!(avro.roots().is_empty());
/// # Ok::<(), morphir_avro_extension::AvroGenerationError>(())
/// ```
pub fn project(
    package: &ProjectionPackage,
    options: &AvroOptions,
) -> Result<AvroPackage, AvroGenerationError> {
    options.validate().map_err(AvroGenerationError::from)?;
    validate_physical_mappings(options).map_err(AvroGenerationError::from)?;
    let mut projector = Projector::new(options);
    let mut diagnostics = projector.register_package(package);
    diagnostics.extend(projector.project_modules(&package.package_name, &package.modules));
    diagnostics.extend(projector.project_protocols(
        package.kind,
        &package.package_name,
        &package.modules,
    ));
    if let Some(error) = projector.internal_failure.take() {
        return Err(error.into());
    }
    sort_diagnostics(&mut diagnostics);
    if options.unsupported == Unsupported::Error && !diagnostics.is_empty() {
        return Err(diagnostics.into());
    }
    let diagnostics = diagnostics
        .into_iter()
        .map(|diagnostic| ProjectedDiagnostic::new(diagnostic, DiagnosticSeverity::Warning))
        .collect();
    projector.finish(diagnostics).map_err(Into::into)
}

#[derive(Clone)]
struct Projector<'options> {
    options: &'options AvroOptions,
    registry: NameRegistry,
    roots: BTreeMap<String, AvroRoot>,
    schemas: BTreeMap<String, NamedSchema>,
    linked_schemas: BTreeMap<String, NamedSchema>,
    protocols: BTreeMap<String, Protocol>,
    declarations: BTreeMap<String, DeclarationInfo>,
    invalid_declarations: BTreeSet<String>,
    active_declarations: BTreeMap<String, Vec<ActiveSpecialization>>,
    building_schemas: BTreeSet<String>,
    internal_failure: Option<AvroInternalError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchemaOwnership {
    Owned,
    Linked,
}

#[derive(Clone)]
struct DeclarationInfo {
    declaration: TypeDeclaration,
    full_name: AvroFullName,
    dependency: bool,
}

#[derive(Clone)]
struct ActiveSpecialization {
    arguments: Vec<TypeExpr>,
    complexity: usize,
}

struct DeclarationCandidate {
    declaration: TypeDeclaration,
    full_name: AvroFullName,
    dependency: bool,
}

fn declaration_candidates(
    package_name: &str,
    modules: &[ProjectionModule],
    dependency: bool,
) -> (Vec<DeclarationCandidate>, Vec<AvroDiagnostic>) {
    let mut candidates = Vec::new();
    let mut diagnostics = Vec::new();
    for module in modules {
        for declaration in &module.types {
            match AvroFullName::new(
                namespace(package_name, &module.path),
                upper_camel(declaration.name()),
            ) {
                Ok(full_name) => candidates.push(DeclarationCandidate {
                    declaration: declaration.clone(),
                    full_name,
                    dependency,
                }),
                Err(error) => diagnostics.push(error.with_source(declaration.source_name())),
            }
        }
    }
    (candidates, diagnostics)
}

mod declarations;
mod encoding;
mod protocol;
mod registration;
mod types;

use self::{registration::sort_diagnostics, types::validate_physical_mappings};
