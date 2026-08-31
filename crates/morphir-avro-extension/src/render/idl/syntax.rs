use std::collections::BTreeSet;

use serde_json::Value;

use crate::{
    AvroDiagnostic, AvroFullName, AvroInternalError, AvroMessage, AvroType, NamedSchema,
    Properties, escape_idl_identifier,
};

pub(super) fn next_schema_protocol_name(
    name: &AvroFullName,
) -> Result<AvroFullName, AvroInternalError> {
    AvroFullName::new(
        name.namespace().to_owned(),
        format!("{}Schemas", name.name()),
    )
    .map_err(|error| {
        AvroInternalError::invariant(format!(
            "schema protocol suffix invalidated checked Avro name {name}: {error}"
        ))
    })
}

pub(super) fn collect_schema_references(schema: &NamedSchema, references: &mut BTreeSet<String>) {
    if let NamedSchema::Record(record) = schema {
        for field in record.fields() {
            collect_type_references(field.tpe(), references);
        }
    }
}

pub(super) fn collect_type_references(tpe: &AvroType, references: &mut BTreeSet<String>) {
    match tpe {
        AvroType::Named(name) => {
            references.insert(name.to_string());
        }
        AvroType::Array(items, _) | AvroType::Map(items, _) => {
            collect_type_references(items, references);
        }
        AvroType::Union(union) => {
            for branch in union.branches() {
                collect_type_references(branch, references);
            }
        }
        AvroType::Logical { physical, .. } | AvroType::Annotated { physical, .. } => {
            collect_type_references(physical, references);
        }
        AvroType::Null
        | AvroType::Boolean
        | AvroType::Int
        | AvroType::Long
        | AvroType::Float
        | AvroType::Double
        | AvroType::Bytes
        | AvroType::String => {}
    }
}

pub(super) fn render_type(tpe: &AvroType) -> Result<String, AvroDiagnostic> {
    Ok(match tpe {
        AvroType::Null => "null".to_owned(),
        AvroType::Boolean => "boolean".to_owned(),
        AvroType::Int => "int".to_owned(),
        AvroType::Long => "long".to_owned(),
        AvroType::Float => "float".to_owned(),
        AvroType::Double => "double".to_owned(),
        AvroType::Bytes => "bytes".to_owned(),
        AvroType::String => "string".to_owned(),
        AvroType::Array(items, properties) => {
            format!(
                "{}array<{}>",
                inline_annotations(properties)?,
                render_type(items)?
            )
        }
        AvroType::Map(values, properties) => {
            format!(
                "{}map<{}>",
                inline_annotations(properties)?,
                render_type(values)?
            )
        }
        AvroType::Union(union) => format!(
            "union {{ {} }}",
            union
                .branches()
                .iter()
                .map(render_type)
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        ),
        AvroType::Named(name) => render_full_name(name),
        AvroType::Logical {
            physical,
            name,
            properties,
        } => render_logical_type(physical, name, properties)?,
        AvroType::Annotated {
            physical,
            properties,
        } => format!(
            "{}{}",
            inline_annotations(properties)?,
            render_type(physical)?
        ),
    })
}

pub(super) fn render_logical_type(
    physical: &AvroType,
    name: &str,
    properties: &Properties,
) -> Result<String, AvroDiagnostic> {
    let shorthand = match (name, physical) {
        ("decimal", AvroType::Bytes) => {
            let precision = property_integer(properties, "precision")?;
            let scale = property_integer(properties, "scale")?;
            Some((
                format!("decimal({precision}, {scale})"),
                &["precision", "scale"][..],
            ))
        }
        ("date", AvroType::Int) => Some(("date".to_owned(), &[][..])),
        ("time-millis", AvroType::Long) => Some(("time_ms".to_owned(), &[][..])),
        ("timestamp-millis", AvroType::Long) => Some(("timestamp_ms".to_owned(), &[][..])),
        ("local-timestamp-millis", AvroType::Long) => {
            Some(("local_timestamp_ms".to_owned(), &[][..]))
        }
        ("uuid", AvroType::String) => Some(("uuid".to_owned(), &[][..])),
        _ => None,
    };

    if let Some((shorthand, encoded_properties)) = shorthand {
        let annotations = properties
            .iter()
            .filter(|(key, _)| {
                key.as_str() != "logicalType" && !encoded_properties.contains(&key.as_str())
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Properties>();
        return Ok(format!(
            "{}{}",
            inline_annotations(&annotations)?,
            shorthand
        ));
    }

    let mut annotations = properties.clone();
    annotations.insert("logicalType".to_owned(), Value::String(name.to_owned()));
    Ok(format!(
        "{}{}",
        inline_annotations(&annotations)?,
        render_type(physical)?
    ))
}

pub(super) fn message_requires_javacc(message: &AvroMessage) -> bool {
    !message.properties().is_empty() && type_contains_named(message.response())
}

pub(super) fn type_contains_named(tpe: &AvroType) -> bool {
    match tpe {
        AvroType::Named(_) => true,
        AvroType::Array(items, _) | AvroType::Map(items, _) => type_contains_named(items),
        AvroType::Union(union) => union.branches().iter().any(type_contains_named),
        AvroType::Logical { physical, .. } | AvroType::Annotated { physical, .. } => {
            type_contains_named(physical)
        }
        AvroType::Null
        | AvroType::Boolean
        | AvroType::Int
        | AvroType::Long
        | AvroType::Float
        | AvroType::Double
        | AvroType::Bytes
        | AvroType::String => false,
    }
}

pub(super) fn property_integer(properties: &Properties, key: &str) -> Result<u64, AvroDiagnostic> {
    properties
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| AvroDiagnostic::invalid_option(format!("logical type property {key}")))
}

pub(super) fn render_full_name(name: &AvroFullName) -> String {
    name.namespace()
        .split('.')
        .chain(std::iter::once(name.name()))
        .map(escape_idl_identifier)
        .collect::<Vec<_>>()
        .join(".")
}

pub(super) fn render_annotations(
    output: &mut String,
    indent: &str,
    properties: &Properties,
) -> Result<(), AvroDiagnostic> {
    for (name, value) in properties {
        annotation(output, indent, name, value)?;
    }
    Ok(())
}

pub(super) fn annotation(
    output: &mut String,
    indent: &str,
    name: &str,
    value: &Value,
) -> Result<(), AvroDiagnostic> {
    output.push_str(indent);
    output.push('@');
    output.push_str(name);
    output.push('(');
    output.push_str(&json_value(value)?);
    output.push_str(")\n");
    Ok(())
}

pub(super) fn render_inline_annotations(
    output: &mut String,
    properties: &Properties,
) -> Result<(), AvroDiagnostic> {
    output.push_str(&inline_annotations(properties)?);
    Ok(())
}

pub(super) fn inline_annotations(properties: &Properties) -> Result<String, AvroDiagnostic> {
    let mut output = String::new();
    for (name, value) in properties {
        output.push('@');
        output.push_str(name);
        output.push('(');
        output.push_str(&json_value(value)?);
        output.push_str(") ");
    }
    Ok(output)
}

pub(super) fn property_string<'properties>(
    properties: &'properties Properties,
    key: &str,
) -> Option<&'properties str> {
    properties.get(key).and_then(Value::as_str)
}

pub(super) fn render_doc(output: &mut String, indent: &str, doc: Option<&str>) {
    let Some(doc) = doc else {
        return;
    };
    output.push_str(indent);
    output.push_str("/**\n");
    for line in doc.lines() {
        output.push_str(indent);
        output.push_str(" * ");
        output.push_str(&escape_doc_line(line));
        output.push('\n');
    }
    output.push_str(indent);
    output.push_str(" */\n");
}

pub(super) fn escape_doc_line(line: &str) -> String {
    line.chars()
        .map(|character| {
            if character.is_control() {
                format!("\\u{:04x}", u32::from(character))
            } else {
                character.to_string()
            }
        })
        .collect::<String>()
        .replace("*/", "* /")
}

pub(super) fn json_value(value: &Value) -> Result<String, AvroDiagnostic> {
    serde_json::to_string(value)
        .map_err(|error| AvroDiagnostic::invalid_option(format!("render IDL annotation: {error}")))
}

pub(super) fn json_string(value: &str) -> Result<String, AvroDiagnostic> {
    json_value(&Value::String(value.to_owned()))
}

pub(super) fn protocol_path(name: &AvroFullName) -> String {
    let mut components = name.namespace().split('.').collect::<Vec<_>>();
    components.retain(|component| !component.is_empty());
    components.push(name.name());
    format!("{}.avdl", components.join("/"))
}

pub(super) fn relative_path(from: &str, to: &str) -> String {
    let mut from = from.split('/').collect::<Vec<_>>();
    from.pop();
    let to = to.split('/').collect::<Vec<_>>();
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    std::iter::repeat_n("..", from.len() - common)
        .chain(to[common..].iter().copied())
        .collect::<Vec<_>>()
        .join("/")
}
