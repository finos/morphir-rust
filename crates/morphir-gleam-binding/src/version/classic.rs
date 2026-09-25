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

type ClassicValue = c::Value<c::Attrs, c::Type<c::Attrs>>;

fn value_type(value: &v::ValueAttributes) -> Result<Type> {
    tpe(value
        .inferred_type
        .as_deref()
        .ok_or("IR v3 requires an inferred type on every value")?)
}

fn literal(value: &v::Literal) -> Result<c::Literal> {
    Ok(match value {
        v::Literal::Bool(v) => c::Literal::Bool(*v),
        v::Literal::Char(v) => c::Literal::Char(*v),
        v::Literal::String(v) => c::Literal::String(v.clone()),
        v::Literal::Integer(v) => c::Literal::WholeNumber(
            v.to_string()
                .parse()
                .map_err(|_| format!("Integer {v} is outside IR v3 range"))?,
        ),
        v::Literal::Float(v) => {
            c::Literal::Float(v.lexeme().parse().map_err(|_| "Invalid IR v3 float")?)
        }
        v::Literal::Decimal(v) => c::Literal::Decimal(v.clone()),
        v::Literal::Document(_) => {
            return Err("Document literals cannot be represented in IR v3".into());
        }
    })
}

fn pattern(value: &v::Pattern) -> Result<c::Pattern<Type>> {
    Ok(match value {
        v::Pattern::WildcardPattern(attrs) => c::Pattern::Wildcard(value_type(attrs)?),
        v::Pattern::AsPattern(attrs, inner, name_) => {
            c::Pattern::As(value_type(attrs)?, Box::new(pattern(inner)?), name(name_)?)
        }
        v::Pattern::TuplePattern(attrs, elements) => c::Pattern::Tuple(
            value_type(attrs)?,
            elements.iter().map(pattern).collect::<Result<_>>()?,
        ),
        v::Pattern::ConstructorPattern(attrs, name_, args) => c::Pattern::Constructor(
            value_type(attrs)?,
            fqname(name_)?,
            args.iter().map(pattern).collect::<Result<_>>()?,
        ),
        v::Pattern::EmptyListPattern(attrs) => c::Pattern::EmptyList(value_type(attrs)?),
        v::Pattern::HeadTailPattern(attrs, head, tail) => c::Pattern::HeadTail(
            value_type(attrs)?,
            Box::new(pattern(head)?),
            Box::new(pattern(tail)?),
        ),
        v::Pattern::LiteralPattern(attrs, lit) => {
            c::Pattern::Literal(value_type(attrs)?, literal(lit)?)
        }
        v::Pattern::UnitPattern(attrs) => c::Pattern::Unit(value_type(attrs)?),
    })
}

fn expression(value: &v::Value) -> Result<ClassicValue> {
    Ok(match value {
        v::Value::Literal(attrs, lit) => c::Value::Literal(value_type(attrs)?, literal(lit)?),
        v::Value::Constructor(attrs, name_) => {
            c::Value::Constructor(value_type(attrs)?, fqname(name_)?)
        }
        v::Value::Variable(attrs, name_) => c::Value::Variable(value_type(attrs)?, name(name_)?),
        v::Value::Reference(attrs, name_) => {
            c::Value::Reference(value_type(attrs)?, fqname(name_)?)
        }
        v::Value::Apply(attrs, function, argument) => c::Value::Apply(
            value_type(attrs)?,
            Box::new(expression(function)?),
            Box::new(expression(argument)?),
        ),
        v::Value::PatternMatch(attrs, subject, cases) => c::Value::PatternMatch(
            value_type(attrs)?,
            Box::new(expression(subject)?),
            cases
                .iter()
                .map(|v::PatternCase(pattern_, body)| Ok((pattern(pattern_)?, expression(body)?)))
                .collect::<Result<_>>()?,
        ),
        _ => return Err("Value form is not yet representable in IR v3".into()),
    })
}

fn value_definition(value: &v::ValueDefinition) -> Result<c::ValueDefinition<c::Attrs, Type>> {
    let body = match &value.body {
        v::ValueBody::Expression(body) => expression(body)?,
        _ => return Err("Only expression definitions can be represented in IR v3".into()),
    };
    Ok(c::ValueDefinition {
        input_types: value
            .input_types
            .iter()
            .map(|(n, ty)| {
                let ty = tpe(ty)?;
                Ok(c::value::ValueArgument {
                    name: key(n)?,
                    annotation: ty.clone(),
                    ty,
                })
            })
            .collect::<Result<_>>()?,
        output_type: tpe(value
            .output_type
            .as_ref()
            .ok_or("IR v3 requires a value output type")?)?,
        body,
    })
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod value_tests {
    use super::*;
    use morphir_core::ir::v4::ValueAttributes;

    fn integer_type() -> v::Type {
        v::Type::Reference(
            Default::default(),
            naming::FQName {
                package_path: naming::PackageName::parse("morphir/SDK").into(),
                module_path: naming::ModuleName::parse("basics").into(),
                local_name: naming::Name::from("int"),
            },
            vec![],
        )
    }

    fn attributes(ty: v::Type) -> ValueAttributes {
        ValueAttributes {
            inferred_type: Some(Box::new(ty)),
            ..Default::default()
        }
    }

    #[test]
    fn writer_preserves_typed_application() {
        let integer = integer_type();
        let function = v::Value::Variable(
            attributes(v::Type::Function(
                Default::default(),
                Box::new(integer.clone()),
                Box::new(integer.clone()),
            )),
            naming::Name::from("identity"),
        );
        let argument =
            v::Value::Literal(attributes(integer.clone()), v::Literal::Integer(2.into()));
        let written = expression(&v::Value::Apply(
            attributes(integer.clone()),
            Box::new(function),
            Box::new(argument),
        ))
        .unwrap();
        assert!(matches!(written, c::Value::Apply(_, function, argument)
            if matches!(*function, c::Value::Variable(_, _))
                && matches!(*argument, c::Value::Literal(_, c::Literal::WholeNumber(2)))));
    }

    #[test]
    fn writer_preserves_typed_empty_list_case() {
        let integer = integer_type();
        let list_type = v::Type::Reference(
            Default::default(),
            naming::FQName {
                package_path: naming::PackageName::parse("morphir/SDK").into(),
                module_path: naming::ModuleName::parse("list").into(),
                local_name: naming::Name::from("list"),
            },
            vec![integer.clone()],
        );
        let subject =
            v::Value::Variable(attributes(list_type.clone()), naming::Name::from("items"));
        let pattern = v::Pattern::EmptyListPattern(attributes(list_type));
        let value = v::Value::PatternMatch(
            attributes(integer.clone()),
            Box::new(subject),
            vec![v::PatternCase(
                pattern,
                v::Value::Literal(attributes(integer), v::Literal::Integer(0.into())),
            )],
        );
        let written = expression(&value).unwrap();
        assert!(matches!(written, c::Value::PatternMatch(_, _, cases)
            if matches!(&cases[0].0, c::Pattern::EmptyList(_))));
    }

    #[test]
    fn writer_preserves_head_tail_pattern_and_named_constructor() {
        let integer = integer_type();
        let list_type = v::Type::Reference(
            Default::default(),
            naming::FQName {
                package_path: naming::PackageName::parse("morphir/SDK").into(),
                module_path: naming::ModuleName::parse("list").into(),
                local_name: naming::Name::from("list"),
            },
            vec![integer.clone()],
        );
        let constructor = naming::FQName {
            package_path: naming::PackageName::parse("example/arity").into(),
            module_path: naming::ModuleName::parse("validation/arity").into(),
            local_name: naming::Name::from("valid"),
        };
        let value = v::Value::PatternMatch(
            attributes(integer.clone()),
            Box::new(v::Value::Variable(
                attributes(list_type.clone()),
                naming::Name::from("items"),
            )),
            vec![v::PatternCase(
                v::Pattern::HeadTailPattern(
                    attributes(list_type),
                    Box::new(v::Pattern::WildcardPattern(attributes(integer.clone()))),
                    Box::new(v::Pattern::AsPattern(
                        attributes(integer.clone()),
                        Box::new(v::Pattern::WildcardPattern(attributes(integer.clone()))),
                        naming::Name::from("rest"),
                    )),
                ),
                v::Value::Constructor(attributes(integer), constructor),
            )],
        );
        let written = expression(&value).unwrap();
        assert!(matches!(written, c::Value::PatternMatch(_, _, cases)
            if matches!(&cases[0].0, c::Pattern::HeadTail(_, _, _))
                && matches!(&cases[0].1, c::Value::Constructor(_, _))));
    }

    #[test]
    fn writer_emits_complete_v3_value_definition() {
        let integer = integer_type();
        let definition = v::ValueDefinition {
            input_types: [("input".into(), integer.clone())].into(),
            output_type: Some(integer.clone()),
            body: v::ValueBody::Expression(v::Value::Variable(
                attributes(integer),
                naming::Name::from("input"),
            )),
        };
        let module = v::ModuleDefinition {
            types: Default::default(),
            values: [(
                "identity".into(),
                v::AccessControlled {
                    access: v::Access::Public,
                    value: v::Documented::new(None, definition),
                },
            )]
            .into(),
            doc: None,
        };
        let file = v::IRFile {
            format_version: Default::default(),
            metadata: None,
            distribution: v::Distribution::Library(v::LibraryContent {
                package_name: naming::PackageName::parse("example/arity"),
                dependencies: Default::default(),
                def: v::PackageDefinition {
                    modules: [(
                        "main".into(),
                        v::AccessControlled {
                            access: v::Access::Public,
                            value: module,
                        },
                    )]
                    .into(),
                },
            }),
        };
        let written = encode(&file).unwrap();
        let c::DistributionBody::Library(_, _, package) = written.distribution else {
            panic!("a Library")
        };
        let values = &package.modules[0].definition.value.values;
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].1.value.value.input_types.len(), 1);
        assert!(matches!(
            values[0].1.value.value.body,
            c::Value::Variable(_, _)
        ));
    }

    #[test]
    fn writer_preserves_inferred_type_on_v3_integer_literal() {
        let value = v::Value::Literal(
            ValueAttributes {
                inferred_type: Some(Box::new(integer_type())),
                ..Default::default()
            },
            v::Literal::Integer(1.into()),
        );
        let written = expression(&value).unwrap();
        assert_eq!(
            written,
            c::Value::Literal(tpe(&integer_type()).unwrap(), c::Literal::WholeNumber(1))
        );
    }

    #[test]
    fn writer_rejects_missing_inferred_type() {
        let value = v::Value::Literal(Default::default(), v::Literal::Integer(1.into()));
        assert!(expression(&value).is_err());
    }
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
                        values: module
                            .value
                            .values
                            .iter()
                            .map(|(n, item)| {
                                Ok((
                                    key(n)?,
                                    c::AccessControlled {
                                        access: access(item.access),
                                        value: c::Documented::new(
                                            docs(&item.value.doc),
                                            value_definition(&item.value.value)?,
                                        ),
                                    },
                                ))
                            })
                            .collect::<Result<_>>()?,
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
