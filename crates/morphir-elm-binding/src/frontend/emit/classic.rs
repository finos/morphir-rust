//! The classic (v3) emitter.
//!
//! Classic names are word lists, so every identifier goes through
//! [`names::words`], which splits `LocalDate` into `["local","date"]` and `SDK`
//! into `["s","d","k"]` exactly as morphir-elm's `Name.fromString` does. Type
//! nodes carry no attributes, so every attribute is [`Attrs::None`], which writes
//! `{}`.

use std::collections::HashSet;

use morphir_core::ir::classic::{
    Access as CAccess, AccessControlled, Attrs, Constructor, Distribution, DistributionBody,
    Documented, FQName, Field, ModuleDefinition, ModuleEntry, Name, PackageDefinition,
    PackageSpecification, Path, Type, TypeDefinition,
};
use serde_json::Value;

use super::names::{argument_words, module_label, words};
use super::{EmitError, Emitter, ModuleIr, PackageInput};
use crate::resolved::{Access, FqName, RConstructor, RType, ResolvedBody, ResolvedModule};

/// The classic module definition, with type attributes and no value attributes.
type ClassicModule = ModuleDefinition<Attrs, Type<Attrs>>;

pub struct ClassicEmitter;

impl Emitter for ClassicEmitter {
    fn emit_module(&self, module: &ResolvedModule) -> Result<Value, EmitError> {
        let definition = AccessControlled {
            access: access(module.access),
            value: module_definition(module),
        };
        serde_json::to_value(&definition).map_err(|error| {
            format!(
                "module `{}` could not be written as a v3 module definition: {error}",
                module_label(&module.name)
            )
        })
    }

    fn emit_distribution(
        &self,
        input: &PackageInput,
        module_irs: &[ModuleIr],
    ) -> Result<Value, EmitError> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut modules = Vec::with_capacity(module_irs.len());
        for (module_path, module_access, module_ir) in module_irs {
            let label = module_label(module_path);
            let definition: AccessControlled<ClassicModule> =
                serde_json::from_value(module_ir.clone()).map_err(|error| {
                    format!("module `{label}` IR is not a valid v3 module definition: {error}")
                })?;
            let path = path(module_path);
            if !seen.insert(path.to_string()) {
                return Err(format!(
                    "module `{label}` is written twice: two module paths share the name `{path}`"
                ));
            }
            modules.push(ModuleEntry {
                path,
                definition: AccessControlled {
                    access: access(*module_access),
                    value: definition.value,
                },
            });
        }

        let mut dependencies = Vec::with_capacity(input.dependencies.len());
        for (dependency_path, specification) in input.dependencies {
            let specification: PackageSpecification<Attrs> =
                serde_json::from_value(specification.clone()).map_err(|error| {
                    format!(
                        "dependency `{}` is not a valid v3 package specification: {error}",
                        module_label(dependency_path)
                    )
                })?;
            dependencies.push((path(dependency_path), specification));
        }

        let distribution = Distribution {
            format_version: 3,
            distribution: DistributionBody::Library(
                path(input.package),
                dependencies,
                PackageDefinition { modules },
            ),
        };
        serde_json::to_value(&distribution)
            .map_err(|error| format!("the v3 distribution could not be written: {error}"))
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
    Name::new(words(source))
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

fn ty(value: &RType) -> Type<Attrs> {
    match value {
        RType::Var(variable) => Type::Variable(Attrs::None, name(variable)),
        RType::Ref(reference, arguments) => Type::Reference(
            Attrs::None,
            fqname(reference),
            arguments.iter().map(ty).collect(),
        ),
        RType::Record(fields) => Type::Record(Attrs::None, fields.iter().map(field).collect()),
        RType::ExtensibleRecord(variable, fields) => Type::ExtensibleRecord(
            Attrs::None,
            name(variable),
            fields.iter().map(field).collect(),
        ),
        RType::Tuple(elements) => Type::Tuple(Attrs::None, elements.iter().map(ty).collect()),
        RType::Function(argument, result) => {
            Type::Function(Attrs::None, Box::new(ty(argument)), Box::new(ty(result)))
        }
        RType::Unit => Type::Unit(Attrs::None),
    }
}

fn field(source: &crate::resolved::RField) -> Field<Attrs> {
    Field {
        name: name(&source.name),
        ty: ty(&source.ty),
    }
}

fn constructor(source: &RConstructor) -> Constructor<Attrs> {
    Constructor {
        name: name(&source.name),
        args: source
            .args
            .iter()
            .enumerate()
            .map(|(index, argument)| (Name::new(argument_words(index)), ty(argument)))
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
