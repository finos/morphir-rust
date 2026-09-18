//! The v4 emitter.
//!
//! A v4 document is written straight from the resolved model; nothing here
//! migrates a classic one. Names go through the same word split the classic
//! emitter uses and are then rebuilt with [`Name::from_words`], which collapses a
//! run of single letters back into an initialism — so `SDK` is the initialism
//! `SDK` and `LocalDate` is `local-date`. Doing it this way is what makes the
//! natively emitted document identical to the one a caller gets by migrating the
//! classic sibling, which `tests/emit.rs` checks.

use indexmap::IndexMap;
use morphir_core::ir::classic::Name as ClassicName;
use morphir_core::ir::v4::{
    Access as VAccess, AccessControlled, ConstructorArg, ConstructorDefinition, Distribution,
    Documentation, Documented, FormatVersion, IRFile, LibraryContent, ModuleDefinition, Name,
    PackageDefinition, Type, TypeAttributes, TypeDefinition,
};
use morphir_core::naming::{FQName, ModuleName, PackageName, Path, resolve};
use serde_json::Value;

use super::{Emitter, ModuleIr, PackageInput};
use crate::resolved::{Access, FqName, RConstructor, RType, ResolvedBody, ResolvedModule};

pub struct V4Emitter;

impl Emitter for V4Emitter {
    fn emit_module(&self, module: &ResolvedModule) -> Value {
        let definition = AccessControlled {
            access: access(module.access),
            value: module_definition(module),
        };
        serde_json::to_value(&definition).expect("a v4 module definition serializes")
    }

    fn emit_distribution(&self, input: &PackageInput, module_irs: &[ModuleIr]) -> Value {
        let modules = module_irs
            .iter()
            .map(|(module_path, module_access, module_ir)| {
                let definition: AccessControlled<ModuleDefinition> =
                    serde_json::from_value(module_ir.clone())
                        .expect("a module value emit_module wrote");
                (
                    module_name(module_path).to_canonical_string(),
                    AccessControlled {
                        access: access(*module_access),
                        value: definition.value,
                    },
                )
            })
            .collect();

        let file = IRFile {
            format_version: FormatVersion::Integer(4),
            distribution: Distribution::Library(LibraryContent {
                package_name: package_name(input.package),
                // A v4 `Library` keys its dependencies by canonical package name
                // and holds each one's *specification*. Nothing supplies those
                // yet — `PackageInput::dependencies` carries classic package
                // specifications, which are the classic emitter's to write — so
                // the map stays empty until a caller has real v4 specifications.
                dependencies: IndexMap::new(),
                def: PackageDefinition { modules },
            }),
        };
        serde_json::to_value(&file).expect("a v4 distribution serializes")
    }

    fn format_version(&self) -> &'static str {
        "4"
    }
}

fn access(access: Access) -> VAccess {
    match access {
        Access::Public => VAccess::Public,
        Access::Private => VAccess::Private,
    }
}

/// The v4 name an identifier spells, by way of the classic word split.
fn name(source: &str) -> Name {
    Name::from_words(
        ClassicName::from_str(source)
            .words
            .iter()
            .map(|word| resolve(*word).to_owned()),
    )
}

fn path(segments: &[String]) -> Path {
    Path {
        segments: segments.iter().map(|segment| name(segment)).collect(),
    }
}

fn module_name(segments: &[String]) -> ModuleName {
    ModuleName::new(path(segments))
}

fn package_name(segments: &[String]) -> PackageName {
    PackageName::new(path(segments))
}

fn fqname(reference: &FqName) -> FQName {
    FQName::new(
        path(&reference.package),
        path(&reference.module),
        name(&reference.name),
    )
}

/// The name morphir-elm gives the `index`-th positional constructor argument,
/// migrated to v4: the classic words `["arg", "1"]` spell `arg-1`.
fn argument_name(index: usize) -> Name {
    Name::from_words(["arg".to_string(), (index + 1).to_string()])
}

fn attrs() -> TypeAttributes {
    TypeAttributes::default()
}

fn ty(value: &RType) -> Type {
    match value {
        RType::Var(variable) => Type::variable(attrs(), name(variable)),
        RType::Ref(reference, arguments) => Type::reference(
            attrs(),
            fqname(reference),
            arguments.iter().map(ty).collect(),
        ),
        RType::Record(fields) => Type::record(attrs(), fields.iter().map(field).collect()),
        RType::ExtensibleRecord(variable, fields) => {
            Type::extensible_record(attrs(), name(variable), fields.iter().map(field).collect())
        }
        RType::Tuple(elements) => Type::tuple(attrs(), elements.iter().map(ty).collect()),
        RType::Function(argument, result) => Type::function(attrs(), ty(argument), ty(result)),
        RType::Unit => Type::unit(attrs()),
    }
}

fn field(source: &crate::resolved::RField) -> morphir_core::ir::v4::Field {
    morphir_core::ir::v4::Field::new(name(&source.name), ty(&source.ty))
}

fn constructor(source: &RConstructor) -> ConstructorDefinition {
    ConstructorDefinition {
        name: name(&source.name),
        args: source
            .args
            .iter()
            .enumerate()
            .map(|(index, argument)| ConstructorArg {
                name: argument_name(index),
                arg_type: ty(argument),
            })
            .collect(),
    }
}

fn type_definition(body: &ResolvedBody, params: &[String]) -> TypeDefinition {
    let type_params = params.iter().map(|param| name(param)).collect();
    match body {
        ResolvedBody::Alias(alias) => TypeDefinition::TypeAliasDefinition {
            type_params,
            type_expr: ty(alias),
        },
        ResolvedBody::Custom {
            constructor_access,
            constructors,
        } => TypeDefinition::CustomTypeDefinition {
            type_params,
            constructors: AccessControlled {
                access: access(*constructor_access),
                value: constructors.iter().map(constructor).collect(),
            },
        },
    }
}

fn module_definition(module: &ResolvedModule) -> ModuleDefinition {
    ModuleDefinition {
        types: module
            .types
            .iter()
            .map(|declaration| {
                (
                    name(&declaration.name).to_canonical_string(),
                    AccessControlled {
                        access: access(declaration.access),
                        value: Documented::new(
                            declaration.doc.clone().map(Documentation::new),
                            type_definition(&declaration.body, &declaration.params),
                        ),
                    },
                )
            })
            .collect(),
        // Value declarations are skipped by this frontend.
        values: IndexMap::new(),
        doc: module.doc.clone().map(Documentation::new),
    }
}
