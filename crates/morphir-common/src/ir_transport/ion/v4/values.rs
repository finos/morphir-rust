//! Ion spelling of v4 value expressions, patterns, and value definitions and specifications.
//!
//! A value is an S-expression whose head names the node, or a shorthand: a bare symbol is a
//! variable, a bare bool or int is a literal, a bare float is a float literal, and `()` is unit.
//! In pattern position a bare symbol binds a name over a wildcard, `_` is a wildcard, `[]` is the
//! empty list and `()` is unit. An integer that does not fit an Ion int is `(int "digits")`.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use ion_rs::Element;
use morphir_core::ir::v4::{self, ValueAttributes};
use morphir_core::naming::{FQName, Name};

use super::annotations::{read_annotations, refuse_on_definition, with_annotations};
use super::attributes::{read_value_attributes, value_attributes};
use super::json::{from_ion, to_ion};
use super::types::{
    read_hole_reason, read_incompleteness, read_type, write_hole_reason, write_incompleteness,
    write_type,
};
use super::{access_of, access_symbol, fq_name, list, local_name, member, optional_doc, unwritten};
use crate::ir_transport::TransportDiagnostic;
use crate::ir_transport::ion::{
    annotation_names, optional_string, required_field, required_string, struct_fields,
};

type ValueDef = v4::AccessControlled<v4::Documented<v4::ValueDefinition>>;
type ValueSpec = v4::Documented<v4::ValueSpecification>;

fn attrs() -> ValueAttributes {
    ValueAttributes::default()
}

// =============================================================================
// Value expressions
// =============================================================================

pub(super) fn read_value(element: &Element) -> Result<v4::Value, TransportDiagnostic> {
    if !annotation_names(element)?.is_empty() {
        return Err(member("a v4 value carries no Ion annotation"));
    }
    if let Some(text) = symbol(element) {
        return Ok(v4::Value::Variable(attrs(), local_name(text)?));
    }
    if let Some(literal) = bare_literal(element)? {
        return Ok(v4::Value::Literal(attrs(), literal));
    }
    let (head, a, rest) = sexp_parts(element)?;
    let Some(head) = head else {
        return Ok(v4::Value::Unit(a));
    };
    let arity = |count: usize| expect_arity(head, &rest, count);
    match head {
        "unit" => {
            arity(0)?;
            Ok(v4::Value::Unit(a.clone()))
        }
        "variable" => {
            arity(1)?;
            Ok(v4::Value::Variable(a.clone(), name_of(rest[0])?))
        }
        "ref" => {
            arity(1)?;
            Ok(v4::Value::Reference(a.clone(), fq_of(rest[0])?))
        }
        "constructor" => {
            arity(1)?;
            Ok(v4::Value::Constructor(a.clone(), fq_of(rest[0])?))
        }
        "apply" => {
            arity(2)?;
            Ok(v4::Value::Apply(
                a.clone(),
                Box::new(read_value(rest[0])?),
                Box::new(read_value(rest[1])?),
            ))
        }
        "lambda" => {
            arity(2)?;
            Ok(v4::Value::Lambda(
                a.clone(),
                read_lambda_pattern(rest[0])?,
                Box::new(read_value(rest[1])?),
            ))
        }
        "if" => {
            arity(3)?;
            Ok(v4::Value::IfThenElse(
                a.clone(),
                Box::new(read_value(rest[0])?),
                Box::new(read_value(rest[1])?),
                Box::new(read_value(rest[2])?),
            ))
        }
        "let" => {
            arity(3)?;
            Ok(v4::Value::LetDefinition(
                a.clone(),
                name_of(rest[0])?,
                Box::new(read_binding(rest[1])?),
                Box::new(read_value(rest[2])?),
            ))
        }
        "letrec" => {
            arity(2)?;
            let bindings = rest[0]
                .as_list()
                .ok_or_else(|| member("letrec bindings are a list"))?;
            let mut read: Vec<v4::LetBinding> = Vec::new();
            for binding in bindings.iter() {
                let pair = pair(binding, "a letrec binding is a name and a definition")?;
                let name = name_of(pair[0])?;
                if read
                    .iter()
                    .any(|v4::LetBinding(existing, _)| *existing == name)
                {
                    return Err(member(format!(
                        "letrec binds '{}' twice",
                        name.to_canonical_string()
                    )));
                }
                read.push(v4::LetBinding(name, read_binding(pair[1])?));
            }
            Ok(v4::Value::LetRecursion(
                a.clone(),
                read,
                Box::new(read_value(rest[1])?),
            ))
        }
        "match" => {
            if rest.is_empty() {
                return Err(member("match has a scrutinee"));
            }
            let mut cases = Vec::new();
            for case in &rest[1..] {
                let pair = pair(case, "a match case is a pattern and a body")?;
                cases.push(v4::PatternCase(
                    read_pattern(pair[0])?,
                    read_value(pair[1])?,
                ));
            }
            Ok(v4::Value::PatternMatch(
                a.clone(),
                Box::new(read_value(rest[0])?),
                cases,
            ))
        }
        "destructure" => {
            arity(3)?;
            Ok(v4::Value::Destructure(
                a.clone(),
                read_pattern(rest[0])?,
                Box::new(read_value(rest[1])?),
                Box::new(read_value(rest[2])?),
            ))
        }
        "field" => {
            arity(2)?;
            Ok(v4::Value::Field(
                a.clone(),
                Box::new(read_value(rest[0])?),
                name_of(rest[1])?,
            ))
        }
        "fieldFunction" => {
            arity(1)?;
            Ok(v4::Value::FieldFunction(a.clone(), name_of(rest[0])?))
        }
        "record" => Ok(v4::Value::Record(a.clone(), read_entries(&rest)?)),
        "update" => {
            if rest.is_empty() {
                return Err(member("update has a record"));
            }
            Ok(v4::Value::UpdateRecord(
                a.clone(),
                Box::new(read_value(rest[0])?),
                read_entries(&rest[1..])?,
            ))
        }
        "tuple" => Ok(v4::Value::Tuple(a.clone(), read_values(&rest)?)),
        "list" => Ok(v4::Value::List(a.clone(), read_values(&rest)?)),
        "hole" => {
            if rest.is_empty() || rest.len() > 2 {
                return Err(member("hole has a reason and an optional expected type"));
            }
            Ok(v4::Value::Hole(
                a.clone(),
                read_hole_reason(rest[0])?,
                rest.get(1)
                    .map(|ty| read_type(ty).map(Box::new))
                    .transpose()?,
            ))
        }
        "int" | "float" | "bool" | "string" | "char" | "decimal" => {
            arity(1)?;
            Ok(v4::Value::Literal(a.clone(), literal(head, rest[0])?))
        }
        "document" => {
            arity(1)?;
            Ok(v4::Value::Literal(
                a.clone(),
                v4::Literal::Document(from_ion(rest[0])?),
            ))
        }
        other => Err(member(format!("unknown v4 value head '{other}'"))),
    }
}

pub(super) fn write_value(value: &v4::Value) -> Result<Element, TransportDiagnostic> {
    let extra = value_attributes(value.attributes())?;
    let head = |name: &str, rest: Vec<Element>| head(name, extra.clone(), rest);
    Ok(match value {
        v4::Value::Literal(_, literal) => write_literal(literal, extra.clone())?,
        v4::Value::Unit(_) if extra.is_none() => sexp(Vec::new()),
        v4::Value::Unit(_) => head("unit", Vec::new()),
        v4::Value::Variable(_, name) if extra.is_none() => name_symbol(name),
        v4::Value::Variable(_, name) => head("variable", vec![name_symbol(name)]),
        v4::Value::Reference(_, name) => head("ref", vec![fq_symbol(name)]),
        v4::Value::Constructor(_, name) => head("constructor", vec![fq_symbol(name)]),
        v4::Value::Apply(_, function, argument) => head(
            "apply",
            vec![write_value(function)?, write_value(argument)?],
        ),
        v4::Value::Lambda(_, pattern, body) => {
            head("lambda", vec![write_pattern(pattern)?, write_value(body)?])
        }
        v4::Value::IfThenElse(_, condition, then_branch, else_branch) => head(
            "if",
            vec![
                write_value(condition)?,
                write_value(then_branch)?,
                write_value(else_branch)?,
            ],
        ),
        v4::Value::LetDefinition(_, name, definition, body) => head(
            "let",
            vec![
                name_symbol(name),
                write_binding(definition)?,
                write_value(body)?,
            ],
        ),
        v4::Value::LetRecursion(_, bindings, body) => {
            let written = bindings
                .iter()
                .map(|v4::LetBinding(name, definition)| {
                    Ok(sexp(vec![name_symbol(name), write_binding(definition)?]))
                })
                .collect::<Result<Vec<_>, TransportDiagnostic>>()?;
            head(
                "letrec",
                vec![Element::from(list(written)), write_value(body)?],
            )
        }
        v4::Value::PatternMatch(_, scrutinee, cases) => {
            let mut items = vec![write_value(scrutinee)?];
            for v4::PatternCase(pattern, body) in cases {
                items.push(Element::from(list(vec![
                    write_pattern(pattern)?,
                    write_value(body)?,
                ])));
            }
            head("match", items)
        }
        v4::Value::Destructure(_, pattern, value, body) => head(
            "destructure",
            vec![
                write_pattern(pattern)?,
                write_value(value)?,
                write_value(body)?,
            ],
        ),
        v4::Value::Field(_, record, name) => {
            head("field", vec![write_value(record)?, name_symbol(name)])
        }
        v4::Value::FieldFunction(_, name) => head("fieldFunction", vec![name_symbol(name)]),
        v4::Value::Record(_, entries) => head("record", write_entries(entries)?),
        v4::Value::UpdateRecord(_, record, entries) => {
            let mut items = vec![write_value(record)?];
            items.extend(write_entries(entries)?);
            head("update", items)
        }
        v4::Value::Tuple(_, elements) => head("tuple", write_values(elements)?),
        v4::Value::List(_, elements) => head("list", write_values(elements)?),
        v4::Value::Hole(_, reason, expected) => {
            let mut items = vec![write_hole_reason(reason)?];
            if let Some(expected) = expected {
                items.push(write_type(expected)?);
            }
            head("hole", items)
        }
    })
}

fn read_values(elements: &[&Element]) -> Result<Vec<v4::Value>, TransportDiagnostic> {
    elements.iter().copied().map(read_value).collect()
}

fn write_values(values: &[v4::Value]) -> Result<Vec<Element>, TransportDiagnostic> {
    values.iter().map(write_value).collect()
}

fn read_entries(elements: &[&Element]) -> Result<Vec<v4::RecordFieldEntry>, TransportDiagnostic> {
    let mut entries: Vec<v4::RecordFieldEntry> = Vec::new();
    for element in elements {
        let pair = pair(element, "a record field is a name and a value")?;
        let name = name_of(pair[0])?;
        if entries
            .iter()
            .any(|v4::RecordFieldEntry(existing, _)| *existing == name)
        {
            return Err(member(format!(
                "field '{}' is set twice",
                name.to_canonical_string()
            )));
        }
        entries.push(v4::RecordFieldEntry(name, read_value(pair[1])?));
    }
    Ok(entries)
}

fn write_entries(entries: &[v4::RecordFieldEntry]) -> Result<Vec<Element>, TransportDiagnostic> {
    entries
        .iter()
        .map(|v4::RecordFieldEntry(name, value)| {
            Ok(sexp(vec![name_symbol(name), write_value(value)?]))
        })
        .collect()
}

/// A let binding: `{ inputTypes, outputType, body }`. It holds an expression body.
fn read_binding(element: &Element) -> Result<v4::ValueDefinition, TransportDiagnostic> {
    let fields = struct_fields(element, "let binding")?;
    Ok(v4::ValueDefinition {
        input_types: read_inputs(fields.get("inputTypes").copied())?,
        output_type: fields.get("outputType").map(|e| read_type(e)).transpose()?,
        body: v4::ValueBody::Expression(read_value(required_field(&fields, "body")?)?),
    })
}

fn write_binding(definition: &v4::ValueDefinition) -> Result<Element, TransportDiagnostic> {
    let v4::ValueBody::Expression(body) = &definition.body else {
        return Err(unwritten("a let binding whose body is not an expression"));
    };
    let mut builder = ion_rs::Struct::builder();
    if !definition.input_types.is_empty() {
        builder = builder.with_field("inputTypes", write_inputs(&definition.input_types)?);
    }
    if let Some(output) = &definition.output_type {
        builder = builder.with_field("outputType", write_type(output)?);
    }
    Ok(Element::from(
        builder.with_field("body", write_value(body)?).build(),
    ))
}

// =============================================================================
// Patterns
// =============================================================================

fn read_pattern(element: &Element) -> Result<v4::Pattern, TransportDiagnostic> {
    if !annotation_names(element)?.is_empty() {
        return Err(member("a v4 pattern carries no Ion annotation"));
    }
    if let Some(text) = symbol(element) {
        return Ok(if text == "_" {
            v4::Pattern::WildcardPattern(attrs())
        } else {
            binding(local_name(text)?)
        });
    }
    if element.as_list().is_some_and(|items| items.is_empty()) {
        return Ok(v4::Pattern::EmptyListPattern(attrs()));
    }
    if let Some(literal) = bare_literal(element)? {
        return Ok(v4::Pattern::LiteralPattern(attrs(), literal));
    }
    let (head, a, rest) = sexp_parts(element)?;
    let Some(head) = head else {
        return Ok(v4::Pattern::UnitPattern(a));
    };
    let arity = |count: usize| expect_arity(head, &rest, count);
    match head {
        "wildcard" => {
            arity(0)?;
            Ok(v4::Pattern::WildcardPattern(a.clone()))
        }
        "as" => {
            arity(2)?;
            Ok(v4::Pattern::AsPattern(
                a.clone(),
                Box::new(read_pattern(rest[0])?),
                name_of(rest[1])?,
            ))
        }
        "tuple" => Ok(v4::Pattern::TuplePattern(a.clone(), read_patterns(&rest)?)),
        "constructor" => {
            if rest.is_empty() {
                return Err(member("a constructor pattern names its constructor"));
            }
            Ok(v4::Pattern::ConstructorPattern(
                a.clone(),
                fq_of(rest[0])?,
                read_patterns(&rest[1..])?,
            ))
        }
        "headTail" => {
            arity(2)?;
            Ok(v4::Pattern::HeadTailPattern(
                a.clone(),
                Box::new(read_pattern(rest[0])?),
                Box::new(read_pattern(rest[1])?),
            ))
        }
        "emptyList" => {
            arity(0)?;
            Ok(v4::Pattern::EmptyListPattern(a.clone()))
        }
        "unit" => {
            arity(0)?;
            Ok(v4::Pattern::UnitPattern(a.clone()))
        }
        "int" | "float" | "bool" | "string" | "char" | "decimal" => {
            arity(1)?;
            Ok(v4::Pattern::LiteralPattern(
                a.clone(),
                literal(head, rest[0])?,
            ))
        }
        "document" => Err(member("a document literal cannot be a pattern")),
        other => Err(member(format!("unknown v4 pattern head '{other}'"))),
    }
}

fn write_pattern(pattern: &v4::Pattern) -> Result<Element, TransportDiagnostic> {
    let extra = value_attributes(pattern_attributes(pattern))?;
    let plain = extra.is_none();
    let head = |name: &str, rest: Vec<Element>| head(name, extra.clone(), rest);
    Ok(match pattern {
        v4::Pattern::WildcardPattern(_) if plain => Element::symbol("_"),
        v4::Pattern::WildcardPattern(_) => head("wildcard", Vec::new()),
        v4::Pattern::AsPattern(_, inner, name)
            if plain
                && matches!(
                    inner.as_ref(),
                    v4::Pattern::WildcardPattern(inner_attrs) if *inner_attrs == ValueAttributes::default()
                ) =>
        {
            name_symbol(name)
        }
        v4::Pattern::AsPattern(_, inner, name) => {
            head("as", vec![write_pattern(inner)?, name_symbol(name)])
        }
        v4::Pattern::TuplePattern(_, patterns) => head("tuple", write_patterns(patterns)?),
        v4::Pattern::ConstructorPattern(_, name, args) => {
            let mut items = vec![fq_symbol(name)];
            items.extend(write_patterns(args)?);
            head("constructor", items)
        }
        v4::Pattern::EmptyListPattern(_) if plain => Element::from(list(Vec::new())),
        v4::Pattern::EmptyListPattern(_) => head("emptyList", Vec::new()),
        v4::Pattern::HeadTailPattern(_, first, tail) => head(
            "headTail",
            vec![write_pattern(first)?, write_pattern(tail)?],
        ),
        v4::Pattern::LiteralPattern(_, literal) => {
            if matches!(literal, v4::Literal::Document(_)) {
                return Err(member("a document literal cannot be a pattern"));
            }
            write_literal(literal, extra.clone())?
        }
        v4::Pattern::UnitPattern(_) if plain => sexp(Vec::new()),
        v4::Pattern::UnitPattern(_) => head("unit", Vec::new()),
    })
}

fn pattern_attributes(pattern: &v4::Pattern) -> &ValueAttributes {
    match pattern {
        v4::Pattern::WildcardPattern(a)
        | v4::Pattern::AsPattern(a, _, _)
        | v4::Pattern::TuplePattern(a, _)
        | v4::Pattern::ConstructorPattern(a, _, _)
        | v4::Pattern::EmptyListPattern(a)
        | v4::Pattern::HeadTailPattern(a, _, _)
        | v4::Pattern::LiteralPattern(a, _)
        | v4::Pattern::UnitPattern(a) => a,
    }
}

/// A lambda's pattern. A non-empty list of names is shorthand: one name binds that name, and
/// several bind a tuple of names.
fn read_lambda_pattern(element: &Element) -> Result<v4::Pattern, TransportDiagnostic> {
    let Some(items) = element.as_list().filter(|items| !items.is_empty()) else {
        return read_pattern(element);
    };
    if !annotation_names(element)?.is_empty() {
        return Err(member("a lambda binding list carries no Ion annotation"));
    }
    let mut bindings = Vec::new();
    for item in items.iter() {
        let text = symbol(item).ok_or_else(|| member("a lambda binding list holds names"))?;
        bindings.push(binding(local_name(text)?));
    }
    Ok(match bindings.len() {
        1 => bindings.pop().expect("length checked"),
        _ => v4::Pattern::TuplePattern(attrs(), bindings),
    })
}

fn binding(name: Name) -> v4::Pattern {
    v4::Pattern::AsPattern(
        attrs(),
        Box::new(v4::Pattern::WildcardPattern(attrs())),
        name,
    )
}

fn read_patterns(elements: &[&Element]) -> Result<Vec<v4::Pattern>, TransportDiagnostic> {
    elements.iter().copied().map(read_pattern).collect()
}

fn write_patterns(patterns: &[v4::Pattern]) -> Result<Vec<Element>, TransportDiagnostic> {
    patterns.iter().map(write_pattern).collect()
}

// =============================================================================
// Literals
// =============================================================================

/// A bool, an int or a float written bare.
fn bare_literal(element: &Element) -> Result<Option<v4::Literal>, TransportDiagnostic> {
    if let Some(value) = element.as_bool() {
        return Ok(Some(v4::Literal::Bool(value)));
    }
    if let Some(value) = element.as_int() {
        let value = value.as_i128().ok_or_else(|| {
            member("an Ion int that does not fit i128 is written (int \"digits\")")
        })?;
        return Ok(Some(v4::Literal::Integer(value.into())));
    }
    if let Some(value) = element.as_float() {
        if !value.is_finite() {
            return Err(member(
                "an Ion float is finite; nan and the infinities have no Morphir literal",
            ));
        }
        return Ok(Some(v4::Literal::Float(v4::FloatLiteral::from_f64(value))));
    }
    Ok(None)
}

fn literal(head: &str, payload: &Element) -> Result<v4::Literal, TransportDiagnostic> {
    let text = || {
        payload
            .as_string()
            .ok_or_else(|| member(format!("({head} ...) holds a string")))
    };
    match head {
        "int" => match payload.as_int() {
            Some(value) => value
                .as_i128()
                .map(|value| v4::Literal::Integer(value.into()))
                .ok_or_else(|| member("(int ...) holds an int that fits i128, or its digits")),
            None => text()?
                .parse()
                .map(v4::Literal::Integer)
                .map_err(|_| member("(int ...) holds decimal digits")),
        },
        "float" => v4::FloatLiteral::from_lexeme(text()?)
            .map(v4::Literal::Float)
            .map_err(|error| member(error.to_string())),
        "bool" => payload
            .as_bool()
            .map(v4::Literal::Bool)
            .ok_or_else(|| member("(bool ...) holds a bool")),
        "string" => Ok(v4::Literal::String(text()?.to_owned())),
        "char" => {
            let mut chars = text()?.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) => Ok(v4::Literal::Char(ch)),
                _ => Err(member("(char ...) holds one character")),
            }
        }
        "decimal" => morphir_core::ir::decimal::DecimalLiteral::parse(text()?)
            .map(v4::Literal::Decimal)
            .map_err(|error| member(error.to_string())),
        other => Err(member(format!("unknown literal head '{other}'"))),
    }
}

/// A literal in its shortest spelling. Attributes force the S-expression form.
fn write_literal(
    literal: &v4::Literal,
    extra: Option<Element>,
) -> Result<Element, TransportDiagnostic> {
    let plain = extra.is_none();
    let head = |name: &str, payload: Element| head(name, extra.clone(), vec![payload]);
    Ok(match literal {
        v4::Literal::Bool(value) if plain => Element::from(*value),
        v4::Literal::Bool(value) => head("bool", Element::from(*value)),
        v4::Literal::Integer(value) => match i128::try_from(value) {
            Ok(small) if plain => Element::from(ion_rs::Int::from(small)),
            Ok(small) => head("int", Element::from(ion_rs::Int::from(small))),
            Err(_) => head("int", Element::string(value.to_string())),
        },
        v4::Literal::Float(value) => head("float", Element::string(value.lexeme())),
        v4::Literal::String(value) => head("string", Element::string(value.as_str())),
        v4::Literal::Char(value) => head("char", Element::string(value.to_string())),
        v4::Literal::Decimal(value) => head("decimal", Element::string(value.lexeme())),
        v4::Literal::Document(payload) => head("document", to_ion(payload)?),
    })
}

// =============================================================================
// Value definitions and specifications
// =============================================================================

pub(super) fn read_value_def(element: &Element) -> Result<(String, ValueDef), TransportDiagnostic> {
    let names = annotation_names(element)?;
    let access = access_of(&names)?;
    let fields = struct_fields(element, "value")?;
    super::refuse_top_level_metadata(&fields)?;
    refuse_on_definition(&fields)?;
    let name = required_string(&fields, "name")?.to_owned();
    let output_type = fields.get("outputType").map(|e| read_type(e)).transpose()?;
    let partial = || {
        fields
            .get("body")
            .map(|e| read_value(e).map(Box::new))
            .transpose()
    };
    let body = match names.as_slice() {
        [_, "def", "value"] => {
            v4::ValueBody::Expression(read_value(required_field(&fields, "body")?)?)
        }
        [_, "def", "native", "value"] => v4::ValueBody::Native {
            native_info: v4::NativeInfo {
                hint: read_hint(required_field(&fields, "hint")?)?,
                description: optional_string(&fields, "description")?.map(str::to_owned),
            },
        },
        [_, "def", "external", "value"] => v4::ValueBody::External {
            externals: read_externals(required_field(&fields, "externals")?)?,
            fallback: partial()?,
        },
        [_, "def", "incomplete", "value"] => v4::ValueBody::Incomplete {
            incompleteness: read_incompleteness(required_field(&fields, "incompleteness")?)?,
            partial_body: partial()?,
        },
        _ => {
            return Err(member(format!(
                "unknown v4 value definition {}",
                names.join("::")
            )));
        }
    };
    if output_type.is_none() && !matches!(body, v4::ValueBody::Incomplete { .. }) {
        return Err(member("outputType is optional only on an incomplete body"));
    }
    Ok((
        name,
        v4::AccessControlled {
            access,
            value: v4::Documented::new(
                optional_doc(&fields)?,
                v4::ValueDefinition {
                    input_types: read_inputs(fields.get("inputTypes").copied())?,
                    output_type,
                    body,
                },
            ),
        },
    ))
}

pub(super) fn write_value_def(
    name: &str,
    defined: &ValueDef,
) -> Result<Element, TransportDiagnostic> {
    let definition = &defined.value.value;
    let mut builder = ion_rs::Struct::builder().with_field("name", name);
    if !definition.input_types.is_empty() {
        builder = builder.with_field("inputTypes", write_inputs(&definition.input_types)?);
    }
    if let Some(output) = &definition.output_type {
        builder = builder.with_field("outputType", write_type(output)?);
    }
    let kind = match &definition.body {
        v4::ValueBody::Expression(body) => {
            builder = builder.with_field("body", write_value(body)?);
            None
        }
        v4::ValueBody::Native { native_info } => {
            builder = builder.with_field("hint", write_hint(&native_info.hint));
            if let Some(description) = &native_info.description {
                builder = builder.with_field("description", description.as_str());
            }
            Some("native")
        }
        v4::ValueBody::External {
            externals,
            fallback,
        } => {
            builder = builder.with_field("externals", write_externals(externals));
            if let Some(fallback) = fallback {
                builder = builder.with_field("body", write_value(fallback)?);
            }
            Some("external")
        }
        v4::ValueBody::Incomplete {
            incompleteness,
            partial_body,
        } => {
            builder = builder.with_field("incompleteness", write_incompleteness(incompleteness)?);
            if let Some(partial) = partial_body {
                builder = builder.with_field("body", write_value(partial)?);
            }
            Some("incomplete")
        }
    };
    if let Some(doc) = &defined.value.doc {
        builder = builder.with_field("doc", doc.text());
    }
    let access = access_symbol(&defined.access);
    let element = Element::from(builder.build());
    Ok(match kind {
        None => element.with_annotations([access, "def", "value"]),
        Some(kind) => element.with_annotations([access, "def", kind, "value"]),
    })
}

pub(super) fn read_value_spec(
    element: &Element,
) -> Result<(String, ValueSpec), TransportDiagnostic> {
    if annotation_names(element)?.as_slice() != ["public", "spec", "value"] {
        return Err(member("a value specification is public::spec::value"));
    }
    let fields = struct_fields(element, "value spec")?;
    super::refuse_top_level_metadata(&fields)?;
    let annotations = read_annotations(&fields)?;
    Ok((
        required_string(&fields, "name")?.to_owned(),
        v4::Documented::new(
            optional_doc(&fields)?,
            v4::ValueSpecification {
                annotations,
                inputs: read_inputs(fields.get("inputs").copied())?,
                output: read_type(required_field(&fields, "output")?)?,
            },
        ),
    ))
}

pub(super) fn write_value_spec(
    name: &str,
    spec: &ValueSpec,
) -> Result<Element, TransportDiagnostic> {
    let mut builder = with_annotations(
        ion_rs::Struct::builder().with_field("name", name),
        &spec.value.annotations,
    )?;
    if !spec.value.inputs.is_empty() {
        builder = builder.with_field("inputs", write_inputs(&spec.value.inputs)?);
    }
    builder = builder.with_field("output", write_type(&spec.value.output)?);
    if let Some(doc) = &spec.doc {
        builder = builder.with_field("doc", doc.text());
    }
    Ok(Element::from(builder.build()).with_annotations(["public", "spec", "value"]))
}

/// Inputs as a struct keyed by parameter name, in parameter order.
fn read_inputs(
    element: Option<&Element>,
) -> Result<IndexMap<String, v4::Type>, TransportDiagnostic> {
    let Some(element) = element else {
        return Ok(IndexMap::new());
    };
    let fields = element
        .as_struct()
        .ok_or_else(|| member("inputs are a struct"))?;
    let mut inputs = IndexMap::new();
    for (symbol, field) in fields.fields() {
        let name = crate::ir_transport::ion::symbol_text(symbol)?;
        local_name(name)?;
        if inputs.insert(name.to_owned(), read_type(field)?).is_some() {
            return Err(member(format!("input '{name}' is listed twice")));
        }
    }
    Ok(inputs)
}

fn write_inputs(inputs: &IndexMap<String, v4::Type>) -> Result<Element, TransportDiagnostic> {
    let mut builder = ion_rs::Struct::builder();
    for (name, ty) in inputs {
        builder = builder.with_field(name.as_str(), write_type(ty)?);
    }
    Ok(Element::from(builder.build()))
}

fn read_hint(element: &Element) -> Result<v4::NativeHint, TransportDiagnostic> {
    if annotation_names(element)?.as_slice() == ["platformSpecific"] {
        let fields: BTreeMap<&str, &Element> = struct_fields(element, "platformSpecific")?;
        return Ok(v4::NativeHint::PlatformSpecific {
            platform: required_string(&fields, "platform")?.to_owned(),
        });
    }
    match symbol(element) {
        Some("arithmetic") => Ok(v4::NativeHint::Arithmetic),
        Some("comparison") => Ok(v4::NativeHint::Comparison),
        Some("stringOp") => Ok(v4::NativeHint::StringOp),
        Some("collectionOp") => Ok(v4::NativeHint::CollectionOp),
        _ => Err(member(
            "hint is arithmetic, comparison, stringOp, collectionOp, or platformSpecific::{ platform }",
        )),
    }
}

fn write_hint(hint: &v4::NativeHint) -> Element {
    match hint {
        v4::NativeHint::Arithmetic => Element::symbol("arithmetic"),
        v4::NativeHint::Comparison => Element::symbol("comparison"),
        v4::NativeHint::StringOp => Element::symbol("stringOp"),
        v4::NativeHint::CollectionOp => Element::symbol("collectionOp"),
        v4::NativeHint::PlatformSpecific { platform } => Element::from(
            ion_rs::Struct::builder()
                .with_field("platform", platform.as_str())
                .build(),
        )
        .with_annotations(["platformSpecific"]),
    }
}

fn read_externals(element: &Element) -> Result<Vec<v4::ExternalBinding>, TransportDiagnostic> {
    let items = element
        .as_list()
        .ok_or_else(|| member("externals is a list"))?;
    let mut bindings: Vec<v4::ExternalBinding> = Vec::new();
    for item in items.iter() {
        let fields = struct_fields(item, "external binding")?;
        let binding = v4::ExternalBinding {
            target_platform: required_string(&fields, "targetPlatform")?.to_owned(),
            external_name: required_string(&fields, "externalName")?.to_owned(),
        };
        if bindings
            .iter()
            .any(|existing| existing.target_platform == binding.target_platform)
        {
            return Err(member(format!(
                "target platform '{}' is bound twice",
                binding.target_platform
            )));
        }
        bindings.push(binding);
    }
    Ok(bindings)
}

fn write_externals(externals: &[v4::ExternalBinding]) -> ion_rs::List {
    list(
        externals
            .iter()
            .map(|binding| {
                Element::from(
                    ion_rs::Struct::builder()
                        .with_field("targetPlatform", binding.target_platform.as_str())
                        .with_field("externalName", binding.external_name.as_str())
                        .build(),
                )
            })
            .collect(),
    )
}

// =============================================================================
// S-expression helpers
// =============================================================================

fn symbol(element: &Element) -> Option<&str> {
    element.as_symbol().and_then(|symbol| symbol.text())
}

/// A symbol or a string, as a head argument that names something.
fn text_of(element: &Element) -> Result<&str, TransportDiagnostic> {
    symbol(element)
        .or_else(|| element.as_string())
        .ok_or_else(|| member("expected a name"))
}

fn name_of(element: &Element) -> Result<Name, TransportDiagnostic> {
    local_name(text_of(element)?)
}

fn fq_of(element: &Element) -> Result<FQName, TransportDiagnostic> {
    fq_name(text_of(element)?)
}

fn name_symbol(name: &Name) -> Element {
    Element::symbol(name.to_canonical_string())
}

fn fq_symbol(name: &FQName) -> Element {
    Element::symbol(name.to_canonical_string())
}

/// The head and the arguments of an S-expression; the head is `None` for `()`.
/// The head, the attributes and the arguments of an S-expression; the head is `None` for `()`.
/// An attribute payload is an unannotated struct right after the head. A hole reason, which is
/// an annotated struct, is not one.
fn sexp_parts(
    element: &Element,
) -> Result<(Option<&str>, ValueAttributes, Vec<&Element>), TransportDiagnostic> {
    let items = element
        .as_sexp()
        .ok_or_else(|| member("expected an S-expression, a symbol, or a literal"))?;
    let mut items = items.iter();
    let Some(first) = items.next() else {
        return Ok((None, attrs(), Vec::new()));
    };
    let head = symbol(first).ok_or_else(|| member("an S-expression starts with a symbol"))?;
    let mut rest: Vec<&Element> = items.collect();
    let mut attributes = attrs();
    // A document's payload may itself be a struct, so there the payload comes second.
    let may_carry = head != "document" || rest.len() == 2;
    if may_carry
        && rest
            .first()
            .is_some_and(|first| first.as_struct().is_some() && first.annotations().is_empty())
    {
        attributes = read_value_attributes(rest.remove(0))?;
    }
    Ok((Some(head), attributes, rest))
}

fn expect_arity(head: &str, rest: &[&Element], count: usize) -> Result<(), TransportDiagnostic> {
    if rest.len() == count {
        Ok(())
    } else {
        Err(member(format!(
            "({head} ...) takes {count} argument(s), found {}",
            rest.len()
        )))
    }
}

fn pair<'a>(element: &'a Element, message: &str) -> Result<Vec<&'a Element>, TransportDiagnostic> {
    let items = element
        .as_sexp()
        .or_else(|| element.as_list())
        .ok_or_else(|| member(message.to_owned()))?;
    let items: Vec<&Element> = items.iter().collect();
    if items.len() != 2 {
        return Err(member(message.to_owned()));
    }
    Ok(items)
}

fn head(name: &str, attributes: Option<Element>, rest: Vec<Element>) -> Element {
    let mut items = vec![Element::symbol(name)];
    items.extend(attributes);
    items.extend(rest);
    sexp(items)
}

fn sexp(items: Vec<Element>) -> Element {
    Element::from(
        items
            .into_iter()
            .fold(ion_rs::Sequence::builder(), |builder, element| {
                builder.push(element)
            })
            .build_sexp(),
    )
}
