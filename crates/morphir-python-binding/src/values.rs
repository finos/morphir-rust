//! Validation shared by Python function compilation and generation.

use crate::{Outcome, error};
use morphir_core::{ir::v4::*, naming::FQName};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub(crate) enum Comparison {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

impl Comparison {
    pub(crate) fn python(self) -> &'static str {
        match self {
            Self::Equal => "==",
            Self::NotEqual => "!=",
            Self::Less => "<",
            Self::LessEqual => "<=",
            Self::Greater => ">",
            Self::GreaterEqual => ">=",
        }
    }

    pub(crate) fn reference(self) -> FQName {
        let name = match self {
            Self::Equal => "equal",
            Self::NotEqual => "not-equal",
            Self::Less => "less-than",
            Self::LessEqual => "less-than-or-equal",
            Self::Greater => "greater-than",
            Self::GreaterEqual => "greater-than-or-equal",
        };
        FQName::from_canonical_string(&format!("morphir/SDK:basics#{name}")).expect("SDK name")
    }

    fn from_reference(reference: &FQName) -> Outcome<Self> {
        [
            Self::Equal,
            Self::NotEqual,
            Self::Less,
            Self::LessEqual,
            Self::Greater,
            Self::GreaterEqual,
        ]
        .into_iter()
        .find(|op| op.reference() == *reference)
        .ok_or_else(|| unsupported("Only scalar comparison applications are supported"))
    }

    pub(crate) fn apply(self, left: Value, right: Value) -> Value {
        Value::Apply(
            Default::default(),
            Box::new(Value::Apply(
                Default::default(),
                Box::new(Value::Reference(Default::default(), self.reference())),
                Box::new(left),
            )),
            Box::new(right),
        )
    }
}

pub(crate) fn scalar(name: &str) -> Type {
    let path = if name == "string" { "string" } else { "basics" };
    Type::Reference(
        Default::default(),
        FQName::from_canonical_string(&format!("morphir/SDK:{path}#{name}")).expect("SDK type"),
        vec![],
    )
}

pub(crate) fn unsupported(message: &str) -> morphir_extension_sdk::Diagnostic {
    error("PY004", message)
}

pub(crate) fn require_type(actual: &Type, expected: &Type) -> Outcome<()> {
    if actual != expected {
        return Err(unsupported(
            "Function expression does not match the required type",
        ));
    }
    Ok(())
}

pub(crate) fn literal_type(literal: &Literal) -> Outcome<Type> {
    Ok(scalar(match literal {
        Literal::Bool(_) => "bool",
        Literal::Integer(_) => "int",
        Literal::Float(_) => "float",
        Literal::String(_) => "string",
        _ => {
            return Err(unsupported(
                "Only bool, int, float and str literals are supported",
            ));
        }
    }))
}

pub(crate) fn require_empty_attributes(value: &Value) -> Outcome<()> {
    if *value.attributes() != ValueAttributes::default() {
        return Err(unsupported(
            "Value attributes cannot yet be preserved in Python",
        ));
    }
    Ok(())
}

pub(crate) fn comparison(value: &Value) -> Outcome<(Comparison, &Value, &Value)> {
    let Value::Apply(_, function, right) = value else {
        return Err(unsupported("Expected a comparison"));
    };
    require_empty_attributes(function)?;
    let Value::Apply(_, reference, left) = function.as_ref() else {
        return Err(unsupported("Expected two comparison arguments"));
    };
    require_empty_attributes(reference)?;
    let Value::Reference(_, reference) = reference.as_ref() else {
        return Err(unsupported("Expected an SDK comparison reference"));
    };
    Ok((Comparison::from_reference(reference)?, left, right))
}

pub(crate) fn infer(value: &Value, parameters: &BTreeMap<String, Type>) -> Outcome<Type> {
    require_empty_attributes(value)?;
    match value {
        Value::Literal(_, literal) => literal_type(literal),
        Value::Variable(_, name) => parameters
            .get(&name.to_canonical_string())
            .cloned()
            .ok_or_else(|| unsupported("Function body references an unknown parameter")),
        Value::IfThenElse(_, condition, then_branch, else_branch) => {
            require_type(&infer(condition, parameters)?, &scalar("bool"))?;
            let result = infer(then_branch, parameters)?;
            require_type(&infer(else_branch, parameters)?, &result)?;
            Ok(result)
        }
        Value::Apply(..) => {
            let (operator, left, right) = comparison(value)?;
            let operand_type = infer(left, parameters)?;
            require_type(&infer(right, parameters)?, &operand_type)?;
            let allowed = ["int", "float", "string"]
                .iter()
                .any(|name| operand_type == scalar(name))
                || (matches!(operator, Comparison::Equal | Comparison::NotEqual)
                    && operand_type == scalar("bool"));
            if !allowed {
                return Err(unsupported(
                    "Comparison operands must be matching scalar types; bool supports only equality",
                ));
            }
            Ok(scalar("bool"))
        }
        _ => Err(unsupported(
            "Only parameters, scalar literals, comparisons and conditionals are supported in function bodies",
        )),
    }
}
