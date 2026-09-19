//! Emit the type-only classic contract, using the shared typed codec.
//! Values are omitted by the frontend with a diagnostic before reaching this module.
use morphir_core::{
    ir::{classic as c, v4 as v},
    naming,
};

type Result<T> = std::result::Result<T, String>;
type Type = c::Type<c::Attrs>;

fn name(value: &naming::Name) -> Result<c::Name> {
    let encoded = c::Name::from_str(&value.to_canonical_string());
    let decoded = morphir_core::migration::migrate_name(&encoded, &Default::default())
        .map_err(|e| e.message)?;
    if decoded != *value {
        return Err(format!(
            "Name '{value}' cannot be represented losslessly in IR v3"
        ));
    }
    Ok(encoded)
}
fn key(value: &str) -> Result<c::Name> {
    name(&naming::Name::from_canonical_string(value).map_err(|e| e.to_string())?)
}
fn path(value: &naming::Path) -> Result<c::Path> {
    Ok(c::Path {
        segments: value.segments.iter().map(name).collect::<Result<_>>()?,
    })
}
fn path_key(value: &str) -> Result<c::Path> {
    path(&naming::Path::from_canonical_string(value).map_err(|e| e.to_string())?)
}
fn fqname(value: &naming::FQName) -> Result<c::FQName> {
    Ok(c::FQName {
        package_path: path(&value.package_path)?,
        module_path: path(&value.module_path)?,
        local_name: name(&value.local_name)?,
    })
}
fn access(value: v::Access) -> c::Access {
    match value {
        v::Access::Public => c::Access::Public,
        v::Access::Private => c::Access::Private,
    }
}
fn docs(value: &Option<v::Documentation>) -> String {
    value
        .as_ref()
        .map(|d| d.text().to_owned())
        .unwrap_or_default()
}
fn params(value: &[naming::Name]) -> Result<Vec<c::Name>> {
    value.iter().map(name).collect()
}
fn fields(value: &[v::Field]) -> Result<Vec<c::Field<c::Attrs>>> {
    value
        .iter()
        .map(|f| {
            Ok(c::Field {
                name: name(&f.name)?,
                ty: tpe(&f.tpe)?,
            })
        })
        .collect()
}
fn tpe(value: &v::Type) -> Result<Type> {
    let a = c::Attrs::None;
    Ok(match value {
        v::Type::Variable(_, n) => c::Type::Variable(a, name(n)?),
        v::Type::Unit(_) => c::Type::Unit(a),
        v::Type::Reference(_, n, args) => {
            c::Type::Reference(a, fqname(n)?, args.iter().map(tpe).collect::<Result<_>>()?)
        }
        v::Type::Tuple(_, elements) => {
            c::Type::Tuple(a, elements.iter().map(tpe).collect::<Result<_>>()?)
        }
        v::Type::Record(_, fs) => c::Type::Record(a, fields(fs)?),
        v::Type::ExtensibleRecord(_, n, fs) => c::Type::ExtensibleRecord(a, name(n)?, fields(fs)?),
        v::Type::Function(_, arg, result) => {
            c::Type::Function(a, Box::new(tpe(arg)?), Box::new(tpe(result)?))
        }
    })
}
fn definition(value: &v::TypeDefinition) -> Result<c::TypeDefinition<c::Attrs>> {
    Ok(match value {
        v::TypeDefinition::TypeAliasDefinition {
            type_params,
            type_expr,
        } => c::TypeDefinition::Alias(params(type_params)?, tpe(type_expr)?),
        v::TypeDefinition::CustomTypeDefinition {
            type_params,
            constructors,
        } => c::TypeDefinition::Custom(
            params(type_params)?,
            c::AccessControlled {
                access: access(constructors.access),
                value: constructors
                    .value
                    .iter()
                    .map(|ctor| {
                        Ok(c::Constructor {
                            name: name(&ctor.name)?,
                            args: ctor
                                .args
                                .iter()
                                .map(|arg| Ok((name(&arg.name)?, tpe(&arg.arg_type)?)))
                                .collect::<Result<_>>()?,
                        })
                    })
                    .collect::<Result<_>>()?,
            },
        ),
        v::TypeDefinition::IncompleteTypeDefinition { .. } => {
            return Err("Incomplete type definitions cannot be represented in IR v3".into());
        }
    })
}
fn specification(value: &v::TypeSpecification) -> Result<c::TypeSpecification<c::Attrs>> {
    Ok(match value {
        v::TypeSpecification::TypeAliasSpecification { annotations, type_params, type_expr } if annotations.is_empty() => c::TypeSpecification::Alias(params(type_params)?, tpe(type_expr)?),
        v::TypeSpecification::OpaqueTypeSpecification { annotations, type_params } if annotations.is_empty() => c::TypeSpecification::Opaque(params(type_params)?),
        v::TypeSpecification::CustomTypeSpecification { annotations, type_params, constructors } if annotations.is_empty() => c::TypeSpecification::Custom(params(type_params)?, constructors.iter().map(|ctor| Ok(c::Constructor { name: name(&ctor.name)?, args: ctor.args.iter().map(|arg| Ok((name(&arg.name)?, tpe(&arg.arg_type)?))).collect::<Result<_>>()? })).collect::<Result<_>>()?),
        _ => return Err("Annotated or derived dependency specifications cannot be represented by the Gleam IR v3 writer".into()),
    })
}
fn package_specification(
    value: &v::PackageSpecification,
) -> Result<c::PackageSpecification<c::Attrs>> {
    Ok(c::PackageSpecification {
        modules: value
            .modules
            .iter()
            .map(|(module_name, module)| {
                if !module.annotations.is_empty() {
                    return Err("Module annotations cannot be represented in IR v3".into());
                }
                Ok(c::package::ModuleSpecEntry {
                    path: path_key(module_name)?,
                    specification: c::ModuleSpecification {
                        types: module
                            .types
                            .iter()
                            .map(|(n, item)| {
                                Ok((
                                    key(n)?,
                                    c::Documented::new(
                                        docs(&item.doc),
                                        specification(&item.value)?,
                                    ),
                                ))
                            })
                            .collect::<Result<_>>()?,
                        values: module
                            .values
                            .iter()
                            .map(|(n, item)| {
                                if !item.value.annotations.is_empty() {
                                    return Err(
                                        "Value annotations cannot be represented in IR v3".into()
                                    );
                                }
                                Ok((
                                    key(n)?,
                                    c::Documented::new(
                                        docs(&item.doc),
                                        c::ValueSpecification {
                                            inputs: item
                                                .value
                                                .inputs
                                                .iter()
                                                .map(|(n, t)| {
                                                    Ok(c::value::ValueParameter {
                                                        name: key(n)?,
                                                        ty: tpe(t)?,
                                                    })
                                                })
                                                .collect::<Result<_>>()?,
                                            output: tpe(&item.value.output)?,
                                        },
                                    ),
                                ))
                            })
                            .collect::<Result<_>>()?,
                        doc: module.doc.as_ref().map(|d| d.text().to_owned()),
                    },
                })
            })
            .collect::<Result<_>>()?,
    })
}
pub(super) fn encode(ir: &v::IRFile) -> Result<c::Distribution> {
    let v::Distribution::Library(library) = &ir.distribution else {
        return Err("Gleam IR v3 requires a Library".into());
    };
    let modules = library
        .def
        .modules
        .iter()
        .map(|(module_name, module)| {
            if !module.value.values.is_empty() {
                return Err(
                    "IR v3 value lowering requires inferred types; use IR v4 for Gleam functions"
                        .into(),
                );
            }
            Ok(c::ModuleEntry {
                path: path_key(module_name)?,
                definition: c::AccessControlled {
                    access: access(module.access),
                    value: c::ModuleDefinition {
                        types: module
                            .value
                            .types
                            .iter()
                            .map(|(n, item)| {
                                Ok((
                                    key(n)?,
                                    c::AccessControlled {
                                        access: access(item.access),
                                        value: c::Documented::new(
                                            docs(&item.value.doc),
                                            definition(&item.value.value)?,
                                        ),
                                    },
                                ))
                            })
                            .collect::<Result<_>>()?,
                        values: vec![],
                        doc: module.value.doc.as_ref().map(|d| d.text().to_owned()),
                    },
                },
            })
        })
        .collect::<Result<_>>()?;
    Ok(c::Distribution {
        format_version: 3,
        distribution: c::DistributionBody::Library(
            path(library.package_name.as_path())?,
            library
                .dependencies
                .iter()
                .map(|(n, spec)| Ok((path_key(n)?, package_specification(spec)?)))
                .collect::<Result<_>>()?,
            c::PackageDefinition { modules },
        ),
    })
}
