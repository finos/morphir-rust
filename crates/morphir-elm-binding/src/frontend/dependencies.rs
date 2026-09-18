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
//! Only the public surface survives: private types, private constructors,
//! private modules, docs and type bodies are all dropped, because nothing
//! downstream of a name lookup can observe them.

use morphir_core::ir::classic::{
    Access as ClassicAccess, AccessControlled as ClassicAccessControlled, Attrs,
    Distribution as ClassicDistribution, DistributionBody, ModuleDefinition, Name as ClassicName,
    Path as ClassicPath, Type as ClassicType, TypeDefinition as ClassicTypeDefinition,
};
use morphir_core::ir::v4::{
    Access as V4Access, AccessControlled as V4AccessControlled, Distribution as V4Distribution,
    IRFile, ModuleDefinition as V4ModuleDefinition, TypeDefinition as V4TypeDefinition,
};
use morphir_core::naming::{ModuleName, Name as V4Name, Path as V4Path, resolve};
use morphir_extension_sdk::{CompileDependency, Diagnostic};
use serde_json::Value;

use crate::frontend::boundary;
use crate::frontend::resolve::DependencyInterface;
use crate::resolved::{Interface, InterfaceType};

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
            let (params, constructors) = match &declaration.value.value {
                ClassicTypeDefinition::Alias(params, _) => (params, None),
                ClassicTypeDefinition::Custom(params, constructors) => (
                    params,
                    (constructors.access == ClassicAccess::Public).then(|| {
                        constructors
                            .value
                            .iter()
                            .map(|constructor| title_case(&classic_words(&constructor.name)))
                            .collect()
                    }),
                ),
            };
            InterfaceType {
                name: title_case(&classic_words(type_name)),
                params: params
                    .iter()
                    .map(|param| camel_case(&classic_words(param)))
                    .collect(),
                constructors,
            }
        })
        .collect();
    types.sort_by(|left, right| left.name.cmp(&right.name));
    Interface {
        name: name.to_vec(),
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
            let (params, constructors) = match &declaration.value.value {
                V4TypeDefinition::TypeAliasDefinition { type_params, .. } => {
                    (type_params.clone(), None)
                }
                V4TypeDefinition::CustomTypeDefinition {
                    type_params,
                    constructors,
                } => (
                    type_params.clone(),
                    (constructors.access == V4Access::Public).then(|| {
                        constructors
                            .value
                            .iter()
                            .map(|constructor| title_case(&constructor.name.words()))
                            .collect()
                    }),
                ),
                // Anything else this version can hold still declares a name;
                // its parameters are not ones a reference can spell.
                _ => (Vec::new(), None),
            };
            InterfaceType {
                name: title_case(&v4_words(key)),
                params: params
                    .iter()
                    .map(|param| camel_case(&param.words()))
                    .collect(),
                constructors,
            }
        })
        .collect();
    types.sort_by(|left, right| left.name.cmp(&right.name));
    Interface {
        name: name.to_vec(),
        types,
    }
}

/// The source-spelled path of a classic module or package path.
fn classic_module_path(path: &ClassicPath) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| title_case(&classic_words(segment)))
        .collect()
}

/// The source-spelled path of a canonical v4 module name.
fn v4_module_path(canonical: &str) -> Result<Vec<String>, String> {
    let name = ModuleName::from_canonical_string(canonical)
        .map_err(|error| format!("`{canonical}` is not a canonical module name: {error}"))?;
    Ok(v4_path(name.as_path()))
}

/// The source-spelled segments of a v4 path.
fn v4_path(path: &V4Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| title_case(&segment.words()))
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

/// `["local", "date"]` becomes `LocalDate`, and `["s", "d", "k"]` becomes `SDK`
/// — the inverse of the word split both emitters share.
fn title_case(words: &[String]) -> String {
    words.iter().map(|word| capitalize(word)).collect()
}

/// The lowercase-initial spelling a type parameter is written with.
fn camel_case(words: &[String]) -> String {
    words
        .iter()
        .enumerate()
        .map(|(index, word)| {
            if index == 0 {
                word.clone()
            } else {
                capitalize(word)
            }
        })
        .collect()
}

fn capitalize(word: &str) -> String {
    let mut characters = word.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::emit::emitter_for;
    use crate::resolved::{
        Access, RConstructor, RType, ResolvedBody, ResolvedModule, ResolvedType,
    };

    fn module() -> ResolvedModule {
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
                    body: ResolvedBody::Alias(RType::Unit),
                },
                ResolvedType {
                    name: "Status".into(),
                    access: Access::Public,
                    doc: None,
                    params: vec!["a".into()],
                    body: ResolvedBody::Custom {
                        constructor_access: Access::Public,
                        constructors: vec![RConstructor {
                            name: "Active".into(),
                            args: vec![],
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
