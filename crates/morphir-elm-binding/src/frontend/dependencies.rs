//! Reading Morphir IR back into the version-neutral interface model.
//!
//! Two callers need this. The incremental compiler decodes a *baseline module
//! definition* so that a reused module still has a public interface for its
//! dependents to resolve against, and the request boundary decodes a
//! *dependency distribution* into the module interfaces the resolver looks
//! names up in.
//!
//! Neither reads IR itself: both go through [`crate::backend::decode`], the one
//! reader per IR version that the backend also generates from. There is
//! therefore exactly one place that knows how a v3 document differs from a v4
//! one, and a baseline interface is by construction the interface the backend
//! would raise the same document into.
//!
//! Only the public surface survives: private types, private constructors and
//! private modules are dropped, as are docs, because nothing a dependent writes
//! can observe them. Public type *bodies* do survive — an alias publishes the
//! type it stands for and a custom type publishes its constructors' argument
//! types — so that retyping either is an interface change, and so that the
//! interface read back out of a baseline equals the one resolution produced.
//! That filtering lives in [`crate::resolved::ResolvedModule::interface`].

use morphir_core::ir::v4::{Distribution as V4Distribution, IRFile};
use morphir_extension_sdk::{CompileDependency, Diagnostic};
use serde_json::Value;

use crate::backend::decode;
use crate::frontend::boundary;
use crate::frontend::resolve::DependencyInterface;
use crate::resolved::{Access, Interface};

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
    decode::interface_of(ir_version, name, ir)
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
            Some((
                decode::v4::path(library.package_name.as_path()),
                specification,
            ))
        })
        .collect()
}

/// A dependency distribution, read by the version it says it is written in, and
/// reduced to the public interface of each of its public modules.
fn dependency_interface(dependency: &CompileDependency) -> Result<DependencyInterface, String> {
    let decoded = match dependency.ir_version.as_str() {
        "3" => decode::classic::decode(&dependency.distribution),
        "4" => decode::v4::decode(&dependency.distribution),
        other => Err(format!("unsupported `irVersion` `{other}`")),
    }?;

    Ok(DependencyInterface {
        package: decoded.package,
        modules: decoded
            .modules
            .iter()
            .filter(|module| module.access == Access::Public)
            .map(|module| module.interface())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::emit::emitter_for;
    use crate::resolved::{
        FqName, RConstructor, RField, RType, ResolvedBody, ResolvedModule, ResolvedType,
    };

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
