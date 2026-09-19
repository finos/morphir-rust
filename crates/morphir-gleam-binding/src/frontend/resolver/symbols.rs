//! Type interfaces and SDK mappings used by the resolver.
use super::*;

#[derive(Clone)]
pub(super) struct Symbol {
    pub(super) name: FQName,
    pub(super) arity: usize,
    pub(super) public: bool,
}
#[derive(Clone)]
pub(super) struct Interface {
    pub(super) package: String,
    pub(super) module: String,
    pub(super) types: BTreeMap<String, Symbol>,
}

/// Canonical Morphir module path for a slash-separated Gleam module name.
pub fn module_name(source_path: &str) -> String {
    ModuleName::parse(source_path.trim_end_matches(".gleam")).to_string()
}

pub(super) fn fqname(package: &PackageName, module: &str, name: &str) -> FQName {
    FQName {
        package_path: package.clone().into(),
        module_path: ModuleName::parse(module).into(),
        local_name: Name::from(name),
    }
}

/// The SDK type identity and source-language arity for a Gleam prelude type.
pub(crate) fn builtin(name: &str) -> Option<(FQName, usize)> {
    let (module, local, arity) = match name {
        "Int" => ("basics", "int", 0),
        "Bool" => ("basics", "bool", 0),
        "Float" => ("basics", "float", 0),
        "String" => ("string", "string", 0),
        "List" => ("list", "list", 1),
        "Result" => ("result", "result", 2),
        _ => return None,
    };
    Some((
        fqname(&PackageName::parse("morphir/SDK"), module, local),
        arity,
    ))
}

pub(super) fn ast_interface(package: &PackageName, module: &ModuleIR) -> Interface {
    Interface {
        package: package.to_string(),
        module: module_name(&module.name),
        types: module
            .types
            .iter()
            .map(|ty| {
                (
                    Name::from(ty.name.as_str()).to_string(),
                    Symbol {
                        name: fqname(package, &module.name, &ty.name),
                        arity: ty.params.len(),
                        public: ty.access == Access::Public,
                    },
                )
            })
            .collect(),
    }
}

pub(super) fn sdk_interfaces() -> Vec<Interface> {
    [
        ("option", "maybe", "option", 1),
        ("dict", "dict", "dict", 2),
        ("set", "set", "set", 1),
    ]
    .into_iter()
    .map(|(source, target, local, arity)| Interface {
        package: "morphir/SDK".into(),
        module: format!("gleam/{source}"),
        types: [(
            local.into(),
            Symbol {
                name: fqname(&PackageName::parse("morphir/SDK"), target, target),
                arity,
                public: true,
            },
        )]
        .into_iter()
        .collect(),
    })
    .collect()
}

pub(super) fn dependency_interfaces(
    dependencies: &IndexMap<String, PackageSpecification>,
) -> Vec<Interface> {
    dependencies
        .iter()
        .flat_map(|(package, spec)| {
            spec.modules.iter().map(move |(module, spec)| Interface {
                package: package.clone(),
                module: module_name(module),
                types: spec
                    .types
                    .iter()
                    .map(|(name, ty)| {
                        let arity = match &ty.value {
                            TypeSpecification::TypeAliasSpecification { type_params, .. }
                            | TypeSpecification::OpaqueTypeSpecification { type_params, .. }
                            | TypeSpecification::CustomTypeSpecification { type_params, .. }
                            | TypeSpecification::DerivedTypeSpecification { type_params, .. } => {
                                type_params.len()
                            }
                        };
                        (
                            Name::from(name.as_str()).to_string(),
                            Symbol {
                                name: fqname(&PackageName::parse(package), module, name),
                                arity,
                                public: true,
                            },
                        )
                    })
                    .collect(),
            })
        })
        .collect()
}
