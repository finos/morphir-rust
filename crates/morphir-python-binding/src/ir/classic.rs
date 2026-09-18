//! Classic v3 encoding for the Python subset, using the shared concrete IR types.

mod expressions;
mod read;
pub(super) use read::decode;

use crate::{Outcome, error, values};
use morphir_core::{
    ir::{classic as c, v4 as v},
    naming,
};

type Type = c::Type<c::Attrs>;
type Definition = c::ValueDefinition<c::Attrs, Type>;

fn name(value: &naming::Name) -> Outcome<c::Name> {
    let encoded = c::Name::from_str(&value.to_canonical_string());
    let decoded = morphir_core::migration::migrate_name(&encoded, &Default::default())
        .map_err(|e| error("PY003", e.message))?;
    if decoded != *value {
        return Err(error(
            "PY003",
            format!(
                "Name '{}' cannot roundtrip through classic IR v3 word arrays; rename it or use v4",
                value.to_canonical_string()
            ),
        ));
    }
    Ok(encoded)
}

fn canonical_name(value: &str) -> Outcome<c::Name> {
    name(&naming::Name::from_canonical_string(value).map_err(|e| error("PY003", e))?)
}

fn path(value: &naming::Path) -> Outcome<c::Path> {
    Ok(c::Path {
        segments: value.segments.iter().map(name).collect::<Outcome<_>>()?,
    })
}

fn fqname(value: &naming::FQName) -> Outcome<c::FQName> {
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

fn tpe(value: &v::Type) -> Outcome<Type> {
    let a = c::Attrs::None;
    Ok(match value {
        v::Type::Reference(_, reference, args) => c::Type::Reference(
            a,
            fqname(reference)?,
            args.iter().map(tpe).collect::<Outcome<_>>()?,
        ),
        v::Type::Tuple(_, elements) => {
            c::Type::Tuple(a, elements.iter().map(tpe).collect::<Outcome<_>>()?)
        }
        v::Type::Record(_, fields) => c::Type::Record(
            a,
            fields
                .iter()
                .map(|f| {
                    Ok(c::Field {
                        name: name(&f.name)?,
                        ty: tpe(&f.tpe)?,
                    })
                })
                .collect::<Outcome<_>>()?,
        ),
        v::Type::Function(_, input, output) => {
            c::Type::Function(a, Box::new(tpe(input)?), Box::new(tpe(output)?))
        }
        _ => {
            return Err(values::unsupported(
                "Type cannot be encoded by the Python v3 subset",
            ));
        }
    })
}

fn definition(
    value: &v::ValueDefinition,
    aliases: &values::TupleAliases,
    signatures: &values::Signatures,
) -> Outcome<Definition> {
    let v::ValueBody::Expression(body) = &value.body else {
        return Err(values::unsupported("Expected an expression body"));
    };
    let mut clean = value.clone();
    clean.body = v::ValueBody::Expression(expressions::erase(body)?);
    let typed = values::annotate_function(&clean, aliases, signatures)?;
    Ok(Definition {
        input_types: value
            .input_types
            .iter()
            .map(|(key, value)| {
                Ok(c::value::ValueArgument {
                    name: canonical_name(key)?,
                    annotation: tpe(value)?,
                    ty: tpe(value)?,
                })
            })
            .collect::<Outcome<_>>()?,
        output_type: tpe(value
            .output_type
            .as_ref()
            .ok_or_else(|| values::unsupported("Expected a return type"))?)?,
        body: expressions::encode(body, &typed, aliases)?,
    })
}

pub(super) fn encode(ir: &v::IRFile) -> Outcome<c::Distribution> {
    let v::Distribution::Library(library) = &ir.distribution else {
        return Err(values::unsupported("Expected a Library"));
    };
    let signatures = values::signatures(library)?;
    let aliases = crate::modules::tuple_aliases(&library.package_name, library.def.modules.iter())?;
    let modules = library
        .def
        .modules
        .iter()
        .map(|(key, module)| {
            let types = module
                .value
                .types
                .iter()
                .map(|(key, item)| {
                    let value = match &item.value.value {
                        v::TypeDefinition::TypeAliasDefinition {
                            type_params,
                            type_expr,
                        } if type_params.is_empty() => {
                            c::TypeDefinition::Alias(vec![], tpe(type_expr)?)
                        }
                        v::TypeDefinition::CustomTypeDefinition {
                            type_params,
                            constructors,
                        } if type_params.is_empty() => c::TypeDefinition::Custom(
                            vec![],
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
                                                .map(|arg| {
                                                    Ok((name(&arg.name)?, tpe(&arg.arg_type)?))
                                                })
                                                .collect::<Outcome<_>>()?,
                                        })
                                    })
                                    .collect::<Outcome<_>>()?,
                            },
                        ),
                        _ => {
                            return Err(values::unsupported(
                                "Type definition cannot be encoded by the Python v3 subset",
                            ));
                        }
                    };
                    Ok((
                        canonical_name(key)?,
                        c::AccessControlled {
                            access: access(item.access),
                            value: c::Documented::new("", value),
                        },
                    ))
                })
                .collect::<Outcome<_>>()?;
            let values = module
                .value
                .values
                .iter()
                .map(|(key, item)| {
                    Ok((
                        canonical_name(key)?,
                        c::AccessControlled {
                            access: access(item.access),
                            value: c::Documented::new(
                                "",
                                definition(&item.value.value, &aliases, &signatures)?,
                            ),
                        },
                    ))
                })
                .collect::<Outcome<_>>()?;
            Ok(c::module::ModuleEntry {
                path: path(
                    &naming::Path::from_canonical_string(key).map_err(|e| error("PY003", e))?,
                )?,
                definition: c::AccessControlled {
                    access: access(module.access),
                    value: c::ModuleDefinition {
                        types,
                        values,
                        doc: None,
                    },
                },
            })
        })
        .collect::<Outcome<_>>()?;
    Ok(c::Distribution {
        format_version: 3,
        distribution: c::DistributionBody::Library(
            path(library.package_name.as_path())?,
            vec![],
            c::PackageDefinition { modules },
        ),
    })
}
