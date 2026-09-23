//! Typed V3 value lowering after Gleam inference.

use super::morphir_type;
use gleam_core::ast::{self, BinOp, TypedExpr, TypedModule, TypedPattern, TypedStatement};
use gleam_core::type_::{Type as GleamType, ValueConstructorVariant};
use indexmap::IndexMap;
use morphir_core::ir::v4::{self as v4, AccessControlled, Documented, ValueDefinition};
use morphir_core::ir::v4::{Type as MorphirType, TypeAttributes};
use morphir_core::naming::{FQName, ModuleName, Name, PackageName};
use std::sync::Arc;

pub(crate) fn lower_typed_functions(
    module: &TypedModule,
    package: &PackageName,
) -> Result<IndexMap<String, AccessControlled<Documented<ValueDefinition>>>, String> {
    module
        .definitions
        .functions
        .iter()
        .map(|function| {
            let source_name = function
                .name
                .as_ref()
                .ok_or("Anonymous top-level function")?
                .1
                .as_str();
            let name = Name::from(source_name).to_string();
            let input_types = function
                .arguments
                .iter()
                .map(|argument| {
                    let name = argument
                        .get_variable_name()
                        .ok_or("Discarded public function parameter is not supported in V3")?;
                    Ok((
                        Name::from(name.as_str()).to_string(),
                        morphir_type(&argument.type_, package)?,
                    ))
                })
                .collect::<Result<IndexMap<_, _>, String>>()?;
            let body = lower_statements(&function.body, package, &module.name)?;
            let definition = ValueDefinition {
                input_types,
                output_type: Some(morphir_type(&function.return_type, package)?),
                body: v4::ValueBody::Expression(body),
            };
            let access = if function.publicity.is_importable() {
                v4::Access::Public
            } else {
                v4::Access::Private
            };
            Ok((
                name,
                AccessControlled {
                    access,
                    value: Documented::new(None, definition),
                },
            ))
        })
        .collect()
}

fn attrs(ty: &Arc<GleamType>, package: &PackageName) -> Result<v4::ValueAttributes, String> {
    Ok(v4::ValueAttributes {
        inferred_type: Some(Box::new(morphir_type(ty, package)?)),
        ..Default::default()
    })
}

fn local_name(package: &PackageName, module: &str, name: &str) -> FQName {
    FQName {
        package_path: package.clone().into(),
        module_path: ModuleName::parse(module).into(),
        local_name: Name::from(name),
    }
}

fn sdk_name(module: &str, name: &str) -> FQName {
    local_name(&PackageName::parse("morphir/SDK"), module, name)
}

fn lower_statements(
    statements: &[TypedStatement],
    package: &PackageName,
    module: &str,
) -> Result<v4::Value, String> {
    match statements {
        [ast::Statement::Expression(expr)] => lower_expr(expr, package, module),
        _ => Err("V3 pilot lowering requires one expression in this function body".into()),
    }
}

fn lower_pattern(
    pattern: &TypedPattern,
    package: &PackageName,
    module: &str,
) -> Result<v4::Pattern, String> {
    let attributes = attrs(&pattern.type_(), package)?;
    Ok(match pattern {
        ast::Pattern::Discard { .. } => v4::Pattern::WildcardPattern(attributes),
        ast::Pattern::Variable { name, .. } => v4::Pattern::AsPattern(
            attributes.clone(),
            Box::new(v4::Pattern::WildcardPattern(attributes)),
            Name::from(name.as_str()),
        ),
        ast::Pattern::Assign { pattern, name, .. } => v4::Pattern::AsPattern(
            attributes,
            Box::new(lower_pattern(pattern, package, module)?),
            Name::from(name.as_str()),
        ),
        ast::Pattern::List { elements, tail, .. } => {
            let tail = match tail {
                Some(tail) => lower_pattern(&tail.pattern, package, module)?,
                None => v4::Pattern::EmptyListPattern(attributes.clone()),
            };
            elements.iter().rev().try_fold(tail, |tail, head| {
                Ok::<_, String>(v4::Pattern::HeadTailPattern(
                    attributes.clone(),
                    Box::new(lower_pattern(head, package, module)?),
                    Box::new(tail),
                ))
            })?
        }
        ast::Pattern::Constructor {
            name,
            constructor,
            arguments,
            ..
        } => {
            if name == "True" || name == "False" {
                v4::Pattern::LiteralPattern(attributes, v4::Literal::Bool(name == "True"))
            } else {
                let source_module = match constructor {
                    gleam_core::analyse::Inferred::Known(known) => known.module.as_str(),
                    gleam_core::analyse::Inferred::Unknown => {
                        return Err(format!("Unresolved constructor pattern '{name}'"));
                    }
                };
                if source_module != module {
                    return Err(format!(
                        "External constructor pattern '{source_module}.{name}' is not supported"
                    ));
                }
                v4::Pattern::ConstructorPattern(
                    attributes,
                    local_name(package, source_module, name),
                    arguments
                        .iter()
                        .map(|argument| lower_pattern(&argument.value, package, module))
                        .collect::<Result<_, _>>()?,
                )
            }
        }
        _ => return Err("Unsupported typed Gleam pattern in V3 lowering".into()),
    })
}

fn lower_expr(expr: &TypedExpr, package: &PackageName, module: &str) -> Result<v4::Value, String> {
    let attributes = attrs(&expr.type_(), package)?;
    Ok(match expr {
        TypedExpr::Int { int_value, .. } => v4::Value::Literal(
            attributes,
            v4::Literal::Integer(
                int_value
                    .to_string()
                    .parse()
                    .map_err(|_| "Cannot convert inferred Gleam integer")?,
            ),
        ),
        TypedExpr::String { value, .. } => {
            v4::Value::Literal(attributes, v4::Literal::String(value.to_string()))
        }
        TypedExpr::Var {
            constructor, name, ..
        } => match &constructor.variant {
            ValueConstructorVariant::LocalVariable { .. } => {
                v4::Value::Variable(attributes, Name::from(name.as_str()))
            }
            ValueConstructorVariant::ModuleFn {
                module: source_module,
                ..
            } => {
                if source_module.as_str() != module {
                    return Err(format!(
                        "External function '{source_module}.{name}' is not supported"
                    ));
                }
                v4::Value::Reference(attributes, local_name(package, source_module, name))
            }
            ValueConstructorVariant::Record {
                module: source_module,
                ..
            } if source_module == "gleam" && (name == "True" || name == "False") => {
                v4::Value::Literal(attributes, v4::Literal::Bool(name == "True"))
            }
            ValueConstructorVariant::Record {
                module: source_module,
                ..
            } => {
                if source_module.as_str() != module {
                    return Err(format!(
                        "External constructor '{source_module}.{name}' is not supported"
                    ));
                }
                v4::Value::Constructor(attributes, local_name(package, source_module, name))
            }
            _ => return Err(format!("Unsupported typed Gleam variable '{name}'")),
        },
        TypedExpr::Call {
            fun,
            arguments,
            type_,
            ..
        } => {
            let mut result = lower_expr(fun, package, module)?;
            let function_type = fun.type_();
            let GleamType::Fn {
                arguments: parameter_types,
                return_,
            } = function_type.as_ref()
            else {
                return Err("Typed Gleam call has no function type".into());
            };
            for (index, argument) in arguments.iter().enumerate() {
                let result_type = if index + 1 == arguments.len() {
                    morphir_type(type_, package)?
                } else {
                    parameter_types
                        .get(index + 1..)
                        .ok_or("Call arity exceeds inferred function type")?
                        .iter()
                        .rev()
                        .try_fold(morphir_type(return_, package)?, |result, argument| {
                            Ok::<_, String>(MorphirType::Function(
                                TypeAttributes::default(),
                                Box::new(morphir_type(argument, package)?),
                                Box::new(result),
                            ))
                        })?
                };
                result = v4::Value::Apply(
                    v4::ValueAttributes {
                        inferred_type: Some(Box::new(result_type)),
                        ..Default::default()
                    },
                    Box::new(result),
                    Box::new(lower_expr(&argument.value, package, module)?),
                );
            }
            result
        }
        TypedExpr::BinOp {
            operator,
            left,
            right,
            ..
        } => {
            let name = match operator {
                BinOp::AddInt => "add",
                BinOp::Eq => "equal",
                _ => return Err(format!("Unsupported Gleam operator '{operator:?}'")),
            };
            let left_type = morphir_type(&left.type_(), package)?;
            let right_type = morphir_type(&right.type_(), package)?;
            let result_type = morphir_type(&expr.type_(), package)?;
            let after_left = MorphirType::Function(
                TypeAttributes::default(),
                Box::new(right_type.clone()),
                Box::new(result_type.clone()),
            );
            let function_type = MorphirType::Function(
                TypeAttributes::default(),
                Box::new(left_type),
                Box::new(after_left.clone()),
            );
            let typed_attrs = |ty| v4::ValueAttributes {
                inferred_type: Some(Box::new(ty)),
                ..Default::default()
            };
            let function =
                v4::Value::Reference(typed_attrs(function_type), sdk_name("basics", name));
            let first = v4::Value::Apply(
                typed_attrs(after_left),
                Box::new(function),
                Box::new(lower_expr(left, package, module)?),
            );
            v4::Value::Apply(
                attributes,
                Box::new(first),
                Box::new(lower_expr(right, package, module)?),
            )
        }
        TypedExpr::Case {
            subjects, clauses, ..
        } => {
            let [subject] = subjects.as_slice() else {
                return Err("V3 pilot supports one case subject".into());
            };
            let cases = clauses
                .iter()
                .map(|clause| -> Result<v4::PatternCase, String> {
                    if clause.guard.is_some() || !clause.alternative_patterns.is_empty() {
                        return Err("Guarded or alternative case clauses are not supported".into());
                    }
                    let [pattern] = clause.pattern.as_slice() else {
                        return Err("V3 pilot supports one case pattern".into());
                    };
                    Ok(v4::PatternCase(
                        lower_pattern(pattern, package, module)?,
                        lower_expr(&clause.then, package, module)?,
                    ))
                })
                .collect::<Result<_, _>>()?;
            v4::Value::PatternMatch(
                attributes,
                Box::new(lower_expr(subject, package, module)?),
                cases,
            )
        }
        TypedExpr::Block { statements, .. } => lower_statements(statements, package, module)?,
        _ => return Err("Unsupported typed Gleam expression in V3 lowering".into()),
    })
}
