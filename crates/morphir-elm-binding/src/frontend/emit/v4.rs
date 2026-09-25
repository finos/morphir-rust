//! The v4 emitter.
//!
//! A v4 document is written straight from the resolved model; nothing here
//! migrates a classic one. Names go through the word split in
//! [`crate::names`] and are then rebuilt with [`Name::from_words`], which
//! collapses a run of single letters back into an initialism — so `SDK` is the
//! initialism `SDK` and `LocalDate` is `local-date`. Doing it this way is what
//! makes the natively emitted document identical to the one a caller gets by
//! migrating the classic sibling, which `tests/emit.rs` checks.
//!
//! Serialization encoding: v4 type expressions are written under the thread-local
//! [`morphir_core::ir::v4::TypeEncoding`], which defaults to `Expanded`. Neither
//! [`V4Emitter::emit_module`] nor [`V4Emitter::emit_distribution`] sets it, so a
//! caller wrapping either in `with_type_encoding` chooses the spelling — but both
//! must run under the *same* choice, because `emit_distribution` reads the stored
//! module values back through the v4 reader and writes them out again. A baseline
//! written compact and assembled expanded (or the reverse) still decodes, since
//! the reader accepts both, but the assembled document is written in whichever
//! encoding is in force during assembly.

use std::collections::HashMap;

use indexmap::IndexMap;
use morphir_core::ir::v4::{
    Access as VAccess, AccessControlled, ConstructorArg, ConstructorDefinition, Distribution,
    Documentation, Documented, FormatVersion, IRFile, LibraryContent, ModuleDefinition, Name,
    PackageDefinition, PackageSpecification, Type, TypeAttributes, TypeDefinition,
};
use morphir_core::naming::{FQName, ModuleName, PackageName, Path};
use serde_json::Value;

use super::{EmitError, Emitter, ModuleIr, Ordering, PackageInput, in_order, name_key, path_key};
use crate::names::{argument_words, module_label, words};
use crate::resolved::{Access, FqName, RConstructor, RType, ResolvedBody, ResolvedModule};

/// Writes Morphir IR v4 natively.
pub struct V4Emitter {
    /// The order modules, types and constructors are written in. A v4 document
    /// keys its modules and types by canonical name in an `IndexMap`, so this
    /// is the order they are inserted in.
    pub ordering: Ordering,
}

impl Emitter for V4Emitter {
    fn emit_module(&self, module: &ResolvedModule) -> Result<Value, EmitError> {
        let definition = AccessControlled {
            access: access(module.access),
            value: module_definition(module, self.ordering)?,
        };
        serde_json::to_value(&definition).map_err(|error| {
            format!(
                "module `{}` could not be written as a v4 module definition: {error}",
                module_label(&module.name)
            )
        })
    }

    fn emit_distribution(
        &self,
        input: &PackageInput,
        module_irs: &[ModuleIr],
    ) -> Result<Value, EmitError> {
        let mut modules: IndexMap<String, AccessControlled<ModuleDefinition>> = IndexMap::new();
        let mut written: HashMap<String, String> = HashMap::new();
        for (module_path, module_access, module_ir) in
            in_order(module_irs, self.ordering, |(path, _, _)| path_key(path))
        {
            let label = module_label(module_path);
            let definition: AccessControlled<ModuleDefinition> =
                serde_json::from_value(module_ir.clone()).map_err(|error| {
                    format!("module `{label}` IR is not a valid v4 module definition: {error}")
                })?;
            let key = module_name(module_path).to_canonical_string();
            // An `IndexMap` would replace the earlier module silently, so a
            // collision of canonical names is reported instead.
            if let Some(earlier) = written.insert(key.clone(), label.clone()) {
                return Err(format!(
                    "module `{label}` and module `{earlier}` share the canonical module name \
                     `{key}`, so only one of them could be written"
                ));
            }
            modules.insert(
                key,
                AccessControlled {
                    access: access(*module_access),
                    value: definition.value,
                },
            );
        }

        // A v4 `Library` keys its dependencies by canonical package name and
        // holds each one's *specification*. Two package paths that share a
        // canonical name would silently replace one another in an `IndexMap`,
        // so a collision is reported.
        let mut dependencies: IndexMap<String, PackageSpecification> = IndexMap::new();
        let mut written: HashMap<String, String> = HashMap::new();
        for (dependency_path, specification) in input.dependencies {
            let label = module_label(dependency_path);
            let specification: PackageSpecification = serde_json::from_value(specification.clone())
                .map_err(|error| {
                    format!("dependency `{label}` is not a valid v4 package specification: {error}")
                })?;
            let key = package_name(dependency_path).to_canonical_string();
            if let Some(earlier) = written.insert(key.clone(), label.clone()) {
                return Err(format!(
                    "dependency `{label}` and dependency `{earlier}` share the canonical package \
                     name `{key}`, so only one of them could be written"
                ));
            }
            dependencies.insert(key, specification);
        }

        let file = IRFile {
            format_version: FormatVersion::Integer(4),
            metadata: None,
            distribution: Distribution::Library(LibraryContent {
                package_name: package_name(input.package),
                dependencies,
                def: PackageDefinition { modules },
            }),
        };
        serde_json::to_value(&file)
            .map_err(|error| format!("the v4 distribution could not be written: {error}"))
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

/// The v4 name an identifier spells, by way of the shared word split.
fn name(source: &str) -> Name {
    Name::from_words(words(source))
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
                name: Name::from_words(argument_words(index)),
                arg_type: ty(argument),
            })
            .collect(),
    }
}

fn type_definition(body: &ResolvedBody, params: &[String], ordering: Ordering) -> TypeDefinition {
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
                value: in_order(constructors, ordering, |source| name_key(&source.name))
                    .into_iter()
                    .map(constructor)
                    .collect(),
            },
        },
    }
}

fn module_definition(
    module: &ResolvedModule,
    ordering: Ordering,
) -> Result<ModuleDefinition, EmitError> {
    let mut types: IndexMap<String, AccessControlled<Documented<TypeDefinition>>> = IndexMap::new();
    let mut written: HashMap<String, String> = HashMap::new();
    for declaration in in_order(&module.types, ordering, |declaration| {
        name_key(&declaration.name)
    }) {
        let key = name(&declaration.name).to_canonical_string();
        // An `IndexMap` would replace the earlier declaration silently.
        if let Some(earlier) = written.insert(key.clone(), declaration.name.clone()) {
            return Err(format!(
                "in module `{}`, types `{}` and `{earlier}` share the canonical name `{key}`, so \
                 only one of them could be written",
                module_label(&module.name),
                declaration.name
            ));
        }
        types.insert(
            key,
            AccessControlled {
                access: access(declaration.access),
                value: Documented::new(
                    declaration.doc.clone().map(Documentation::new),
                    type_definition(&declaration.body, &declaration.params, ordering),
                ),
            },
        );
    }

    Ok(ModuleDefinition {
        types,
        // Value declarations are skipped by this frontend.
        values: IndexMap::new(),
        doc: module.doc.clone().map(Documentation::new),
    })
}
