//! The v4 reader.
//!
//! A v4 document is read natively; nothing here migrates it into a classic one
//! first. Names arrive canonical (`local-date`, `SDK`) and are rebuilt in the
//! one spelling a Morphir document can express (see [`crate::names`]).
//!
//! v4 can hold a declaration Elm has no form for — an `IncompleteTypeDefinition`
//! is a type still being written. It is decoded as an opaque declaration, which
//! is what `PackageDefinition::to_specification` makes of it too, and named in
//! [`super::Unsupported`] so that a caller writing source reports it rather than
//! inventing a body for it.

use morphir_core::ir::v4::{
    Access as VAccess, AccessControlled, Distribution, Field as V4Field, IRFile, ModuleDefinition,
    PackageDefinition, Type, TypeDefinition,
};
use morphir_core::naming::{ModuleName, Name, Path};
use serde_json::Value;

use super::{Decoded, DecodedModule, Unsupported};
use crate::names;
use crate::resolved::{
    Access, FqName, RConstructor, RField, RType, ResolvedBody, ResolvedModule, ResolvedType,
};

/// A whole v4 distribution.
pub fn decode(ir: &Value) -> Result<Decoded, String> {
    let file: IRFile = serde_json::from_value(ir.clone())
        .map_err(|error| format!("not a v4 document: {error}"))?;
    let (package, definition): (Vec<String>, PackageDefinition) = match file.distribution {
        Distribution::Library(library) => (path(library.package_name.as_path()), library.def),
        Distribution::Application(application) => {
            (path(application.package_name.as_path()), application.def)
        }
        Distribution::Specs(_) => {
            return Err(
                "a v4 `Specs` distribution holds specifications, not the module definitions \
                 generation needs"
                    .to_string(),
            );
        }
    };

    let mut modules = Vec::with_capacity(definition.modules.len());
    let mut omitted_values = 0;
    let mut unsupported = Vec::new();
    for (key, entry) in &definition.modules {
        let name = module_path(key)?;
        let decoded = module(&name, entry);
        omitted_values += decoded.omitted_values;
        unsupported.extend(decoded.unsupported);
        modules.push(decoded.module);
    }

    Ok(Decoded {
        package,
        modules,
        omitted_values,
        unsupported,
    })
}

/// One access-controlled module definition, as
/// [`crate::frontend::emit::Emitter::emit_module`] wrote it.
pub fn module_definition(name: &[String], ir: &Value) -> Result<DecodedModule, String> {
    let definition: AccessControlled<ModuleDefinition> = serde_json::from_value(ir.clone())
        .map_err(|error| format!("not a v4 module definition: {error}"))?;
    Ok(module(&spelled_path(name), &definition))
}

fn module(name: &[String], definition: &AccessControlled<ModuleDefinition>) -> DecodedModule {
    let mut unsupported = Vec::new();
    let types = definition
        .value
        .types
        .iter()
        .map(|(key, declaration)| declaration_of(name, key, declaration, &mut unsupported))
        .collect();

    DecodedModule {
        module: ResolvedModule {
            name: name.to_vec(),
            access: access(definition.access),
            doc: definition
                .value
                .doc
                .as_ref()
                .map(|doc| doc.text().to_string()),
            types,
            depends_on: Vec::new(),
            skipped_values: Vec::new(),
        },
        omitted_values: definition.value.values.len(),
        unsupported,
    }
}

type V4Declaration = AccessControlled<morphir_core::ir::v4::Documented<TypeDefinition>>;

fn declaration_of(
    module: &[String],
    key: &str,
    declaration: &V4Declaration,
    unsupported: &mut Vec<Unsupported>,
) -> ResolvedType {
    let name = names::type_spelling_of(&name_words(key));
    let (params, body) = match &declaration.value.value {
        TypeDefinition::TypeAliasDefinition {
            type_params,
            type_expr,
        } => (type_params.clone(), ResolvedBody::Alias(ty(type_expr))),
        TypeDefinition::CustomTypeDefinition {
            type_params,
            constructors,
        } => (
            type_params.clone(),
            ResolvedBody::Custom {
                constructor_access: access(constructors.access),
                constructors: constructors
                    .value
                    .iter()
                    .map(|constructor| RConstructor {
                        name: names::type_spelling_of(&constructor.name.words()),
                        args: constructor
                            .args
                            .iter()
                            .map(|argument| ty(&argument.arg_type))
                            .collect(),
                    })
                    .collect(),
            },
        ),
        // A type this version can hold but Elm cannot write publishes no shape,
        // so it reads back as opaque — and is reported, so that a caller
        // writing Elm leaves it out instead of writing a body it does not have.
        TypeDefinition::IncompleteTypeDefinition { type_params, .. } => {
            unsupported.push(Unsupported {
                construct: "IncompleteTypeDefinition",
                module: module.to_vec(),
                name: name.clone(),
            });
            (
                type_params.clone(),
                ResolvedBody::Custom {
                    constructor_access: Access::Private,
                    constructors: Vec::new(),
                },
            )
        }
    };

    ResolvedType {
        name,
        access: access(declaration.access),
        doc: declaration
            .value
            .doc
            .as_ref()
            .map(|doc| doc.text().to_string()),
        params: params
            .iter()
            .map(|param| names::value_spelling_of(&param.words()))
            .collect(),
        body,
    }
}

/// A v4 type expression, with every identifier in its document spelling.
pub fn ty(value: &Type) -> RType {
    match value {
        Type::Variable(_, variable) => RType::Var(names::value_spelling_of(&variable.words())),
        Type::Reference(_, reference, arguments) => RType::Ref(
            FqName {
                package: path(&reference.package_path),
                module: path(&reference.module_path),
                name: names::type_spelling_of(&reference.local_name.words()),
            },
            arguments.iter().map(ty).collect(),
        ),
        Type::Record(_, fields) => RType::Record(fields.iter().map(field).collect()),
        Type::ExtensibleRecord(_, variable, fields) => RType::ExtensibleRecord(
            names::value_spelling_of(&variable.words()),
            fields.iter().map(field).collect(),
        ),
        Type::Tuple(_, elements) => RType::Tuple(elements.iter().map(ty).collect()),
        Type::Function(_, argument, result) => {
            RType::Function(Box::new(ty(argument)), Box::new(ty(result)))
        }
        Type::Unit(_) => RType::Unit,
    }
}

fn field(source: &V4Field) -> RField {
    RField {
        name: names::value_spelling_of(&source.name.words()),
        ty: ty(&source.tpe),
    }
}

fn access(access: VAccess) -> Access {
    match access {
        VAccess::Public => Access::Public,
        VAccess::Private => Access::Private,
    }
}

/// The document-spelled segments of a v4 path.
pub fn path(path: &Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| names::type_spelling_of(&segment.words()))
        .collect()
}

/// The document-spelled path of a canonical v4 module name.
pub fn module_path(canonical: &str) -> Result<Vec<String>, String> {
    let name = ModuleName::from_canonical_string(canonical)
        .map_err(|error| format!("`{canonical}` is not a canonical module name: {error}"))?;
    Ok(path(name.as_path()))
}

/// The document spelling of a path a caller stated.
fn spelled_path(segments: &[String]) -> Vec<String> {
    segments
        .iter()
        .map(|segment| names::type_spelling(segment))
        .collect()
}

/// A canonical v4 name's words, falling back to the key itself when the key is
/// not one this crate would have written.
fn name_words(canonical: &str) -> Vec<String> {
    match Name::from_canonical_string(canonical) {
        Ok(name) => name.words(),
        Err(_) => vec![canonical.to_string()],
    }
}
