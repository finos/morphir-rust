//! The classic (v3) emitter.
//!
//! Classic names are word lists, so every identifier goes through
//! [`classic::Name::from_str`], which splits `LocalDate` into `["local","date"]`
//! and `SDK` into `["s","d","k"]` exactly as morphir-elm's `Name.fromString` does.
//! Type nodes carry no attributes, so every attribute is [`Attrs::None`], which
//! writes `{}`.

use morphir_core::ir::classic::{
    Access as CAccess, AccessControlled, Attrs, Constructor, Distribution, DistributionBody,
    Documented, FQName, Field, ModuleDefinition, ModuleEntry, Name, PackageDefinition,
    PackageSpecification, Path, Type, TypeDefinition,
};
use serde_json::Value;

use super::{Emitter, ModuleIr, PackageInput};
use crate::resolved::{Access, FqName, RConstructor, RType, ResolvedBody, ResolvedModule};

/// The classic module definition, with type attributes and no value attributes.
type ClassicModule = ModuleDefinition<Attrs, Type<Attrs>>;

pub struct ClassicEmitter;

impl Emitter for ClassicEmitter {
    fn emit_module(&self, module: &ResolvedModule) -> Value {
        let definition = AccessControlled {
            access: access(module.access),
            value: module_definition(module),
        };
        serde_json::to_value(&definition).expect("a classic module definition serializes")
    }

    fn emit_distribution(&self, input: &PackageInput, module_irs: &[ModuleIr]) -> Value {
        let modules = module_irs
            .iter()
            .map(|(module_path, module_access, module_ir)| {
                let definition: AccessControlled<ClassicModule> =
                    serde_json::from_value(module_ir.clone())
                        .expect("a module value emit_module wrote");
                ModuleEntry {
                    path: path(module_path),
                    definition: AccessControlled {
                        access: access(*module_access),
                        value: definition.value,
                    },
                }
            })
            .collect();

        let dependencies = input
            .dependencies
            .iter()
            .map(|(dependency_path, specification)| {
                let specification: PackageSpecification<Attrs> =
                    serde_json::from_value(specification.clone())
                        .expect("a classic package specification");
                (path(dependency_path), specification)
            })
            .collect();

        let distribution = Distribution {
            format_version: 3,
            distribution: DistributionBody::Library(
                path(input.package),
                dependencies,
                PackageDefinition { modules },
            ),
        };
        serde_json::to_value(&distribution).expect("a classic distribution serializes")
    }

    fn format_version(&self) -> &'static str {
        "3"
    }
}

fn access(access: Access) -> CAccess {
    match access {
        Access::Public => CAccess::Public,
        Access::Private => CAccess::Private,
    }
}

fn name(source: &str) -> Name {
    Name::from_str(source)
}

fn path(segments: &[String]) -> Path {
    Path::new(segments.iter().map(|segment| name(segment)).collect())
}

fn fqname(reference: &FqName) -> FQName {
    FQName::new(
        path(&reference.package),
        path(&reference.module),
        name(&reference.name),
    )
}

/// The name morphir-elm gives the `index`-th positional constructor argument.
///
/// `Morphir.Elm.Frontend` builds the word list `[ "arg", String.fromInt (index + 1) ]`
/// directly, so the argument names are one-based and their words are already split.
fn argument_name(index: usize) -> Name {
    Name::new(["arg".to_string(), (index + 1).to_string()])
}

fn ty(value: &RType) -> Type<Attrs> {
    match value {
        RType::Var(variable) => Type::Variable(Attrs::None, name(variable)),
        RType::Ref(reference, arguments) => Type::Reference(
            Attrs::None,
            fqname(reference),
            arguments.iter().map(ty).collect(),
        ),
        RType::Record(fields) => Type::Record(
            Attrs::None,
            fields
                .iter()
                .map(|field| Field {
                    name: name(&field.name),
                    ty: ty(&field.ty),
                })
                .collect(),
        ),
        RType::ExtensibleRecord(variable, fields) => Type::ExtensibleRecord(
            Attrs::None,
            name(variable),
            fields
                .iter()
                .map(|field| Field {
                    name: name(&field.name),
                    ty: ty(&field.ty),
                })
                .collect(),
        ),
        RType::Tuple(elements) => Type::Tuple(Attrs::None, elements.iter().map(ty).collect()),
        RType::Function(argument, result) => {
            Type::Function(Attrs::None, Box::new(ty(argument)), Box::new(ty(result)))
        }
        RType::Unit => Type::Unit(Attrs::None),
    }
}

fn constructor(source: &RConstructor) -> Constructor<Attrs> {
    Constructor {
        name: name(&source.name),
        args: source
            .args
            .iter()
            .enumerate()
            .map(|(index, argument)| (argument_name(index), ty(argument)))
            .collect(),
    }
}

fn type_definition(body: &ResolvedBody, params: &[String]) -> TypeDefinition<Attrs> {
    let params = params.iter().map(|param| name(param)).collect();
    match body {
        ResolvedBody::Alias(alias) => TypeDefinition::Alias(params, ty(alias)),
        ResolvedBody::Custom {
            constructor_access,
            constructors,
        } => TypeDefinition::Custom(
            params,
            AccessControlled {
                access: access(*constructor_access),
                value: constructors.iter().map(constructor).collect(),
            },
        ),
    }
}

fn module_definition(module: &ResolvedModule) -> ClassicModule {
    ModuleDefinition {
        types: module
            .types
            .iter()
            .map(|declaration| {
                (
                    name(&declaration.name),
                    AccessControlled {
                        access: access(declaration.access),
                        value: Documented {
                            // Classic documentation is a string, so an
                            // undocumented declaration carries the empty one.
                            doc: declaration.doc.clone().unwrap_or_default(),
                            value: type_definition(&declaration.body, &declaration.params),
                        },
                    },
                )
            })
            .collect(),
        // Value declarations are skipped by this frontend.
        values: vec![],
        doc: module.doc.clone(),
    }
}
