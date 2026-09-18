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
    context: &Context<'_>,
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
            let parameter_name =
                Name::from_canonical_string(name).map_err(|e| error("PY003", e))?;
            let python = names::field_name(&parameter_name)?;
            reserve(&mut seen, &python)?;
            Ok(format!(
                "{python}: {}",
                annotation(entry, package, module, types)?
            ))
        })
        .collect::<Outcome<Vec<_>>>()?;
    values::validate_function(definition, aliases, context.signatures)?;
    let mut source = format!("\ndef {name}({}) -> {output}:\n", inputs.join(", "));
    context.render_body(&mut source, body, 1)?;
    Ok(source)
}

pub(super) struct Context<'a> {
    pub(super) names: &'a BTreeMap<String, String>,
    pub(super) signatures: &'a values::Signatures,
}

impl Context<'_> {
    fn render_body(&self, source: &mut String, body: &Value, depth: usize) -> Outcome<()> {
        let indent = "    ".repeat(depth);
        match body {
            Value::IfThenElse(_, condition, then_branch, else_branch) => {
                source.push_str(&format!("{indent}if {}:\n", self.expression(condition)?));
                self.render_body(source, then_branch, depth + 1)?;
                source.push_str(&format!("{indent}else:\n"));
                self.render_body(source, else_branch, depth + 1)?;
            }
            _ => source.push_str(&format!("{indent}return {}\n", self.expression(body)?)),
        }
        Ok(())
    }

    fn expression(&self, value: &Value) -> Outcome<String> {
        match value {
            Value::Lambda(_, Pattern::AsPattern(_, _, name), body) => Ok(format!(
                "(lambda {}: {})",
                names::field_name(name)?,
                self.expression(body)?
            )),
            Value::Reference(_, name) => {
                let signature = &self.signatures[&name.to_canonical_string()];
                let python = self.reference(name)?;
                match signature.inputs.len() {
                    0 => Ok(format!("{python}()")),
                    1 => Ok(python),
                    _ => Err(unsupported(
                        "Only unary named functions can be used as callable values",
                    )),
                }
            }
            Value::Variable(_, name) => names::field_name(name),
            Value::Tuple(_, elements) => Ok(format!(
                "({})",
                elements
                    .iter()
                    .map(|value| self.expression(value))
                    .collect::<Outcome<Vec<_>>>()?
                    .join(", ")
            )),
            Value::Literal(_, Literal::Bool(value)) => {
                Ok(if *value { "True" } else { "False" }.into())
            }
            Value::Literal(_, Literal::Integer(value)) => Ok(value.to_string()),
            Value::Literal(_, Literal::Float(value)) => Ok(format!("{:?}", value.value())),
            // JSON escaping is also valid in a Python string literal, including control characters.
            Value::Literal(_, Literal::String(value)) => {
                serde_json::to_string(value).map_err(|e| error("PY005", e.to_string()))
            }
            Value::IfThenElse(_, condition, then_branch, else_branch) => Ok(format!(
                "({} if {} else {})",
                self.expression(then_branch)?,
                self.expression(condition)?,
                self.expression(else_branch)?
            )),
            Value::Apply(..) => {
                if let Ok((operator, left, right)) = values::comparison(value) {
                    return Ok(format!(
                        "({} {} {})",
                        self.expression(left)?,
                        operator.python(),
                        self.expression(right)?
                    ));
                }
                let mut head = value;
                let mut arguments = vec![];
                while let Value::Apply(_, function, argument) = head {
                    arguments.push(argument.as_ref());
                    head = function;
                }
                arguments.reverse();
                let (mut source, consumed) = if let Value::Reference(_, name) = head {
                    let count = self.signatures[&name.to_canonical_string()].inputs.len();
                    if arguments.len() < count {
                        return Err(unsupported(
                            "Partial applications of multi-parameter functions are not supported",
                        ));
                    }
                    let args = arguments[..count]
                        .iter()
                        .map(|arg| self.expression(arg))
                        .collect::<Outcome<Vec<_>>>()?;
                    (
                        format!("{}({})", self.reference(name)?, args.join(", ")),
                        count,
                    )
                } else {
                    (self.expression(head)?, 0)
                };
                for argument in &arguments[consumed..] {
                    source = format!("{source}({})", self.expression(argument)?);
                }
                Ok(source)
            }
            _ => Err(unsupported("Unsupported Python function expression")),
        }
    }

    fn reference(&self, name: &morphir_core::naming::FQName) -> Outcome<String> {
        self.names
            .get(&name.to_canonical_string())
            .cloned()
            .ok_or_else(|| unsupported("Unknown function reference"))
    }
}
