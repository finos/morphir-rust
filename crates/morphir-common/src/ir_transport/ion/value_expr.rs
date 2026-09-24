//! Ion S-expression spelling of a classic value and pattern.
//!
//! A v3 value or pattern carries its type attribute as `{ inferredType }`
//! immediately after the head. A writer emits that struct. A reader that
//! does not see it uses `unit`.

use ion_rs::Element;
use morphir_core::ir::classic;

use super::type_expr::{canonical_fq, local_name, read_type, write_type};
use super::{IonCodec, required_field, struct_fields};
use crate::ir_transport::{Stage, TransportDiagnostic};

type TypeExpr = classic::Type<classic::Attrs>;
pub(super) type ClassicValue = classic::Value<classic::Attrs, TypeExpr>;
pub(super) type ClassicDefinition = classic::ValueDefinition<classic::Attrs, TypeExpr>;
pub(super) type ClassicValueEntry = (
    classic::Name,
    classic::AccessControlled<classic::Documented<ClassicDefinition>>,
);
type LetBinding = (classic::Name, Box<ClassicDefinition>);
type ClassicPattern = classic::Pattern<TypeExpr>;

pub(super) fn read_value(element: &Element) -> Result<ClassicValue, TransportDiagnostic> {
    if let Some(text) = element.as_symbol().and_then(|symbol| symbol.text()) {
        return Ok(classic::Value::Variable(
            classic::Type::Unit(classic::Attrs::None),
            local_name(text)?,
        ));
    }
    if let Some(value) = element.as_bool() {
        return Ok(literal(default_type(), classic::Literal::Bool(value)));
    }
    if let Some(value) = element.as_i64() {
        return Ok(literal(
            default_type(),
            classic::Literal::WholeNumber(value),
        ));
    }
    if let Some(value) = element.as_float() {
        return Ok(literal(default_type(), classic::Literal::Float(value)));
    }
    if element.as_sexp().is_some_and(|sexp| sexp.is_empty()) {
        return Ok(classic::Value::Unit(default_type()));
    }
    let (head, mut rest) = sexp_parts(element)?;
    let attr = take_type_attr(&mut rest)?;
    match head.as_str() {
        "unit" => Ok(classic::Value::Unit(attr)),
        "variable" => Ok(classic::Value::Variable(
            attr,
            local_name(symbol_text(one(&rest, "variable")?)?)?,
        )),
        "ref" => Ok(classic::Value::Reference(
            attr,
            read_fq(symbol_text(one(&rest, "ref")?)?)?,
        )),
        "constructor" => Ok(classic::Value::Constructor(
            attr,
            read_fq(symbol_text(one(&rest, "constructor")?)?)?,
        )),
        "apply" => Ok(classic::Value::Apply(
            attr,
            Box::new(read_value(nth(&rest, 0, "apply")?)?),
            Box::new(read_value(nth(&rest, 1, "apply")?)?),
        )),
        "lambda" => Ok(classic::Value::Lambda(
            attr,
            read_lambda_pattern(nth(&rest, 0, "lambda")?)?,
            Box::new(read_value(nth(&rest, 1, "lambda")?)?),
        )),
        "if" => Ok(classic::Value::IfThenElse(
            attr,
            Box::new(read_value(nth(&rest, 0, "if")?)?),
            Box::new(read_value(nth(&rest, 1, "if")?)?),
            Box::new(read_value(nth(&rest, 2, "if")?)?),
        )),
        "let" => {
            let (name, definition) = read_binding(nth(&rest, 0, "let")?, nth(&rest, 1, "let")?)?;
            Ok(classic::Value::LetDefinition(
                attr,
                name,
                Box::new(definition),
                Box::new(read_value(nth(&rest, 2, "let")?)?),
            ))
        }
        "letrec" => {
            let bindings = read_letrec(nth(&rest, 0, "letrec")?)?;
            Ok(classic::Value::LetRecursion(
                attr,
                bindings,
                Box::new(read_value(nth(&rest, 1, "letrec")?)?),
            ))
        }
        "match" => {
            let scrutinee = read_value(nth(&rest, 0, "match")?)?;
            let mut cases = Vec::new();
            for case in rest.iter().skip(1) {
                cases.push(read_case(case)?);
            }
            Ok(classic::Value::PatternMatch(
                attr,
                Box::new(scrutinee),
                cases,
            ))
        }
        "destructure" => Ok(classic::Value::Destructure(
            attr,
            read_pattern(nth(&rest, 0, "destructure")?)?,
            Box::new(read_value(nth(&rest, 1, "destructure")?)?),
            Box::new(read_value(nth(&rest, 2, "destructure")?)?),
        )),
        "field" => Ok(classic::Value::Field(
            attr,
            Box::new(read_value(nth(&rest, 0, "field")?)?),
            local_name(symbol_text(nth(&rest, 1, "field")?)?)?,
        )),
        "fieldFunction" => Ok(classic::Value::FieldFunction(
            attr,
            local_name(symbol_text(one(&rest, "fieldFunction")?)?)?,
        )),
        "record" => Ok(classic::Value::Record(attr, read_value_fields(&rest)?)),
        "update" => Ok(classic::Value::Update(
            attr,
            Box::new(read_value(nth(&rest, 0, "update")?)?),
            read_value_fields(&rest[1..])?,
        )),
        "tuple" => Ok(classic::Value::Tuple(attr, read_values(&rest)?)),
        "list" => Ok(classic::Value::List(attr, read_values(&rest)?)),
        "int" | "float" | "bool" | "string" | "char" | "decimal" => Ok(literal(
            attr,
            read_literal(head.as_str(), one(&rest, &head)?)?,
        )),
        other => Err(value_error(format!("unknown value head '{other}'"))),
    }
}

pub(super) fn write_value(value: &ClassicValue) -> Result<Element, TransportDiagnostic> {
    Ok(match value {
        classic::Value::Unit(attr) => sexp("unit", attr, Vec::new()),
        classic::Value::Variable(attr, name) => sexp(
            "variable",
            attr,
            vec![Element::symbol(canonical_local(name))],
        ),
        classic::Value::Reference(attr, name) => {
            sexp("ref", attr, vec![Element::symbol(canonical_fq(name))])
        }
        classic::Value::Constructor(attr, name) => sexp(
            "constructor",
            attr,
            vec![Element::symbol(canonical_fq(name))],
        ),
        classic::Value::Apply(attr, function, argument) => sexp(
            "apply",
            attr,
            vec![write_value(function)?, write_value(argument)?],
        ),
        classic::Value::Lambda(attr, pattern, body) => sexp(
            "lambda",
            attr,
            vec![write_pattern(pattern)?, write_value(body)?],
        ),
        classic::Value::IfThenElse(attr, condition, then_branch, else_branch) => sexp(
            "if",
            attr,
            vec![
                write_value(condition)?,
                write_value(then_branch)?,
                write_value(else_branch)?,
            ],
        ),
        classic::Value::LetDefinition(attr, name, definition, body) => sexp(
            "let",
            attr,
            vec![
                Element::symbol(canonical_local(name)),
                write_definition(definition)?,
                write_value(body)?,
            ],
        ),
        classic::Value::LetRecursion(attr, bindings, body) => {
            let mut written = ion_rs::Sequence::builder();
            for (name, definition) in bindings {
                written = written.push(sexp_raw(vec![
                    Element::symbol(canonical_local(name)),
                    write_definition(definition)?,
                ]));
            }
            sexp(
                "letrec",
                attr,
                vec![Element::from(written.build_list()), write_value(body)?],
            )
        }
        classic::Value::PatternMatch(attr, scrutinee, cases) => {
            let mut items = vec![write_value(scrutinee)?];
            for (pattern, body) in cases {
                items.push(Element::from(
                    ion_rs::Sequence::builder()
                        .push(write_pattern(pattern)?)
                        .push(write_value(body)?)
                        .build_list(),
                ));
            }
            sexp("match", attr, items)
        }
        classic::Value::Destructure(attr, pattern, value, body) => sexp(
            "destructure",
            attr,
            vec![
                write_pattern(pattern)?,
                write_value(value)?,
                write_value(body)?,
            ],
        ),
        classic::Value::Field(attr, record, name) => sexp(
            "field",
            attr,
            vec![write_value(record)?, Element::symbol(canonical_local(name))],
        ),
        classic::Value::FieldFunction(attr, name) => sexp(
            "fieldFunction",
            attr,
            vec![Element::symbol(canonical_local(name))],
        ),
        classic::Value::Record(attr, fields) => sexp("record", attr, write_pairs(fields)?),
        classic::Value::Update(attr, record, fields) => {
            let mut items = vec![write_value(record)?];
            items.extend(write_pairs(fields)?);
            sexp("update", attr, items)
        }
        classic::Value::Tuple(attr, elements) => sexp("tuple", attr, write_values(elements)?),
        classic::Value::List(attr, elements) => sexp("list", attr, write_values(elements)?),
        classic::Value::Literal(attr, literal) => write_literal(attr, literal)?,
    })
}

pub(super) fn read_inputs(
    element: Option<&Element>,
) -> Result<Vec<classic::value::ValueArgument<classic::Attrs, TypeExpr>>, TransportDiagnostic> {
    let Some(element) = element else {
        return Ok(Vec::new());
    };
    let Some(list) = element.as_list() else {
        return Err(value_error("inputTypes is a list"));
    };
    let mut inputs = Vec::new();
    for item in list.iter() {
        let fields = struct_fields(item, "input")?;
        let ty = read_type(required_field(&fields, "type")?)?;
        let annotation = match fields.get("inferredType") {
            Some(inferred) => read_type(inferred)?,
            None => ty.clone(),
        };
        inputs.push(classic::value::ValueArgument {
            name: local_name(require_text(&fields, "name")?)?,
            annotation,
            ty,
        });
    }
    Ok(inputs)
}

pub(super) fn write_inputs(
    inputs: &[classic::value::ValueArgument<classic::Attrs, TypeExpr>],
) -> ion_rs::List {
    inputs
        .iter()
        .map(|input| {
            let mut builder = ion_rs::Struct::builder()
                .with_field("name", canonical_local(&input.name))
                .with_field("type", write_type(&input.ty));
            if input.annotation != input.ty {
                builder = builder.with_field("inferredType", write_type(&input.annotation));
            }
            Element::from(builder.build())
        })
        .fold(ion_rs::Sequence::builder(), |builder, element| {
            builder.push(element)
        })
        .build_list()
}

pub(super) fn read_definition(
    fields: &std::collections::BTreeMap<&str, &Element>,
) -> Result<ClassicDefinition, TransportDiagnostic> {
    Ok(classic::ValueDefinition {
        input_types: read_inputs(fields.get("inputTypes").copied())?,
        output_type: read_type(required_field(fields, "outputType")?)?,
        body: read_value(required_field(fields, "body")?)?,
    })
}

fn write_definition(definition: &ClassicDefinition) -> Result<Element, TransportDiagnostic> {
    let mut builder =
        ion_rs::Struct::builder().with_field("outputType", write_type(&definition.output_type));
    if !definition.input_types.is_empty() {
        builder = builder.with_field("inputTypes", write_inputs(&definition.input_types));
    }
    builder = builder.with_field("body", write_value(&definition.body)?);
    Ok(Element::from(builder.build()))
}

fn read_pattern(element: &Element) -> Result<ClassicPattern, TransportDiagnostic> {
    if let Some(text) = element.as_symbol().and_then(|symbol| symbol.text()) {
        return bare_pattern(text);
    }
    if element.as_list().is_some_and(|list| list.is_empty()) {
        return Ok(classic::Pattern::EmptyList(default_type()));
    }
    if element.as_sexp().is_some_and(|sexp| sexp.is_empty()) {
        return Ok(classic::Pattern::Unit(default_type()));
    }
    if element.as_bool().is_some() || element.as_i64().is_some() || element.as_float().is_some() {
        return Ok(classic::Pattern::Literal(
            default_type(),
            read_bare_literal(element)?,
        ));
    }
    let (head, mut rest) = sexp_parts(element)?;
    let attr = take_type_attr(&mut rest)?;
    match head.as_str() {
        "wildcard" => Ok(classic::Pattern::Wildcard(attr)),
        "as" => Ok(classic::Pattern::As(
            attr,
            Box::new(read_pattern(nth(&rest, 0, "as")?)?),
            local_name(symbol_text(nth(&rest, 1, "as")?)?)?,
        )),
        "tuple" => Ok(classic::Pattern::Tuple(attr, read_patterns(&rest)?)),
        "constructor" => {
            let name = read_fq(symbol_text(nth(&rest, 0, "constructor")?)?)?;
            let mut args = Vec::new();
            for argument in rest.iter().skip(1) {
                args.push(read_pattern(argument)?);
            }
            Ok(classic::Pattern::Constructor(attr, name, args))
        }
        "headTail" => Ok(classic::Pattern::HeadTail(
            attr,
            Box::new(read_pattern(nth(&rest, 0, "headTail")?)?),
            Box::new(read_pattern(nth(&rest, 1, "headTail")?)?),
        )),
        "unit" => Ok(classic::Pattern::Unit(attr)),
        "emptyList" => Ok(classic::Pattern::EmptyList(attr)),
        "int" | "float" | "bool" | "string" | "char" | "decimal" => Ok(classic::Pattern::Literal(
            attr,
            read_literal(head.as_str(), one(&rest, &head)?)?,
        )),
        other => Err(value_error(format!("unknown pattern head '{other}'"))),
    }
}

fn write_pattern(pattern: &ClassicPattern) -> Result<Element, TransportDiagnostic> {
    Ok(match pattern {
        classic::Pattern::Wildcard(attr) => sexp("wildcard", attr, Vec::new()),
        classic::Pattern::As(attr, inner, name) => sexp(
            "as",
            attr,
            vec![
                write_pattern(inner)?,
                Element::symbol(canonical_local(name)),
            ],
        ),
        classic::Pattern::Tuple(attr, patterns) => sexp("tuple", attr, write_patterns(patterns)?),
        classic::Pattern::Constructor(attr, name, args) => {
            let mut items = vec![Element::symbol(canonical_fq(name))];
            items.extend(write_patterns(args)?);
            sexp("constructor", attr, items)
        }
        classic::Pattern::EmptyList(attr) => sexp("emptyList", attr, Vec::new()),
        classic::Pattern::HeadTail(attr, head, tail) => sexp(
            "headTail",
            attr,
            vec![write_pattern(head)?, write_pattern(tail)?],
        ),
        classic::Pattern::Literal(attr, literal) => write_literal(attr, literal)?,
        classic::Pattern::Unit(attr) => sexp("unit", attr, Vec::new()),
    })
}

fn read_lambda_pattern(element: &Element) -> Result<ClassicPattern, TransportDiagnostic> {
    if let Some(list) = element.as_list() {
        let mut patterns = Vec::new();
        for item in list.iter() {
            let Some(text) = item.as_symbol().and_then(|symbol| symbol.text()) else {
                return Err(value_error("a lambda binding list contains names"));
            };
            patterns.push(classic::Pattern::As(
                default_type(),
                Box::new(classic::Pattern::Wildcard(default_type())),
                local_name(text)?,
            ));
        }
        return Ok(match patterns.len() {
            1 => patterns.pop().expect("length checked"),
            _ => classic::Pattern::Tuple(default_type(), patterns),
        });
    }
    read_pattern(element)
}

fn bare_pattern(text: &str) -> Result<ClassicPattern, TransportDiagnostic> {
    if text == "_" {
        return Ok(classic::Pattern::Wildcard(default_type()));
    }
    Ok(classic::Pattern::As(
        default_type(),
        Box::new(classic::Pattern::Wildcard(default_type())),
        local_name(text)?,
    ))
}

fn read_literal(head: &str, element: &Element) -> Result<classic::Literal, TransportDiagnostic> {
    match head {
        "int" => element
            .as_i64()
            .map(classic::Literal::WholeNumber)
            .ok_or_else(|| value_error("int payload is an Ion int")),
        "float" => element
            .as_float()
            .map(classic::Literal::Float)
            .ok_or_else(|| value_error("float payload is an Ion float")),
        "bool" => element
            .as_bool()
            .map(classic::Literal::Bool)
            .ok_or_else(|| value_error("bool payload is an Ion bool")),
        "string" => element
            .as_string()
            .map(|text| classic::Literal::String(text.to_owned()))
            .ok_or_else(|| value_error("string payload is an Ion string")),
        "char" => {
            let text = element
                .as_string()
                .ok_or_else(|| value_error("char payload is a one-character string"))?;
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) => Ok(classic::Literal::Char(ch)),
                _ => Err(value_error("char payload is a one-character string")),
            }
        }
        "decimal" => {
            let text = element
                .as_string()
                .ok_or_else(|| value_error("decimal payload is the lexeme"))?;
            morphir_core::ir::decimal::DecimalLiteral::parse(text)
                .map(classic::Literal::Decimal)
                .map_err(|error| value_error(error.to_string()))
        }
        _ => Err(value_error(format!("unknown literal head '{head}'"))),
    }
}

fn read_bare_literal(element: &Element) -> Result<classic::Literal, TransportDiagnostic> {
    if let Some(value) = element.as_bool() {
        return Ok(classic::Literal::Bool(value));
    }
    if let Some(value) = element.as_i64() {
        return Ok(classic::Literal::WholeNumber(value));
    }
    if let Some(value) = element.as_float() {
        return Ok(classic::Literal::Float(value));
    }
    Err(value_error("a literal pattern is a bool, int, or float"))
}

fn write_literal(
    attr: &TypeExpr,
    literal: &classic::Literal,
) -> Result<Element, TransportDiagnostic> {
    let (head, payload) = match literal {
        classic::Literal::WholeNumber(value) => ("int", Element::from(*value)),
        classic::Literal::Float(value) => ("float", Element::from(*value)),
        classic::Literal::Bool(value) => ("bool", Element::from(*value)),
        classic::Literal::String(value) => ("string", Element::string(value.as_str())),
        classic::Literal::Char(value) => ("char", Element::string(value.to_string())),
        classic::Literal::Decimal(value) => ("decimal", Element::string(value.lexeme())),
    };
    Ok(sexp(head, attr, vec![payload]))
}

fn read_binding(
    name: &Element,
    definition: &Element,
) -> Result<(classic::Name, ClassicDefinition), TransportDiagnostic> {
    let fields = struct_fields(definition, "definition")?;
    Ok((local_name(symbol_text(name)?)?, read_definition(&fields)?))
}

fn read_letrec(element: &Element) -> Result<Vec<LetBinding>, TransportDiagnostic> {
    let Some(list) = element.as_list().or_else(|| element.as_sexp()) else {
        return Err(value_error("letrec bindings are a list"));
    };
    read_letrec_pairs(list)
}

fn read_letrec_pairs(list: &ion_rs::Sequence) -> Result<Vec<LetBinding>, TransportDiagnostic> {
    let mut bindings = Vec::new();
    for item in list.iter() {
        let Some(inner) = item.as_sexp().or_else(|| item.as_list()) else {
            return Err(value_error("a letrec binding is a name and a definition"));
        };
        let children: Vec<&Element> = inner.iter().collect();
        if children.len() != 2 {
            return Err(value_error("a letrec binding is a name and a definition"));
        }
        let fields = struct_fields(children[1], "definition")?;
        bindings.push((
            local_name(symbol_text(children[0])?)?,
            Box::new(read_definition(&fields)?),
        ));
    }
    Ok(bindings)
}

fn read_case(element: &Element) -> Result<(ClassicPattern, ClassicValue), TransportDiagnostic> {
    let Some(list) = element.as_list().or_else(|| element.as_sexp()) else {
        return Err(value_error("a match case is a pattern and a body"));
    };
    let children: Vec<&Element> = list.iter().collect();
    if children.len() != 2 {
        return Err(value_error("a match case is a pattern and a body"));
    }
    Ok((read_pattern(children[0])?, read_value(children[1])?))
}

fn read_values(elements: &[&Element]) -> Result<Vec<ClassicValue>, TransportDiagnostic> {
    elements.iter().copied().map(read_value).collect()
}

fn read_patterns(elements: &[&Element]) -> Result<Vec<ClassicPattern>, TransportDiagnostic> {
    elements.iter().copied().map(read_pattern).collect()
}

fn read_value_fields(
    elements: &[&Element],
) -> Result<Vec<(classic::Name, ClassicValue)>, TransportDiagnostic> {
    let mut fields = Vec::new();
    for element in elements {
        let Some(pair) = element.as_sexp().or_else(|| element.as_list()) else {
            return Err(value_error("a record field is a name and a value"));
        };
        let children: Vec<&Element> = pair.iter().collect();
        if children.len() != 2 {
            return Err(value_error("a record field is a name and a value"));
        }
        fields.push((
            local_name(symbol_text(children[0])?)?,
            read_value(children[1])?,
        ));
    }
    Ok(fields)
}

fn write_values(values: &[ClassicValue]) -> Result<Vec<Element>, TransportDiagnostic> {
    values.iter().map(write_value).collect()
}

fn write_patterns(patterns: &[ClassicPattern]) -> Result<Vec<Element>, TransportDiagnostic> {
    patterns.iter().map(write_pattern).collect()
}

fn write_pairs(
    fields: &[(classic::Name, ClassicValue)],
) -> Result<Vec<Element>, TransportDiagnostic> {
    fields
        .iter()
        .map(|(name, value)| {
            Ok(sexp_raw(vec![
                Element::symbol(canonical_local(name)),
                write_value(value)?,
            ]))
        })
        .collect()
}

fn sexp(head: &str, attr: &TypeExpr, rest: Vec<Element>) -> Element {
    let mut items = vec![
        Element::symbol(head),
        Element::from(
            ion_rs::Struct::builder()
                .with_field("inferredType", write_type(attr))
                .build(),
        ),
    ];
    items.extend(rest);
    sexp_raw(items)
}

fn sexp_raw(items: Vec<Element>) -> Element {
    Element::from(
        items
            .into_iter()
            .fold(ion_rs::Sequence::builder(), |builder, element| {
                builder.push(element)
            })
            .build_sexp(),
    )
}

fn sexp_parts(element: &Element) -> Result<(String, Vec<&Element>), TransportDiagnostic> {
    let Some(sequence) = element.as_sexp() else {
        return Err(value_error("expected an S-expression"));
    };
    let mut children = sequence.iter();
    let head = children
        .next()
        .ok_or_else(|| value_error("an S-expression has a head"))?;
    let head = symbol_text(head)?.to_owned();
    Ok((head, children.collect()))
}

fn take_type_attr(rest: &mut Vec<&Element>) -> Result<TypeExpr, TransportDiagnostic> {
    let Some(first) = rest.first() else {
        return Ok(default_type());
    };
    if first.as_struct().is_none() {
        return Ok(default_type());
    }
    let fields = struct_fields(first, "attributes")?;
    if !fields.contains_key("inferredType") {
        return Ok(default_type());
    }
    let ty = read_type(required_field(&fields, "inferredType")?)?;
    rest.remove(0);
    Ok(ty)
}

fn literal(attr: TypeExpr, literal: classic::Literal) -> ClassicValue {
    classic::Value::Literal(attr, literal)
}

fn default_type() -> TypeExpr {
    classic::Type::Unit(classic::Attrs::None)
}

fn read_fq(text: &str) -> Result<classic::FQName, TransportDiagnostic> {
    let name = morphir_core::naming::FQName::from_canonical_string(text).map_err(|error| {
        IonCodec::error(
            "morphir::ir::ion::invalid_name",
            Stage::Normalization,
            error,
        )
    })?;
    Ok(classic::FQName::new(
        super::classic_path_from(&name.package_path),
        super::classic_path_from(&name.module_path),
        super::classic_name_from(&name.local_name),
    ))
}

fn canonical_local(name: &classic::Name) -> String {
    super::canonical_name(name)
}

fn symbol_text(element: &Element) -> Result<&str, TransportDiagnostic> {
    if let Some(text) = element.as_symbol().and_then(|symbol| symbol.text()) {
        return Ok(text);
    }
    if let Some(text) = element.as_string() {
        return Ok(text);
    }
    Err(value_error("expected a symbol or a string"))
}

fn require_text<'a>(
    fields: &std::collections::BTreeMap<&str, &'a Element>,
    name: &str,
) -> Result<&'a str, TransportDiagnostic> {
    let element = required_field(fields, name)?;
    symbol_text(element)
}

fn one<'a>(rest: &[&'a Element], head: &str) -> Result<&'a Element, TransportDiagnostic> {
    nth(rest, 0, head)
}

fn nth<'a>(
    rest: &[&'a Element],
    index: usize,
    head: &str,
) -> Result<&'a Element, TransportDiagnostic> {
    rest.get(index)
        .copied()
        .ok_or_else(|| value_error(format!("'{head}' is missing an argument")))
}

fn value_error(message: impl Into<String>) -> TransportDiagnostic {
    IonCodec::error(
        "morphir::ir::ion::invalid_member",
        Stage::Normalization,
        message,
    )
}
