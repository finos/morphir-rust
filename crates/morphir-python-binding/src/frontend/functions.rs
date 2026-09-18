use super::{annotation, declare};
use crate::{
    Outcome, names,
    values::{self, Comparison, unsupported},
};
use morphir_core::ir::v4::*;
use num_bigint::BigInt;
use ruff_python_ast::{CmpOp, Expr, Number, Stmt, StmtFunctionDef, UnaryOp};
use std::collections::BTreeSet;

pub(super) fn signature(
    function: &StmtFunctionDef,
    types: &super::TypeScope,
) -> Outcome<ValueSpecification> {
    let parameters = &function.parameters;
    if function.is_async
        || function.type_params.is_some()
        || !function.decorator_list.is_empty()
        || !parameters.posonlyargs.is_empty()
        || !parameters.kwonlyargs.is_empty()
        || parameters.vararg.is_some()
        || parameters.kwarg.is_some()
    {
        return Err(unsupported(
            "Expected a synchronous, non-generic function with ordinary annotated parameters and no decorators",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut source_names = BTreeSet::new();
    let input_types = parameters
        .args
        .iter()
        .map(|parameter| {
            if parameter.default.is_some() {
                return Err(unsupported("Parameter defaults are not supported"));
            }
            let source_name = parameter.parameter.name.as_str();
            declare(&mut seen, source_name)?;
            let name = names::identifier(source_name)?;
            names::field_name(&name)?;
            source_names.insert(source_name);
            let tpe = annotation(
                parameter.annotation().ok_or_else(|| {
                    unsupported("Every function parameter needs a type annotation")
                })?,
                types,
            )?;
            Ok((name.to_canonical_string(), tpe))
        })
        .collect::<Outcome<_>>()?;
    let output_type = annotation(
        function
            .returns
            .as_deref()
            .ok_or_else(|| unsupported("Functions need a return type annotation"))?,
        types,
    )?;
    Ok(ValueSpecification {
        annotations: vec![],
        inputs: input_types,
        output: output_type,
    })
}

pub(super) fn lower(
    function: &StmtFunctionDef,
    types: &super::TypeScope,
    aliases: &values::TupleAliases,
    signatures: &values::Signatures,
) -> Outcome<ValueDefinition> {
    let signature = signature(function, types)?;
    let source_names = function
        .parameters
        .args
        .iter()
        .map(|p| p.parameter.name.as_str())
        .collect();
    let context = Context { types, signatures };
    let body = context.block(&function.body, None, &source_names)?;
    let definition = ValueDefinition {
        input_types: signature.inputs,
        output_type: Some(signature.output),
        body: ValueBody::Expression(body),
    };
    values::validate_function(&definition, aliases, signatures)?;
    Ok(definition)
}

struct Context<'a> {
    types: &'a super::TypeScope,
    signatures: &'a values::Signatures,
}

impl Context<'_> {
    // The continuation is used only by paths that reach the end of a branch.
    fn block(
        &self,
        statements: &[Stmt],
        continuation: Option<&Value>,
        parameters: &BTreeSet<&str>,
    ) -> Outcome<Value> {
        let Some((first, rest)) = statements.split_first() else {
            return continuation
                .cloned()
                .ok_or_else(|| unsupported("Every function path must return a value"));
        };
        match first {
            Stmt::Return(statement) if rest.is_empty() => self.expression(
                statement
                    .value
                    .as_deref()
                    .ok_or_else(|| unsupported("Bare return is not supported"))?,
                parameters,
            ),
            Stmt::If(statement) => {
                let fallback = if rest.is_empty() {
                    continuation.cloned()
                } else {
                    Some(self.block(rest, continuation, parameters)?)
                };
                let mut otherwise = fallback.clone();
                for clause in statement.elif_else_clauses.iter().rev() {
                    let branch = self.block(&clause.body, fallback.as_ref(), parameters)?;
                    otherwise = Some(match &clause.test {
                        Some(test) => conditional(
                            self.expression(test, parameters)?,
                            branch,
                            otherwise.ok_or_else(|| {
                                unsupported("Every function path must return a value")
                            })?,
                        ),
                        None => branch,
                    });
                }
                Ok(conditional(
                    self.expression(&statement.test, parameters)?,
                    self.block(&statement.body, fallback.as_ref(), parameters)?,
                    otherwise
                        .ok_or_else(|| unsupported("Every function path must return a value"))?,
                ))
            }
            _ => Err(unsupported(
                "Function bodies may contain only returns and if/elif/else statements; unreachable statements are not supported",
            )),
        }
    }

    fn expression(&self, expr: &Expr, parameters: &BTreeSet<&str>) -> Outcome<Value> {
        let literal = match expr {
            Expr::Lambda(lambda) => {
                let p = lambda
                    .parameters
                    .as_ref()
                    .ok_or_else(|| unsupported("A lambda requires one parameter"))?;
                if p.args.len() != 1
                    || !p.posonlyargs.is_empty()
                    || !p.kwonlyargs.is_empty()
                    || p.vararg.is_some()
                    || p.kwarg.is_some()
                    || p.args[0].default.is_some()
                {
                    return Err(unsupported(
                        "Lambdas require exactly one ordinary parameter without a default",
                    ));
                }
                let source = p.args[0].parameter.name.as_str();
                let name = names::identifier(source)?;
                names::field_name(&name)?;
                if parameters.iter().any(|outer| {
                    *outer != source && names::identifier(outer).is_ok_and(|outer| outer == name)
                }) {
                    return Err(unsupported(
                        "Lambda parameter spelling collides with an enclosing parameter after name normalization",
                    ));
                }
                let mut scope = parameters.clone();
                scope.insert(source);
                return Ok(Value::Lambda(
                    Default::default(),
                    Pattern::AsPattern(
                        Default::default(),
                        Box::new(Pattern::WildcardPattern(Default::default())),
                        name,
                    ),
                    Box::new(self.expression(&lambda.body, &scope)?),
                ));
            }
            Expr::Call(call) => {
                if !call.arguments.keywords.is_empty()
                    || call
                        .arguments
                        .args
                        .iter()
                        .any(|arg| matches!(arg, Expr::Starred(_)))
                {
                    return Err(unsupported(
                        "Only positional calls without starred arguments are supported",
                    ));
                }
                let function =
                    if let Some((name, signature)) = self.reference(&call.func, parameters) {
                        if call.arguments.args.len() != signature.inputs.len() {
                            return Err(unsupported(
                                "Named function calls must supply every declared parameter",
                            ));
                        }
                        Value::Reference(Default::default(), name.clone())
                    } else {
                        if call.arguments.args.len() != 1 {
                            return Err(unsupported("Callable values take exactly one argument"));
                        }
                        self.expression(&call.func, parameters)?
                    };
                return call
                    .arguments
                    .args
                    .iter()
                    .try_fold(function, |function, arg| {
                        Ok(Value::Apply(
                            Default::default(),
                            Box::new(function),
                            Box::new(self.expression(arg, parameters)?),
                        ))
                    });
            }
            Expr::Name(_) | Expr::Attribute(_) if self.reference(expr, parameters).is_some() => {
                let (name, signature) = self.reference(expr, parameters).unwrap();
                if signature.inputs.len() != 1 {
                    return Err(unsupported(
                        "Only unary named functions can be used as callable values",
                    ));
                }
                return Ok(Value::Reference(Default::default(), name.clone()));
            }
            Expr::Tuple(tuple) if tuple.elts.len() >= 2 => {
                return Ok(Value::Tuple(
                    Default::default(),
                    tuple
                        .elts
                        .iter()
                        .map(|element| self.expression(element, parameters))
                        .collect::<Outcome<_>>()?,
                ));
            }
            Expr::Name(name) if parameters.contains(name.id.as_str()) => {
                return Ok(Value::Variable(
                    Default::default(),
                    names::identifier(name.id.as_str())?,
                ));
            }
            Expr::If(expression_if) => {
                return Ok(conditional(
                    self.expression(&expression_if.test, parameters)?,
                    self.expression(&expression_if.body, parameters)?,
                    self.expression(&expression_if.orelse, parameters)?,
                ));
            }
            Expr::Compare(compare) if compare.ops.len() == 1 && compare.operands.len() == 2 => {
                let operator = match compare.ops[0] {
                    CmpOp::Eq => Comparison::Equal,
                    CmpOp::NotEq => Comparison::NotEqual,
                    CmpOp::Lt => Comparison::Less,
                    CmpOp::LtE => Comparison::LessEqual,
                    CmpOp::Gt => Comparison::Greater,
                    CmpOp::GtE => Comparison::GreaterEqual,
                    _ => {
                        return Err(unsupported(
                            "Identity and membership comparisons are not supported",
                        ));
                    }
                };
                return Ok(operator.apply(
                    self.expression(&compare.operands[0], parameters)?,
                    self.expression(&compare.operands[1], parameters)?,
                ));
            }
            Expr::BooleanLiteral(value) => Literal::Bool(value.value),
            Expr::StringLiteral(value) => Literal::String(value.value.to_string()),
            Expr::NumberLiteral(value) => number(&value.value, false)?,
            Expr::UnaryOp(unary) if unary.op == UnaryOp::USub => {
                let Expr::NumberLiteral(value) = unary.operand.as_ref() else {
                    return Err(unsupported(
                        "Unary minus is supported only for numeric literals",
                    ));
                };
                number(&value.value, true)?
            }
            _ => {
                return Err(unsupported(
                    "Expected a parameter, scalar literal, fixed tuple, comparison or conditional expression",
                ));
            }
        };
        Ok(Value::Literal(Default::default(), literal))
    }

    fn reference(
        &self,
        expr: &Expr,
        parameters: &BTreeSet<&str>,
    ) -> Option<(&morphir_core::naming::FQName, &ValueSpecification)> {
        let name = super::imports::qualified_name(expr).ok()?;
        if parameters.contains(name.split('.').next()?) {
            return None;
        }
        let super::Symbol::Function(fq) = self.types.get(&name)? else {
            return None;
        };
        Some((fq, self.signatures.get(&fq.to_canonical_string())?))
    }
}

fn conditional(condition: Value, then_branch: Value, else_branch: Value) -> Value {
    Value::IfThenElse(
        Default::default(),
        Box::new(condition),
        Box::new(then_branch),
        Box::new(else_branch),
    )
}

fn number(value: &Number, negative: bool) -> Outcome<Literal> {
    match value {
        Number::Int(value) => {
            // Ruff retains the original token for integers larger than u64.
            let token = value.to_string().replace('_', "").to_ascii_lowercase();
            let (digits, radix) = match token.get(..2) {
                Some("0x") => (&token[2..], 16),
                Some("0o") => (&token[2..], 8),
                Some("0b") => (&token[2..], 2),
                _ => (token.as_str(), 10),
            };
            let magnitude = BigInt::parse_bytes(digits.as_bytes(), radix)
                .ok_or_else(|| unsupported("Invalid integer literal"))?;
            Ok(Literal::Integer(if negative {
                -magnitude
            } else {
                magnitude
            }))
        }
        Number::Float(value) if value.is_finite() => {
            Ok(Literal::Float(FloatLiteral::from_f64(if negative {
                -*value
            } else {
                *value
            })))
        }
        _ => Err(unsupported(
            "Only finite real numeric literals are supported",
        )),
    }
}
