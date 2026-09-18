//! Reading Morphir IR back into the version-neutral interface model.
//!
//! Two callers need this. The incremental compiler decodes a *baseline module
//! definition* so that a reused module still has a public interface for its
//! dependents to resolve against, and the request boundary decodes a
//! *dependency distribution* into the module interfaces the resolver looks
//! names up in. Both versions are read here, through `morphir_core`'s own
//! models rather than by walking JSON, so a document this crate cannot read is
//! reported instead of silently misread.
//!
//! Only the public surface survives: private types, private constructors and
//! private modules are dropped, as are docs, because nothing a dependent writes
//! can observe them. Public type *bodies* do survive — an alias publishes the
//! type it stands for and a custom type publishes its constructors' argument
//! types — so that retyping either is an interface change, and so that the
//! interface read back out of a baseline equals the one resolution produced.

use morphir_core::ir::classic::{
    Access as ClassicAccess, AccessControlled as ClassicAccessControlled, Attrs,
    Distribution as ClassicDistribution, DistributionBody, ModuleDefinition, Name as ClassicName,
    Path as ClassicPath, Type as ClassicType, TypeDefinition as ClassicTypeDefinition,
};
use morphir_core::ir::v4::{
    Access as V4Access, AccessControlled as V4AccessControlled, Distribution as V4Distribution,
    Field as V4Field, IRFile, ModuleDefinition as V4ModuleDefinition, Type as V4Type,
    TypeDefinition as V4TypeDefinition,
};
use morphir_core::naming::{ModuleName, Name as V4Name, Path as V4Path, resolve};
use morphir_extension_sdk::{CompileDependency, Diagnostic};
use serde_json::Value;

use crate::frontend::boundary;
use crate::frontend::resolve::DependencyInterface;
use crate::names;
use crate::resolved::{FqName, Interface, InterfaceType, RField, RType};

/// The classic module definition this frontend writes and reads.
type ClassicModule = ModuleDefinition<Attrs, ClassicType<Attrs>>;

/// The public interface of one module definition, as
/// [`crate::frontend::emit::Emitter::emit_module`] wrote it.
///
/// A module definition carries no name of its own — the name is the key its
/// package filed it under — so the caller states it.
pub fn interface_from_module_ir(
    ir_version: &str,
    name: &[String],
    ir: &Value,
) -> Result<Interface, String> {
    match ir_version {
        "3" => serde_json::from_value::<ClassicAccessControlled<ClassicModule>>(ir.clone())
            .map(|definition| classic_interface(name, &definition))
            .map_err(|error| format!("not a v3 module definition: {error}")),
        "4" => serde_json::from_value::<V4AccessControlled<V4ModuleDefinition>>(ir.clone())
            .map(|definition| v4_interface(name, &definition))
            .map_err(|error| format!("not a v4 module definition: {error}")),
        other => Err(format!("unsupported IR version `{other}`")),
    }
}

/// The dependency interfaces a compile request supplies, and one `ELM_REQUEST`
/// diagnostic per distribution that could not be read. An unreadable dependency
/// is ignored: the references that needed it report themselves.
pub fn from_request(
    dependencies: &[CompileDependency],
) -> (Vec<DependencyInterface>, Vec<Diagnostic>) {
    let mut interfaces = Vec::with_capacity(dependencies.len());
    let mut diagnostics = Vec::new();
    for dependency in dependencies {
        match dependency_interface(dependency) {
            Ok(interface) => interfaces.push(interface),
            Err(reason) => diagnostics.push(boundary::request_error(format!(
                "dependency `{}` is ignored: {reason}",
                dependency.package_name
            ))),
        }
    }
    (interfaces, diagnostics)
}

/// The v4 package specifications a request's dependencies publish, as
/// `(package path, v4 `PackageSpecification` JSON)` — what a v4 `Library` holds
/// in its `dependencies` map.
///
/// Only a dependency whose `irVersion` is `"4"` and whose distribution is a v4
/// `Library` has one; a classic dependency is skipped, and so is a distribution
/// that does not decode, because [`from_request`] has already reported it.
pub fn v4_specifications(dependencies: &[CompileDependency]) -> Vec<(Vec<String>, Value)> {
    dependencies
        .iter()
        .filter(|dependency| dependency.ir_version == "4")
        .filter_map(|dependency| {
            let file: IRFile = serde_json::from_value(dependency.distribution.clone()).ok()?;
            let V4Distribution::Library(library) = file.distribution else {
                return None;
            };
            let specification = serde_json::to_value(library.def.to_specification()).ok()?;
            Some((v4_path(library.package_name.as_path()), specification))
        })
        .collect()
}

fn dependency_interface(dependency: &CompileDependency) -> Result<DependencyInterface, String> {
    match dependency.ir_version.as_str() {
        "3" => classic_dependency(dependency),
        "4" => v4_dependency(dependency),
        other => Err(format!("unsupported `irVersion` `{other}`")),
    }
}

fn classic_dependency(dependency: &CompileDependency) -> Result<DependencyInterface, String> {
    let distribution: ClassicDistribution = serde_json::from_value(dependency.distribution.clone())
        .map_err(|error| format!("not a v3 distribution: {error}"))?;
    let DistributionBody::Library(package, _, definition) = distribution.distribution;
    let modules = definition
        .modules
        .iter()
        .filter(|entry| entry.definition.access == ClassicAccess::Public)
        .map(|entry| {
            let name = classic_module_path(&entry.path);
            classic_interface(&name, &entry.definition)
        })
        .collect();
    Ok(DependencyInterface {
        package: classic_module_path(&package),
        modules,
    })
}

fn v4_dependency(dependency: &CompileDependency) -> Result<DependencyInterface, String> {
    let file: IRFile = serde_json::from_value(dependency.distribution.clone())
        .map_err(|error| format!("not a v4 distribution: {error}"))?;
    let V4Distribution::Library(library) = file.distribution else {
        return Err("only a v4 `Library` supplies module definitions".to_string());
    };
    let mut modules = Vec::new();
    for (key, definition) in &library.def.modules {
        if definition.access != V4Access::Public {
            continue;
        }
        let name = v4_module_path(key)?;
        modules.push(v4_interface(&name, definition));
    }
    Ok(DependencyInterface {
        package: v4_path(library.package_name.as_path()),
        modules,
    })
}

fn classic_interface(
    name: &[String],
    definition: &ClassicAccessControlled<ClassicModule>,
) -> Interface {
    let mut types: Vec<InterfaceType> = definition
        .value
        .types
        .iter()
        .filter(|(_, declaration)| declaration.access == ClassicAccess::Public)
        .map(|(type_name, declaration)| {
            let (params, alias, constructors) = match &declaration.value.value {
                ClassicTypeDefinition::Alias(params, body) => {
                    (params, Some(classic_type(body)), None)
                }
                ClassicTypeDefinition::Custom(params, constructors) => (
                    params,
                    None,
                    (constructors.access == ClassicAccess::Public).then(|| {
                        constructors
                            .value
                            .iter()
                            .map(|constructor| {
                                (
                                    names::type_spelling_of(&classic_words(&constructor.name)),
                                    constructor
                                        .args
                                        .iter()
                                        .map(|(_, argument)| classic_type(argument))
                                        .collect(),
                                )
                            })
                            .collect()
                    }),
                ),
            };
            InterfaceType {
                name: names::type_spelling_of(&classic_words(type_name)),
                params: params
                    .iter()
                    .map(|param| names::value_spelling_of(&classic_words(param)))
                    .collect(),
                alias,
                constructors,
            }
        })
        .collect();
    types.sort_by(|left, right| left.name.cmp(&right.name));
    Interface {
        name: spelled_path(name),
        types,
    }
}

fn v4_interface(name: &[String], definition: &V4AccessControlled<V4ModuleDefinition>) -> Interface {
    let mut types: Vec<InterfaceType> = definition
        .value
        .types
        .iter()
        .filter(|(_, declaration)| declaration.access == V4Access::Public)
        .map(|(key, declaration)| {
            let (params, alias, constructors) = match &declaration.value.value {
                V4TypeDefinition::TypeAliasDefinition {
                    type_params,
                    type_expr,
                } => (type_params.clone(), Some(v4_type(type_expr)), None),
                V4TypeDefinition::CustomTypeDefinition {
                    type_params,
                    constructors,
                } => (
                    type_params.clone(),
                    None,
                    (constructors.access == V4Access::Public).then(|| {
                        constructors
                            .value
                            .iter()
                            .map(|constructor| {
                                (
                                    names::type_spelling_of(&constructor.name.words()),
                                    constructor
                                        .args
                                        .iter()
                                        .map(|argument| v4_type(&argument.arg_type))
                                        .collect(),
                                )
                            })
                            .collect()
                    }),
                ),
                // A type this version can hold but Elm cannot write still
                // declares a name; it publishes no shape, so it reads back as
                // opaque, exactly as `PackageDefinition::to_specification` does.
                _ => (Vec::new(), None, None),
            };
            InterfaceType {
                name: names::type_spelling_of(&v4_words(key)),
                params: params
                    .iter()
                    .map(|param| names::value_spelling_of(&param.words()))
                    .collect(),
                alias,
                constructors,
            }
        })
        .collect();
    types.sort_by(|left, right| left.name.cmp(&right.name));
    Interface {
        name: spelled_path(name),
        types,
    }
}

fn classic_type(ty: &ClassicType<Attrs>) -> RType {
    match ty {
        ClassicType::Variable(_, variable) => {
            RType::Var(names::value_spelling_of(&classic_words(variable)))
        }
        ClassicType::Reference(_, reference, arguments) => RType::Ref(
            FqName {
                package: classic_module_path(&reference.package_path),
                module: classic_module_path(&reference.module_path),
                name: names::type_spelling_of(&classic_words(&reference.local_name)),
            },
            arguments.iter().map(classic_type).collect(),
        ),
        ClassicType::Record(_, fields) => RType::Record(
            fields
                .iter()
                .map(|field| RField {
                    name: names::value_spelling_of(&classic_words(&field.name)),
                    ty: classic_type(&field.ty),
                })
                .collect(),
        ),
        ClassicType::ExtensibleRecord(_, variable, fields) => RType::ExtensibleRecord(
            names::value_spelling_of(&classic_words(variable)),
            fields
                .iter()
                .map(|field| RField {
                    name: names::value_spelling_of(&classic_words(&field.name)),
                    ty: classic_type(&field.ty),
                })
                .collect(),
        ),
        ClassicType::Tuple(_, elements) => {
            RType::Tuple(elements.iter().map(classic_type).collect())
        }
        ClassicType::Function(_, argument, result) => RType::Function(
            Box::new(classic_type(argument)),
            Box::new(classic_type(result)),
        ),
        ClassicType::Unit(_) => RType::Unit,
    }
}

fn v4_type(ty: &V4Type) -> RType {
    match ty {
        V4Type::Variable(_, variable) => RType::Var(names::value_spelling_of(&variable.words())),
        V4Type::Reference(_, reference, arguments) => RType::Ref(
            FqName {
                package: v4_path(&reference.package_path),
                module: v4_path(&reference.module_path),
                name: names::type_spelling_of(&reference.local_name.words()),
            },
            arguments.iter().map(v4_type).collect(),
        ),
        V4Type::Record(_, fields) => RType::Record(fields.iter().map(v4_field).collect()),
        V4Type::ExtensibleRecord(_, variable, fields) => RType::ExtensibleRecord(
            names::value_spelling_of(&variable.words()),
            fields.iter().map(v4_field).collect(),
        ),
        V4Type::Tuple(_, elements) => RType::Tuple(elements.iter().map(v4_type).collect()),
        V4Type::Function(_, argument, result) => {
            RType::Function(Box::new(v4_type(argument)), Box::new(v4_type(result)))
        }
        V4Type::Unit(_) => RType::Unit,
    }
}

fn v4_field(field: &V4Field) -> RField {
    RField {
        name: names::value_spelling_of(&field.name.words()),
        ty: v4_type(&field.tpe),
    }
}

/// The document spelling of a path a caller stated.
fn spelled_path(path: &[String]) -> Vec<String> {
    path.iter()
        .map(|segment| names::type_spelling(segment))
        .collect()
}

/// The document-spelled path of a classic module or package path.
fn classic_module_path(path: &ClassicPath) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| names::type_spelling_of(&classic_words(segment)))
        .collect()
}

/// The document-spelled path of a canonical v4 module name.
fn v4_module_path(canonical: &str) -> Result<Vec<String>, String> {
    let name = ModuleName::from_canonical_string(canonical)
        .map_err(|error| format!("`{canonical}` is not a canonical module name: {error}"))?;
    Ok(v4_path(name.as_path()))
}

/// The document-spelled segments of a v4 path.
fn v4_path(path: &V4Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| names::type_spelling_of(&segment.words()))
        .collect()
}

fn classic_words(name: &ClassicName) -> Vec<String> {
    name.words
        .iter()
        .map(|word| resolve(*word).to_owned())
        .collect()
}

/// A canonical v4 name's words, falling back to the key itself when the key is
/// not one this crate would have written.
fn v4_words(canonical: &str) -> Vec<String> {
    match V4Name::from_canonical_string(canonical) {
        Ok(name) => name.words(),
        Err(_) => vec![canonical.to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::emit::emitter_for;
    use crate::resolved::{Access, RConstructor, ResolvedBody, ResolvedModule, ResolvedType};

    fn sdk(module: &str, name: &str) -> FqName {
        FqName {
            package: vec!["Morphir".into(), "SDK".into()],
            module: vec![module.into()],
            name: name.into(),
        }
    }

    /// A module covering every shape a type expression can take, so that the
    /// round trip is checked on all of them: an alias to a record (with a
    /// reference, a tuple, a function, a unit and an extensible record inside),
    /// and a parameterised custom type whose constructors carry argument types.
    fn module() -> ResolvedModule {
        let int = RType::Ref(sdk("Basics", "Int"), vec![]);
        let string = RType::Ref(sdk("String", "String"), vec![]);
        ResolvedModule {
            name: vec!["My".into(), "Types".into()],
            access: Access::Public,
            doc: None,
            types: vec![
                ResolvedType {
                    name: "LocalDate".into(),
                    access: Access::Public,
                    doc: Some("A date.".into()),
                    params: vec![],
                    body: ResolvedBody::Alias(RType::Record(vec![
                        RField {
                            name: "id".into(),
                            ty: string.clone(),
                        },
                        RField {
                            name: "dayOfYear".into(),
                            ty: int.clone(),
                        },
                        RField {
                            name: "pair".into(),
                            ty: RType::Tuple(vec![int.clone(), RType::Unit]),
                        },
                        RField {
                            name: "render".into(),
                            ty: RType::Function(
                                Box::new(RType::ExtensibleRecord(
                                    "r".into(),
                                    vec![RField {
                                        name: "label".into(),
                                        ty: string.clone(),
                                    }],
                                )),
                                Box::new(string.clone()),
                            ),
                        },
                    ])),
                },
                ResolvedType {
                    name: "Status".into(),
                    access: Access::Public,
                    doc: None,
                    params: vec!["a".into(), "errorType".into()],
                    body: ResolvedBody::Custom {
                        constructor_access: Access::Public,
                        constructors: vec![
                            RConstructor {
                                name: "Active".into(),
                                args: vec![],
                            },
                            RConstructor {
                                name: "Closed".into(),
                                args: vec![
                                    string.clone(),
                                    RType::Var("a".into()),
                                    RType::Ref(
                                        sdk("List", "List"),
                                        vec![RType::Var("errorType".into())],
                                    ),
                                ],
                            },
                        ],
                    },
                },
                ResolvedType {
                    name: "Opaque".into(),
                    access: Access::Public,
                    doc: None,
                    params: vec![],
                    body: ResolvedBody::Custom {
                        constructor_access: Access::Private,
                        constructors: vec![RConstructor {
                            name: "Opaque".into(),
                            args: vec![int],
                        }],
                    },
                },
                ResolvedType {
                    name: "Hidden".into(),
                    access: Access::Private,
                    doc: None,
                    params: vec![],
                    body: ResolvedBody::Alias(RType::Unit),
                },
            ],
            depends_on: vec![],
            skipped_values: vec![],
        }
    }

    /// The whole point of the decoders: an interface read back out of a module
    /// definition is the interface resolution produced, down to the type
    /// bodies, so a baseline entry and a fresh compile are comparable.
    #[test]
    fn a_module_definition_round_trips_through_its_interface_in_both_versions() {
        let module = module();
        for version in ["3", "4"] {
            let emitter = emitter_for(version).expect("a supported version");
            let ir = emitter.emit_module(&module).expect("the module is written");
            let interface = interface_from_module_ir(version, &module.name, &ir)
                .unwrap_or_else(|error| panic!("v{version}: {error}"));

            assert_eq!(interface, module.interface(), "v{version}");
        }
    }

    #[test]
    fn the_interface_carries_the_specification_of_every_public_type() {
        let interface = module().interface();

        let names: Vec<&str> = interface.types.iter().map(|ty| ty.name.as_str()).collect();
        assert_eq!(names, ["LocalDate", "Opaque", "Status"]);

        let alias = &interface.types[0];
        assert!(matches!(alias.alias, Some(RType::Record(_))));
        assert!(alias.constructors.is_none());

        // Private constructors leave the type opaque: neither body is published.
        let opaque = &interface.types[1];
        assert!(opaque.alias.is_none() && opaque.constructors.is_none());

        let custom = &interface.types[2];
        assert_eq!(custom.params, ["a", "errorType"]);
        assert!(custom.alias.is_none());
        let constructors = custom.constructors.as_ref().expect("public constructors");
        assert_eq!(constructors[0].0, "Active");
        assert!(constructors[0].1.is_empty());
        assert_eq!(constructors[1].0, "Closed");
        assert_eq!(constructors[1].1.len(), 3);
    }

    #[test]
    fn retyping_an_exposed_alias_changes_the_digest() {
        let mut retyped = module();
        retyped.types[0].body = ResolvedBody::Alias(RType::Unit);

        assert_ne!(retyped.interface_digest(), module().interface_digest());
    }

    #[test]
    fn a_value_that_is_not_a_module_definition_is_reported() {
        let error = interface_from_module_ir("3", &["My".into()], &serde_json::json!({"nope": 1}))
            .expect_err("a bare object is not a module definition");
        assert!(error.contains("v3 module definition"), "{error}");
    }

    #[test]
    fn an_unreadable_dependency_is_ignored_with_a_diagnostic() {
        let (interfaces, diagnostics) = from_request(&[CompileDependency {
            package_name: "morphir/sdk".into(),
            ir_version: "3".into(),
            distribution: serde_json::json!({"modules": {}}),
        }]);

        assert!(interfaces.is_empty());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code.as_deref(), Some(boundary::REQUEST));
    }

    #[test]
    fn a_distribution_becomes_the_dependency_interfaces_it_declares() {
        let module = module();
        for version in ["3", "4"] {
            let emitter = emitter_for(version).expect("a supported version");
            let ir = emitter.emit_module(&module).expect("the module is written");
            let package = vec!["Acme".to_string(), "Lib".to_string()];
            let distribution = emitter
                .emit_distribution(
                    &crate::frontend::emit::PackageInput {
                        package: &package,
                        modules: std::slice::from_ref(&module),
                        dependencies: &[],
                    },
                    &[(module.name.clone(), Access::Public, ir)],
                )
                .expect("the distribution is written");

            let (interfaces, diagnostics) = from_request(&[CompileDependency {
                package_name: "acme/lib".into(),
                ir_version: version.into(),
                distribution,
            }]);

            assert!(diagnostics.is_empty(), "v{version}: {diagnostics:?}");
            assert_eq!(interfaces.len(), 1);
            assert_eq!(interfaces[0].package, package, "v{version}");
            assert_eq!(
                interfaces[0].modules,
                vec![module.interface()],
                "v{version}"
            );
        }
    }
}
