//! The classic (v3) reader.
//!
//! Classic names are word lists, so every identifier is rebuilt in the one
//! spelling a Morphir document can express (see [`crate::names`]). Classic holds
//! only aliases and custom types, both of which Elm can write, so this reader
//! never reports an unsupported construct.

use morphir_core::ir::classic::{
    Access as CAccess, AccessControlled, Attrs, Distribution, DistributionBody, ModuleDefinition,
    Name, Path, Type, TypeDefinition,
};
use morphir_core::naming::resolve;
use serde_json::Value;

use super::{Decoded, DecodedModule};
use crate::names;
use crate::resolved::{
    Access, FqName, RConstructor, RField, RType, ResolvedBody, ResolvedModule, ResolvedType,
};

/// The classic module definition this binding writes and reads.
pub type ClassicModule = ModuleDefinition<Attrs, Type<Attrs>>;

/// A whole classic distribution.
pub fn decode(ir: &Value) -> Result<Decoded, String> {
    let distribution: Distribution = serde_json::from_value(ir.clone())
        .map_err(|error| format!("not a v3 distribution: {error}"))?;
    let DistributionBody::Library(package_path, _dependencies, definition) =
        distribution.distribution;

    let package = path(&package_path);
    let mut modules = Vec::with_capacity(definition.modules.len());
    let mut omitted_values = 0;
    for entry in &definition.modules {
        let decoded = module(&path(&entry.path), &entry.definition);
        omitted_values += decoded.omitted_values;
        modules.push(decoded.module);
    }

    Ok(Decoded {
        package,
        modules,
        omitted_values,
        unsupported: Vec::new(),
    })
}

/// One access-controlled module definition, as
/// [`crate::frontend::emit::Emitter::emit_module`] wrote it. A module definition
/// carries no name of its own — the name is the key its package filed it under
/// — so the caller states it.
pub fn module_definition(name: &[String], ir: &Value) -> Result<DecodedModule, String> {
    let definition: AccessControlled<ClassicModule> = serde_json::from_value(ir.clone())
        .map_err(|error| format!("not a v3 module definition: {error}"))?;
    Ok(module(&spelled_path(name), &definition))
}

fn module(name: &[String], definition: &AccessControlled<ClassicModule>) -> DecodedModule {
    DecodedModule {
        module: ResolvedModule {
            name: name.to_vec(),
            access: access(&definition.access),
            // Classic documentation is a string, so an undocumented module is
            // as likely to carry `""` as to carry nothing.
            doc: definition.value.doc.clone().filter(|text| !text.is_empty()),
            types: definition
                .value
                .types
                .iter()
                .map(|(type_name, declaration)| declaration_of(type_name, declaration))
                .collect(),
            // In-package dependencies are a frontend notion; generation reads
            // every reference's package out of the name itself.
            depends_on: Vec::new(),
            skipped_values: Vec::new(),
        },
        omitted_values: definition.value.values.len(),
        unsupported: Vec::new(),
    }
}

type ClassicDeclaration =
    AccessControlled<morphir_core::ir::classic::Documented<TypeDefinition<Attrs>>>;

fn declaration_of(name: &Name, declaration: &ClassicDeclaration) -> ResolvedType {
    let (params, body) = match &declaration.value.value {
        TypeDefinition::Alias(params, alias) => (params, ResolvedBody::Alias(ty(alias))),
        TypeDefinition::Custom(params, constructors) => (
            params,
            ResolvedBody::Custom {
                constructor_access: access(&constructors.access),
                constructors: constructors
                    .value
                    .iter()
                    .map(|constructor| RConstructor {
                        name: names::type_spelling_of(&words(&constructor.name)),
                        args: constructor
                            .args
                            .iter()
                            .map(|(_, argument)| ty(argument))
                            .collect(),
                    })
                    .collect(),
            },
        ),
    };
    ResolvedType {
        name: names::type_spelling_of(&words(name)),
        access: access(&declaration.access),
        doc: Some(declaration.value.doc.clone()).filter(|text| !text.is_empty()),
        params: params
            .iter()
            .map(|param| names::value_spelling_of(&words(param)))
            .collect(),
        body,
    }
}

/// A classic type expression, with every identifier in its document spelling.
pub fn ty(value: &Type<Attrs>) -> RType {
    match value {
        Type::Variable(_, variable) => RType::Var(names::value_spelling_of(&words(variable))),
        Type::Reference(_, reference, arguments) => RType::Ref(
            FqName {
                package: path(&reference.package_path),
                module: path(&reference.module_path),
                name: names::type_spelling_of(&words(&reference.local_name)),
            },
            arguments.iter().map(ty).collect(),
        ),
        Type::Record(_, fields) => RType::Record(fields.iter().map(field).collect()),
        Type::ExtensibleRecord(_, variable, fields) => RType::ExtensibleRecord(
            names::value_spelling_of(&words(variable)),
            fields.iter().map(field).collect(),
        ),
        Type::Tuple(_, elements) => RType::Tuple(elements.iter().map(ty).collect()),
        Type::Function(_, argument, result) => {
            RType::Function(Box::new(ty(argument)), Box::new(ty(result)))
        }
        Type::Unit(_) => RType::Unit,
    }
}

fn field(source: &morphir_core::ir::classic::Field<Attrs>) -> RField {
    RField {
        name: names::value_spelling_of(&words(&source.name)),
        ty: ty(&source.ty),
    }
}

fn access(access: &CAccess) -> Access {
    match access {
        CAccess::Public => Access::Public,
        CAccess::Private => Access::Private,
    }
}

/// The document-spelled segments of a classic module or package path.
pub fn path(path: &Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| names::type_spelling_of(&words(segment)))
        .collect()
}

/// The document spelling of a path a caller stated.
pub fn spelled_path(path: &[String]) -> Vec<String> {
    path.iter()
        .map(|segment| names::type_spelling(segment))
        .collect()
}

/// A classic name's words.
pub fn words(name: &Name) -> Vec<String> {
    name.words
        .iter()
        .map(|word| resolve(*word).to_owned())
        .collect()
}
