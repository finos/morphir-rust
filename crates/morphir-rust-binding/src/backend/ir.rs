use crate::{Outcome, error};
use morphir_core::{
    format_version::{NormalizedFormatVersion, ScalarValue, SupportTable},
    ir::{classic, v4::*},
    migration::{MigrationOptions, migrate_distribution},
    naming::Name,
};
use std::collections::BTreeMap;

pub(super) struct Package {
    pub declarations: Vec<Declaration>,
    pub functions: Vec<Function>,
    pub dependencies: BTreeMap<String, usize>,
    pub modules: BTreeMap<String, Access>,
    pub omitted_values: usize,
}

pub(super) struct Function {
    pub owner: Declaration,
    pub definition: ValueDefinition,
}

pub(super) struct Declaration {
    pub fqname: String,
    pub module: String,
    pub name: Name,
    pub params: Vec<Name>,
    pub access: Access,
    pub doc: Option<String>,
    pub body: Body,
}

pub(super) enum Body {
    Alias(Type),
    Custom(Access, Vec<ConstructorDefinition>),
    Opaque,
    Derived(Type, String, String),
}

pub(super) fn decode(value: &serde_json::Value) -> Outcome<Package> {
    let scalar = ScalarValue::from_json(
        value
            .get("formatVersion")
            .ok_or_else(|| error("RS_VERSION", "Missing formatVersion"))?,
    )
    .map_err(|e| error("RS_VERSION", e.to_string()))?;
    let version = NormalizedFormatVersion::from_scalar(&scalar, &SupportTable::reference())
        .map_err(|e| error("RS_VERSION", e.to_string()))?;
    if !version.is_supported() || !matches!(version.release.major(), 3 | 4) {
        return Err(error(
            "RS_VERSION",
            "Only supported IR v3 and v4 releases can be generated",
        ));
    }
    let mut value = value.clone();
    value["formatVersion"] = version.release.major().into();
    let ir: IRFile = if version.release.major() == 3 {
        let classic: classic::Distribution =
            serde_json::from_value(value).map_err(|e| error("RS_IR", e.to_string()))?;
        migrate_distribution(&classic, MigrationOptions::default())
            .map_err(|e| error("RS_IR", format!("{e:?}")))?
            .value
    } else {
        serde_json::from_value(value).map_err(|e| error("RS_IR", e.to_string()))?
    };
    package_from_ir(ir)
}

fn package_from_ir(ir: IRFile) -> Outcome<Package> {
    let mut result = Package {
        declarations: vec![],
        functions: vec![],
        dependencies: BTreeMap::new(),
        modules: BTreeMap::new(),
        omitted_values: 0,
    };
    match ir.distribution {
        Distribution::Library(library) => {
            dependencies(&mut result, &library.dependencies);
            definitions(
                &mut result,
                &library.package_name.to_canonical_string(),
                library.def,
            )?;
        }
        Distribution::Application(application) => {
            dependencies(
                &mut result,
                &application
                    .dependencies
                    .iter()
                    .map(|(name, definition)| (name.clone(), definition.to_specification()))
                    .collect(),
            );
            definitions(
                &mut result,
                &application.package_name.to_canonical_string(),
                application.def,
            )?;
        }
        Distribution::Specs(specs) => {
            dependencies(&mut result, &specs.dependencies);
            let package = specs.package_name.to_canonical_string();
            for (module_name, module) in specs.spec.modules {
                result.modules.insert(module_name.clone(), Access::Public);
                result.omitted_values += module.values.len();
                for (name, documented) in module.types {
                    let (params, body) = specification(documented.value);
                    result.declarations.push(Declaration {
                        fqname: format!("{package}:{module_name}#{name}"),
                        module: module_name.clone(),
                        name: Name::from_canonical_string(&name)
                            .map_err(|e| error("RS_NAME", e))?,
                        params,
                        access: Access::Public,
                        doc: documented.doc.map(|d| d.text().to_owned()),
                        body,
                    });
                }
            }
        }
    }
    result.declarations.sort_by(|a, b| a.fqname.cmp(&b.fqname));
    Ok(result)
}

fn definitions(result: &mut Package, package: &str, definitions: PackageDefinition) -> Outcome<()> {
    for (module_name, module) in definitions.modules {
        result.modules.insert(module_name.clone(), module.access);
        for (name, controlled) in module.value.values {
            let definition = controlled.value.value;
            if !matches!(definition.body, ValueBody::Expression(_)) {
                result.omitted_values += 1;
                continue;
            }
            let mut variables = std::collections::BTreeSet::new();
            for tpe in definition
                .input_types
                .values()
                .chain(definition.output_type.iter())
            {
                super::types::variables(tpe, &mut variables);
            }
            result.functions.push(Function {
                owner: Declaration {
                    fqname: format!("{package}:{module_name}#{name}"),
                    module: module_name.clone(),
                    name: Name::from_canonical_string(&name).map_err(|e| error("RS_NAME", e))?,
                    params: variables
                        .iter()
                        .map(|v| Name::from_canonical_string(v).map_err(|e| error("RS_NAME", e)))
                        .collect::<Outcome<_>>()?,
                    access: controlled.access,
                    doc: controlled.value.doc.map(|d| d.text().to_owned()),
                    body: Body::Opaque,
                },
                definition,
            });
        }

        for (name, controlled) in module.value.types {
            let (params, body) = match controlled.value.value {
                TypeDefinition::TypeAliasDefinition {
                    type_params,
                    type_expr,
                } => (type_params, Body::Alias(type_expr)),
                TypeDefinition::CustomTypeDefinition {
                    type_params,
                    constructors,
                } => (
                    type_params,
                    Body::Custom(constructors.access, constructors.value),
                ),
                TypeDefinition::IncompleteTypeDefinition { .. } => {
                    return Err(error(
                        "RS_INCOMPLETE",
                        format!("Cannot generate incomplete type {package}:{module_name}#{name}"),
                    ));
                }
            };
            result.declarations.push(Declaration {
                fqname: format!("{package}:{module_name}#{name}"),
                module: module_name.clone(),
                name: Name::from_canonical_string(&name).map_err(|e| error("RS_NAME", e))?,
                params,
                access: controlled.access,
                doc: controlled.value.doc.map(|d| d.text().to_owned()),
                body,
            });
        }
    }
    Ok(())
}

fn specification(spec: TypeSpecification) -> (Vec<Name>, Body) {
    match spec {
        TypeSpecification::TypeAliasSpecification {
            type_params,
            type_expr,
            ..
        } => (type_params, Body::Alias(type_expr)),
        TypeSpecification::OpaqueTypeSpecification { type_params, .. } => {
            (type_params, Body::Opaque)
        }
        TypeSpecification::DerivedTypeSpecification {
            type_params,
            base_type,
            from_base_type,
            to_base_type,
            ..
        } => (
            type_params,
            Body::Derived(
                base_type,
                from_base_type.to_canonical_string(),
                to_base_type.to_canonical_string(),
            ),
        ),
        TypeSpecification::CustomTypeSpecification {
            type_params,
            constructors,
            ..
        } => (
            type_params,
            Body::Custom(
                Access::Public,
                constructors
                    .into_iter()
                    .map(|c| ConstructorDefinition {
                        name: c.name,
                        args: c
                            .args
                            .into_iter()
                            .map(|a| ConstructorArg {
                                name: a.name,
                                arg_type: a.arg_type,
                            })
                            .collect(),
                    })
                    .collect(),
            ),
        ),
    }
}

fn dependencies(result: &mut Package, dependencies: &Dependencies) {
    for (package, specification) in dependencies {
        for (module, spec) in &specification.modules {
            for (name, definition) in &spec.types {
                let params = match &definition.value {
                    TypeSpecification::TypeAliasSpecification { type_params, .. }
                    | TypeSpecification::OpaqueTypeSpecification { type_params, .. }
                    | TypeSpecification::CustomTypeSpecification { type_params, .. }
                    | TypeSpecification::DerivedTypeSpecification { type_params, .. } => {
                        type_params
                    }
                };
                result
                    .dependencies
                    .insert(format!("{package}:{module}#{name}"), params.len());
            }
        }
    }
}
