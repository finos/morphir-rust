use super::{annotation, reserve};
use crate::{
    Outcome, error, names,
    values::{self, unsupported},
};
use morphir_core::ir::v4::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn render(
    name: &str,
    definition: &ValueDefinition,
    package: &PackageName,
    module: &str,
    types: &BTreeMap<String, String>,
    aliases: &values::TupleAliases,
) -> Outcome<String> {
    let ValueBody::Expression(body) = &definition.body else {
        return Err(unsupported(
            "Only expression function bodies can be generated as Python",
        ));
    };
    let output_type = definition
        .output_type
        .as_ref()
        .ok_or_else(|| unsupported("Function return type is required"))?;
    let output = annotation(output_type, package, module, types)?;
    let mut seen = BTreeSet::new();
    let inputs = definition
        .input_types
        .iter()
        .map(|(name, entry)| {
            if entry
                .type_attributes
                .as_ref()
                .is_some_and(|attrs| *attrs != ValueAttributes::default())
            {
                return Err(unsupported(
                    "Parameter attributes cannot yet be preserved in Python",
                ));
            }
            let parameter_name =
                Name::from_canonical_string(name).map_err(|e| error("PY003", e))?;
            let python = names::field_name(&parameter_name)?;
            reserve(&mut seen, &python)?;
            Ok(format!(
                "{python}: {}",
                annotation(&entry.input_type, package, module, types)?
            ))
        })
        .collect::<Outcome<Vec<_>>>()?;
    values::validate_function(definition, aliases)?;
    let mut source = format!("\ndef {name}({}) -> {output}:\n", inputs.join(", "));
    render_body(&mut source, body, 1)?;
    Ok(source)
}

fn render_body(source: &mut String, body: &Value, depth: usize) -> Outcome<()> {
    let indent = "    ".repeat(depth);
    match body {
        Value::IfThenElse(_, condition, then_branch, else_branch) => {
            source.push_str(&format!("{indent}if {}:\n", expression(condition)?));
            render_body(source, then_branch, depth + 1)?;
            source.push_str(&format!("{indent}else:\n"));
            render_body(source, else_branch, depth + 1)?;
        }
        _ => source.push_str(&format!("{indent}return {}\n", expression(body)?)),
    }
    Ok(())
}

fn expression(value: &Value) -> Outcome<String> {
    match value {
        Value::Variable(_, name) => names::field_name(name),
        Value::Tuple(_, elements) => Ok(format!(
            "({})",
            elements
                .iter()
                .map(expression)
                .collect::<Outcome<Vec<_>>>()?
                .join(", ")
        )),
        Value::Literal(_, Literal::Bool(value)) => Ok(if *value { "True" } else { "False" }.into()),
        Value::Literal(_, Literal::Integer(value)) => Ok(value.to_string()),
        Value::Literal(_, Literal::Float(value)) => Ok(format!("{:?}", value.value())),
        // JSON escaping is also valid in a Python string literal, including control characters.
        Value::Literal(_, Literal::String(value)) => {
            serde_json::to_string(value).map_err(|e| error("PY005", e.to_string()))
        }
        Value::IfThenElse(_, condition, then_branch, else_branch) => Ok(format!(
            "({} if {} else {})",
            expression(then_branch)?,
            expression(condition)?,
            expression(else_branch)?
        )),
        Value::Apply(..) => {
            let (operator, left, right) = values::comparison(value)?;
            Ok(format!(
                "({} {} {})",
                expression(left)?,
                operator.python(),
                expression(right)?
            ))
        }
        _ => Err(unsupported("Unsupported Python function expression")),
    }
}
