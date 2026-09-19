//! Adapt the official Gleam parser's syntax tree to the frontend's lowering input.
//!
//! Syntax, lexing, precedence, comments and escape rules belong to `gleam-core`.
//! Valid syntax outside the lowering subset remains an explicit, located
//! `Expr::Unsupported`, so extracting a module's types does not require lowering
//! every function. Incomplete syntax accepted for upstream editor recovery is an
//! error here, because it cannot describe a complete source module.

#![allow(
    clippy::result_large_err,
    reason = "Preserve the existing public ParseError API across the adapter"
)]

use gleam_core::{ast as gleam, warning::WarningEmitter};

use crate::frontend::ast::{
    Access, BinaryOperator, CaseBranch, Expr, Field, Import, Literal, ModuleIR, Pattern, Span,
    Statement, TypeDef, TypeExpr, ValueDef, Variant,
};
use crate::frontend::errors::{ParseError, from_upstream, located_error};

#[path = "parser_recovery.rs"]
mod recovery;

pub fn parse_gleam(path: &str, source: &str) -> Result<ModuleIR, ParseError> {
    let parsed = gleam_core::parse::parse_module(path.into(), source, &WarningEmitter::null())
        .map_err(|error| from_upstream(path, source, error))?;
    recovery::validate(&parsed.module, source)?;
    let module_doc = parsed
        .extra
        .module_comments
        .iter()
        .map(|span| source[span.start as usize..span.end as usize].trim())
        .collect::<Vec<_>>()
        .join("\n");
    let mut module = ModuleIR {
        name: path.trim_end_matches(".gleam").replace('\\', "/"),
        doc: (!parsed.extra.module_comments.is_empty()).then_some(module_doc),
        types: vec![],
        values: vec![],
        imports: vec![],
    };
    for targeted in &parsed.module.definitions {
        if targeted.target.is_some() {
            return Err(located_error(
                "Target-specific declarations are not supported",
                targeted.definition.location(),
                source,
            ));
        }
        match &targeted.definition {
            gleam::Definition::Import(import) => module.imports.push(Import {
                module: import.module.to_string(),
                alias: import
                    .as_name
                    .as_ref()
                    .map(|(name, _)| name.name().to_string()),
                types: import
                    .unqualified_types
                    .iter()
                    .map(|item| (item.name.to_string(), item.used_name().to_string()))
                    .collect(),
                values: import
                    .unqualified_values
                    .iter()
                    .map(|item| (item.name.to_string(), item.used_name().to_string()))
                    .collect(),
            }),
            gleam::Definition::TypeAlias(alias) => module.types.push(TypeDef {
                name: alias.alias.to_string(),
                params: alias
                    .parameters
                    .iter()
                    .map(|(_, name)| name.to_string())
                    .collect(),
                body: type_expr(&alias.type_ast, source)?,
                doc: documentation(alias.documentation.as_ref().map(|(_, text)| text.as_str())),
                span: span(gleam::SrcSpan::new(
                    alias.location.start,
                    alias.type_ast.location().end,
                )),
                access: access(alias.publicity, alias.location, source)?,
                constructor_access: access(alias.publicity, alias.location, source)?,
            }),
            gleam::Definition::CustomType(custom) => {
                if custom.external_erlang.is_some() || custom.external_javascript.is_some() {
                    return Err(located_error(
                        "External types are not supported",
                        custom.full_location(),
                        source,
                    ));
                }
                let access = access(custom.publicity, custom.location, source)?;
                module.types.push(TypeDef {
                    name: custom.name.to_string(),
                    params: custom
                        .parameters
                        .iter()
                        .map(|(_, name)| name.to_string())
                        .collect(),
                    body: TypeExpr::CustomType {
                        variants: custom
                            .constructors
                            .iter()
                            .map(|constructor| {
                                Ok(Variant {
                                    name: constructor.name.to_string(),
                                    labels: constructor
                                        .arguments
                                        .iter()
                                        .map(|arg| {
                                            arg.label.as_ref().map(|(_, label)| label.to_string())
                                        })
                                        .collect(),
                                    fields: constructor
                                        .arguments
                                        .iter()
                                        .map(|arg| type_expr(&arg.ast, source))
                                        .collect::<Result<_, ParseError>>()?,
                                })
                            })
                            .collect::<Result<_, ParseError>>()?,
                    },
                    doc: documentation(
                        custom.documentation.as_ref().map(|(_, text)| text.as_str()),
                    ),
                    span: span(custom.full_location()),
                    access,
                    constructor_access: if custom.opaque {
                        Access::Private
                    } else {
                        access
                    },
                });
            }
            gleam::Definition::Function(function) => {
                module.values.push(function_definition(function, source)?)
            }
            gleam::Definition::ModuleConstant(constant) => module.values.push(ValueDef {
                span: span(gleam::SrcSpan::new(
                    constant.location.start,
                    constant.value.location().end,
                )),
                params: vec![],
                param_labels: vec![],
                doc: documentation(
                    constant
                        .documentation
                        .as_ref()
                        .map(|(_, text)| text.as_str()),
                ),
                name: constant.name.to_string(),
                type_annotation: constant
                    .annotation
                    .as_ref()
                    .map(|annotation| type_expr(annotation, source))
                    .transpose()?,
                body: match constant.annotation.as_ref() {
                    None => unsupported(
                        "unannotated constant declarations require type inference",
                        gleam::SrcSpan::new(constant.location.start, constant.value.location().end),
                    ),
                    Some(gleam::TypeAst::Fn(_)) => unsupported(
                        "function-typed constant declarations require distinct constant lowering",
                        gleam::SrcSpan::new(constant.location.start, constant.value.location().end),
                    ),
                    Some(_) => constant_expr(&constant.value, source)?,
                },
                access: access(constant.publicity, constant.location, source)?,
            }),
        }
    }
    Ok(module)
}

fn span(location: gleam::SrcSpan) -> Span {
    Span {
        start: location.start as usize,
        end: location.end as usize,
    }
}

fn documentation(text: Option<&str>) -> Option<String> {
    text.map(|text| text.lines().map(str::trim).collect::<Vec<_>>().join("\n"))
}

fn access(
    publicity: gleam::Publicity,
    location: gleam::SrcSpan,
    source: &str,
) -> Result<Access, ParseError> {
    match publicity {
        gleam::Publicity::Public => Ok(Access::Public),
        gleam::Publicity::Private => Ok(Access::Private),
        gleam::Publicity::Internal { .. } => Err(located_error(
            "Internal visibility is not supported",
            location,
            source,
        )),
    }
}

fn type_expr(annotation: &gleam::TypeAst, source: &str) -> Result<TypeExpr, ParseError> {
    Ok(match annotation {
        gleam::TypeAst::Constructor(constructor) => {
            let (module, name) = match &constructor.name {
                gleam::TypeAstConstructorName::Unqualified { name, .. } => (None, name.to_string()),
                gleam::TypeAstConstructorName::Qualified {
                    module,
                    name: Some((name, _)),
                    ..
                } => (Some(module.to_string()), name.to_string()),
                gleam::TypeAstConstructorName::Qualified { name: None, .. } => {
                    return Err(located_error(
                        "Missing qualified type name",
                        constructor.location,
                        source,
                    ));
                }
            };
            TypeExpr::Named {
                module,
                name,
                parameters: constructor
                    .arguments
                    .iter()
                    .map(|arg| type_expr(arg, source))
                    .collect::<Result<_, _>>()?,
            }
        }
        gleam::TypeAst::Fn(function) => TypeExpr::Function {
            parameters: function
                .arguments
                .iter()
                .map(|arg| type_expr(arg, source))
                .collect::<Result<_, _>>()?,
            return_type: Box::new(type_expr(&function.return_, source)?),
        },
        gleam::TypeAst::Var(variable) => TypeExpr::Variable {
            name: variable.name.to_string(),
        },
        gleam::TypeAst::Tuple(tuple) => TypeExpr::Tuple {
            elements: tuple
                .elements
                .iter()
                .map(|element| type_expr(element, source))
                .collect::<Result<_, _>>()?,
        },
        gleam::TypeAst::Hole(hole) => TypeExpr::Hole {
            name: hole.name.to_string(),
        },
    })
}

fn argument_name(argument: &gleam::UntypedArg) -> String {
    match &argument.names {
        gleam::ArgNames::Discard { name, .. }
        | gleam::ArgNames::LabelledDiscard { name, .. }
        | gleam::ArgNames::Named { name, .. }
        | gleam::ArgNames::NamedLabelled { name, .. } => name.to_string(),
    }
}

fn function_definition(
    function: &gleam::UntypedFunction,
    source: &str,
) -> Result<ValueDef, ParseError> {
    let name = function
        .name
        .as_ref()
        .ok_or_else(|| located_error("Missing function name", function.location, source))?;
    Ok(ValueDef {
        span: span(function.full_location()),
        params: function.arguments.iter().map(argument_name).collect(),
        param_labels: function
            .arguments
            .iter()
            .map(|arg| arg.names.get_label().map(ToString::to_string))
            .collect(),
        doc: documentation(
            function
                .documentation
                .as_ref()
                .map(|(_, text)| text.as_str()),
        ),
        name: name.1.to_string(),
        type_annotation: Some(TypeExpr::Function {
            parameters: function
                .arguments
                .iter()
                .map(|arg| optional_annotation(arg.annotation.as_ref(), source))
                .collect::<Result<_, _>>()?,
            return_type: Box::new(optional_annotation(
                function.return_annotation.as_ref(),
                source,
            )?),
        }),
        body: if function.external_erlang.is_some() || function.external_javascript.is_some() {
            unsupported("external function bindings", function.full_location())
        } else {
            block(&function.body, source)?
        },
        access: access(function.publicity, function.location, source)?,
    })
}

fn optional_annotation(
    annotation: Option<&gleam::TypeAst>,
    source: &str,
) -> Result<TypeExpr, ParseError> {
    annotation
        .map(|annotation| type_expr(annotation, source))
        .transpose()
        .map(|annotation| annotation.unwrap_or(TypeExpr::Hole { name: "_".into() }))
}

fn unsupported(feature: &str, location: gleam::SrcSpan) -> Expr {
    Expr::Unsupported {
        feature: feature.into(),
        span: span(location),
    }
}

fn block(statements: &[gleam::UntypedStatement], source: &str) -> Result<Expr, ParseError> {
    Ok(Expr::Block {
        statements: statements
            .iter()
            .map(|statement| {
                Ok(match statement {
                    gleam::Statement::Expression(value) => {
                        Statement::Expression(expression(value, source)?)
                    }
                    gleam::Statement::Assignment(assignment) => {
                        if !matches!(assignment.kind, gleam::AssignmentKind::Let) {
                            Statement::Expression(unsupported(
                                "assert assignments",
                                assignment.location,
                            ))
                        } else {
                            match pattern(&assignment.pattern) {
                                Ok(pattern) => Statement::Assignment {
                                    pattern,
                                    annotation: assignment
                                        .annotation
                                        .as_ref()
                                        .map(|annotation| type_expr(annotation, source))
                                        .transpose()?,
                                    value: Box::new(expression(&assignment.value, source)?),
                                },
                                Err((feature, location)) => {
                                    Statement::Expression(unsupported(feature, location))
                                }
                            }
                        }
                    }
                    gleam::Statement::Use(use_) => {
                        Statement::Expression(unsupported("use expressions", use_.location))
                    }
                    gleam::Statement::Assert(assertion) => {
                        Statement::Expression(unsupported("boolean assertions", assertion.location))
                    }
                })
            })
            .collect::<Result<_, ParseError>>()?,
    })
}

fn expression(value: &gleam::UntypedExpr, source: &str) -> Result<Expr, ParseError> {
    use gleam::UntypedExpr as G;
    Ok(match value {
        G::Int {
            int_value,
            location,
            ..
        } => match int_value.to_string().parse() {
            Ok(value) => Expr::Literal {
                value: Literal::Int { value },
            },
            Err(_) => unsupported(
                "integer literals outside the signed 64-bit range",
                *location,
            ),
        },
        G::Float {
            float_value,
            location,
            ..
        } => float_literal(float_value.value(), *location),
        G::String { value, .. } => Expr::Literal {
            value: Literal::String {
                value: gleam_core::strings::convert_string_escape_chars(value).to_string(),
            },
        },
        G::Var { name, .. } => variable_or_constructor(None, name),
        G::Block { statements, .. } => block(statements, source)?,
        G::Fn {
            arguments,
            body,
            kind,
            location,
            ..
        } => {
            if matches!(kind, gleam::FunctionLiteralKind::Capture { .. }) {
                unsupported("function captures", *location)
            } else {
                Expr::Lambda {
                    params: arguments.iter().map(argument_name).collect(),
                    body: Box::new(block(body, source)?),
                }
            }
        }
        G::List { elements, tail, .. } => Expr::List {
            elements: elements
                .iter()
                .map(|element| expression(element, source))
                .collect::<Result<_, _>>()?,
            tail: tail
                .as_deref()
                .map(|tail| expression(tail, source).map(Box::new))
                .transpose()?,
        },
        G::Call { fun, arguments, .. } => Expr::Apply {
            function: Box::new(expression(fun, source)?),
            arguments: arguments
                .iter()
                .map(|arg| {
                    expression(&arg.value, source).map(|value| field(arg.label.as_deref(), value))
                })
                .collect::<Result<_, _>>()?,
        },
        G::BinOp {
            operator,
            left,
            right,
            ..
        } => Expr::BinaryOp {
            op: binary_operator(*operator),
            left: Box::new(expression(left, source)?),
            right: Box::new(expression(right, source)?),
        },
        G::PipeLine { expressions } => {
            let mut expressions = expressions.iter();
            let first = expressions.next().expect("upstream pipeline is nonempty");
            expressions.try_fold(expression(first, source)?, |left, right| {
                Ok::<_, ParseError>(Expr::BinaryOp {
                    op: BinaryOperator::Pipe,
                    left: Box::new(left),
                    right: Box::new(expression(right, source)?),
                })
            })?
        }
        G::Case {
            subjects,
            clauses,
            location,
        } => {
            let clauses = clauses
                .as_ref()
                .ok_or_else(|| located_error("Missing case expression body", *location, source))?;
            if subjects.len() != 1 || clauses.iter().any(|clause| clause.pattern.len() != 1) {
                unsupported("case expressions with multiple subjects", *location)
            } else if clauses.iter().any(|clause| clause.guard.is_some()) {
                unsupported("case guards", *location)
            } else {
                match case_branches(clauses, source)? {
                    Ok(clauses) => Expr::Case {
                        subjects: subjects
                            .iter()
                            .map(|subject| expression(subject, source))
                            .collect::<Result<_, _>>()?,
                        clauses,
                    },
                    Err((feature, location)) => unsupported(feature, location),
                }
            }
        }
        G::FieldAccess {
            container, label, ..
        } => {
            if label.is_empty() {
                return Err(located_error(
                    "Missing field name",
                    value.location(),
                    source,
                ));
            }
            if label.chars().next().is_some_and(char::is_uppercase) {
                match container.as_ref() {
                    G::Var { name, .. } => variable_or_constructor(Some(name.to_string()), label),
                    _ => {
                        return Err(located_error(
                            "Invalid qualified constructor",
                            value.location(),
                            source,
                        ));
                    }
                }
            } else {
                Expr::FieldAccess {
                    container: Box::new(expression(container, source)?),
                    label: label.to_string(),
                }
            }
        }
        G::Tuple { elements, .. } => Expr::Tuple {
            elements: elements
                .iter()
                .map(|element| expression(element, source))
                .collect::<Result<_, _>>()?,
        },
        G::TupleIndex { tuple, index, .. } => Expr::TupleIndex {
            tuple: Box::new(expression(tuple, source)?),
            index: *index,
        },
        G::Todo {
            kind: gleam::TodoKind::IncompleteUse,
            location,
            ..
        } => {
            return Err(located_error(
                "Incomplete use expression",
                *location,
                source,
            ));
        }
        G::Todo { location, .. } => unsupported("todo expressions", *location),
        G::Panic { location, .. } => unsupported("panic expressions", *location),
        G::Echo { location, .. } => unsupported("echo expressions", *location),
        G::BitArray { location, .. } => unsupported("bit array expressions", *location),
        G::RecordUpdate { location, .. } => unsupported("record updates", *location),
        G::NegateBool { value, .. } => Expr::NegateBool {
            value: Box::new(expression(value, source)?),
        },
        G::NegateInt { value, .. } => Expr::NegateInt {
            value: Box::new(expression(value, source)?),
        },
    })
}

fn float_literal(value: f64, location: gleam::SrcSpan) -> Expr {
    if value.is_finite() {
        Expr::Literal {
            value: Literal::Float { value },
        }
    } else {
        unsupported("non-finite floating point literals", location)
    }
}

fn variable_or_constructor(module: Option<String>, name: &str) -> Expr {
    match (module.as_deref(), name) {
        (None, "True") => Expr::Literal {
            value: Literal::Bool { value: true },
        },
        (None, "False") => Expr::Literal {
            value: Literal::Bool { value: false },
        },
        _ if name.chars().next().is_some_and(char::is_uppercase) => Expr::Constructor {
            module,
            name: name.into(),
        },
        _ => match module {
            None => Expr::Variable { name: name.into() },
            Some(module) => Expr::FieldAccess {
                container: Box::new(Expr::Variable { name: module }),
                label: name.into(),
            },
        },
    }
}

fn field<T>(label: Option<&str>, item: T) -> Field<T> {
    match label {
        Some(label) => Field::Labelled {
            label: label.into(),
            item,
        },
        None => Field::Unlabelled { item },
    }
}

type UnsupportedPattern = (&'static str, gleam::SrcSpan);

fn case_branches(
    clauses: &[gleam::UntypedClause],
    source: &str,
) -> Result<Result<Vec<CaseBranch>, UnsupportedPattern>, ParseError> {
    let mut branches = vec![];
    for clause in clauses {
        let body = expression(&clause.then, source)?;
        for patterns in std::iter::once(&clause.pattern).chain(&clause.alternative_patterns) {
            let [single] = patterns.as_slice() else {
                return Ok(Err((
                    "case expressions with multiple subjects",
                    clause.location,
                )));
            };
            match pattern(single) {
                Ok(pattern) => branches.push(CaseBranch {
                    pattern,
                    body: body.clone(),
                }),
                Err(unsupported) => return Ok(Err(unsupported)),
            }
        }
    }
    Ok(Ok(branches))
}

fn pattern(value: &gleam::UntypedPattern) -> Result<Pattern, UnsupportedPattern> {
    use gleam::Pattern as G;
    Ok(match value {
        G::Int {
            int_value,
            location,
            ..
        } => Pattern::Literal {
            value: Literal::Int {
                value: int_value.to_string().parse().map_err(|_| {
                    (
                        "integer patterns outside the signed 64-bit range",
                        *location,
                    )
                })?,
            },
        },
        G::Float {
            float_value,
            location,
            ..
        } => {
            let value = float_value.value();
            if !value.is_finite() {
                return Err(("non-finite float patterns", *location));
            }
            Pattern::Literal {
                value: Literal::Float { value },
            }
        }
        G::String { value, .. } => Pattern::Literal {
            value: Literal::String {
                value: gleam_core::strings::convert_string_escape_chars(value).to_string(),
            },
        },
        G::Variable { name, .. } => Pattern::Variable {
            name: name.to_string(),
        },
        G::Discard { name, .. } if name == "_" => Pattern::Wildcard,
        G::Discard { name, .. } => Pattern::Discard {
            name: name.to_string(),
        },
        G::Assign {
            name,
            pattern: inner,
            ..
        } => Pattern::Assignment {
            pattern: Box::new(pattern(inner)?),
            name: name.to_string(),
        },
        G::List { elements, tail, .. } => Pattern::List {
            elements: elements.iter().map(pattern).collect::<Result<_, _>>()?,
            tail: tail
                .as_ref()
                .map(|tail| pattern(&tail.pattern).map(Box::new))
                .transpose()?,
        },
        G::Constructor {
            name,
            module,
            arguments,
            spread,
            ..
        } => {
            if module.is_none()
                && arguments.is_empty()
                && spread.is_none()
                && (name == "True" || name == "False")
            {
                Pattern::Literal {
                    value: Literal::Bool {
                        value: name == "True",
                    },
                }
            } else {
                Pattern::Constructor {
                    module: module.as_ref().map(|(module, _)| module.to_string()),
                    name: name.to_string(),
                    arguments: arguments
                        .iter()
                        .map(|arg| {
                            pattern(&arg.value).map(|value| field(arg.label.as_deref(), value))
                        })
                        .collect::<Result<_, _>>()?,
                    with_spread: spread.is_some(),
                }
            }
        }
        G::Tuple { elements, .. } => Pattern::Tuple {
            elements: elements.iter().map(pattern).collect::<Result<_, _>>()?,
        },
        G::StringPrefix { location, .. } => return Err(("string prefix patterns", *location)),
        G::BitArray { location, .. } => return Err(("bit array patterns", *location)),
        G::BitArraySize(size) => return Err(("bit array size patterns", size.location())),
        G::Invalid { location, .. } => return Err(("invalid patterns", *location)),
    })
}

fn constant_expr(value: &gleam::UntypedConstant, source: &str) -> Result<Expr, ParseError> {
    use gleam::Constant as G;
    Ok(match value {
        G::Int {
            int_value,
            location,
            ..
        } => match int_value.to_string().parse() {
            Ok(value) => Expr::Literal {
                value: Literal::Int { value },
            },
            Err(_) => unsupported(
                "integer literals outside the signed 64-bit range",
                *location,
            ),
        },
        G::Float {
            float_value,
            location,
            ..
        } => float_literal(float_value.value(), *location),
        G::String { value, .. } => Expr::Literal {
            value: Literal::String {
                value: gleam_core::strings::convert_string_escape_chars(value).to_string(),
            },
        },
        G::Tuple { elements, .. } => Expr::Tuple {
            elements: elements
                .iter()
                .map(|element| constant_expr(element, source))
                .collect::<Result<_, _>>()?,
        },
        G::List {
            tail: Some(_),
            location,
            ..
        } => unsupported("constant list tails", *location),
        G::List { elements, .. } => Expr::List {
            elements: elements
                .iter()
                .map(|element| constant_expr(element, source))
                .collect::<Result<_, _>>()?,
            tail: None,
        },
        G::Record {
            module,
            name,
            arguments,
            ..
        } if module.is_none()
            && arguments.is_none()
            && matches!(name.as_str(), "True" | "False") =>
        {
            variable_or_constructor(None, name)
        }
        G::Record { location, .. } => unsupported("constant constructor resolution", *location),
        G::Var { location, .. } => unsupported("constant value references", *location),
        G::StringConcatenation { location, .. } => {
            unsupported("constant string concatenation", *location)
        }
        G::RecordUpdate { location, .. } => unsupported("constant record updates", *location),
        G::BitArray { location, .. } => unsupported("constant bit arrays", *location),
        G::Todo { location, .. } => unsupported("todo constants", *location),
        G::Invalid { location, .. } => {
            return Err(located_error(
                "Invalid constant expression",
                *location,
                source,
            ));
        }
    })
}

fn binary_operator(operator: gleam::BinOp) -> BinaryOperator {
    match operator {
        gleam::BinOp::And => BinaryOperator::And,
        gleam::BinOp::Or => BinaryOperator::Or,
        gleam::BinOp::Eq => BinaryOperator::Eq,
        gleam::BinOp::NotEq => BinaryOperator::NotEq,
        gleam::BinOp::LtInt => BinaryOperator::LtInt,
        gleam::BinOp::LtEqInt => BinaryOperator::LtEqInt,
        gleam::BinOp::GtInt => BinaryOperator::GtInt,
        gleam::BinOp::GtEqInt => BinaryOperator::GtEqInt,
        gleam::BinOp::LtFloat => BinaryOperator::LtFloat,
        gleam::BinOp::LtEqFloat => BinaryOperator::LtEqFloat,
        gleam::BinOp::GtFloat => BinaryOperator::GtFloat,
        gleam::BinOp::GtEqFloat => BinaryOperator::GtEqFloat,
        gleam::BinOp::AddInt => BinaryOperator::AddInt,
        gleam::BinOp::SubInt => BinaryOperator::SubInt,
        gleam::BinOp::MultInt => BinaryOperator::MultInt,
        gleam::BinOp::DivInt => BinaryOperator::DivInt,
        gleam::BinOp::RemainderInt => BinaryOperator::RemainderInt,
        gleam::BinOp::AddFloat => BinaryOperator::AddFloat,
        gleam::BinOp::SubFloat => BinaryOperator::SubFloat,
        gleam::BinOp::MultFloat => BinaryOperator::MultFloat,
        gleam::BinOp::DivFloat => BinaryOperator::DivFloat,
        gleam::BinOp::Concatenate => BinaryOperator::Concatenate,
    }
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod tests;
