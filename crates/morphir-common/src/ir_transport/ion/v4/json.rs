//! JSON values in Ion: a document literal's payload, and the `constraints` and `extensions`
//! attribute maps.
//!
//! JSON null, booleans, strings, arrays and objects are the Ion values of the same kind. A number
//! keeps its lexeme. An integer is an Ion int. Any other number is an Ion decimal when the decimal's
//! text gives the lexeme back (with `d` for `e`), and `number::"<lexeme>"` when it does not, as for
//! `1.5e2` or an integer too large for an Ion int. An Ion float, timestamp, symbol, blob, clob,
//! S-expression or typed null has no JSON counterpart and is refused.

use std::str::FromStr;

use ion_rs::{Element, IonType};
use serde_json::{Map, Number, Value as JsonValue};

use super::{list, member};
use crate::ir_transport::TransportDiagnostic;
use crate::ir_transport::ion::{annotation_names, symbol_text};

/// The annotation that marks a number whose lexeme an Ion int or decimal cannot keep.
const NUMBER: &str = "number";

pub(super) fn to_ion(value: &JsonValue) -> Result<Element, TransportDiagnostic> {
    Ok(match value {
        JsonValue::Null => Element::null(IonType::Null),
        JsonValue::Bool(value) => Element::from(*value),
        JsonValue::Number(number) => number_to_ion(&number.to_string())?,
        JsonValue::String(text) => Element::string(text.as_str()),
        JsonValue::Array(items) => Element::from(list(
            items.iter().map(to_ion).collect::<Result<Vec<_>, _>>()?,
        )),
        JsonValue::Object(members) => object_to_ion(members)?,
    })
}

pub(super) fn object_to_ion(
    members: &Map<String, JsonValue>,
) -> Result<Element, TransportDiagnostic> {
    let mut builder = ion_rs::Struct::builder();
    for (name, value) in members {
        builder = builder.with_field(name.as_str(), to_ion(value)?);
    }
    Ok(Element::from(builder.build()))
}

pub(super) fn from_ion(element: &Element) -> Result<JsonValue, TransportDiagnostic> {
    let annotations = annotation_names(element)?;
    if annotations == [NUMBER] {
        let text = element
            .as_string()
            .ok_or_else(|| member("number:: annotates a string that holds a JSON number"))?;
        return json_number(text).map(JsonValue::Number);
    }
    if !annotations.is_empty() {
        return Err(member(format!(
            "a JSON value carries no Ion annotation except number::, found {}",
            annotations.join("::")
        )));
    }
    if element.is_null() {
        return if element.ion_type() == IonType::Null {
            Ok(JsonValue::Null)
        } else {
            Err(member("a typed Ion null has no JSON counterpart"))
        };
    }
    match element.ion_type() {
        IonType::Bool => Ok(JsonValue::Bool(
            element.as_bool().expect("ion type checked"),
        )),
        IonType::Int => {
            let value = element
                .as_int()
                .and_then(|value| value.as_i128())
                .ok_or_else(|| member("an Ion int that does not fit i128 is written number::"))?;
            json_number(&value.to_string()).map(JsonValue::Number)
        }
        IonType::Decimal => {
            let decimal = element.as_decimal().expect("ion type checked");
            json_number(&decimal_lexeme(&decimal.to_string())).map(JsonValue::Number)
        }
        IonType::String => Ok(JsonValue::String(
            element.as_string().expect("ion type checked").to_owned(),
        )),
        IonType::List => Ok(JsonValue::Array(
            element
                .as_list()
                .expect("ion type checked")
                .iter()
                .map(from_ion)
                .collect::<Result<_, _>>()?,
        )),
        IonType::Struct => object_from_ion(element).map(JsonValue::Object),
        other => Err(member(format!(
            "an Ion {other} has no JSON counterpart; a float is written as a decimal"
        ))),
    }
}

pub(super) fn object_from_ion(
    element: &Element,
) -> Result<Map<String, JsonValue>, TransportDiagnostic> {
    let fields = element
        .as_struct()
        .ok_or_else(|| member("a JSON object is an Ion struct"))?;
    if !annotation_names(element)?.is_empty() {
        return Err(member("a JSON object carries no Ion annotation"));
    }
    let mut object = Map::new();
    for (name, value) in fields.fields() {
        let name = symbol_text(name)?;
        if object.insert(name.to_owned(), from_ion(value)?).is_some() {
            return Err(member(format!("member '{name}' is listed twice")));
        }
    }
    Ok(object)
}

/// An Ion int or decimal that reads back as `lexeme`, else `number::"<lexeme>"`.
fn number_to_ion(lexeme: &str) -> Result<Element, TransportDiagnostic> {
    let integer = lexeme
        .strip_prefix('-')
        .unwrap_or(lexeme)
        .chars()
        .all(|character| character.is_ascii_digit());
    if integer {
        if let Ok(value) = lexeme.parse::<i128>() {
            return Ok(Element::from(ion_rs::Int::from(value)));
        }
    } else if let Ok(decimal) = Element::read_one(lexeme.replace(['e', 'E'], "d").as_bytes())
        && decimal.ion_type() == IonType::Decimal
        && decimal
            .as_decimal()
            .is_some_and(|value| decimal_lexeme(&value.to_string()) == lexeme)
    {
        return Ok(decimal);
    }
    Ok(Element::string(lexeme).with_annotations([NUMBER]))
}

/// The JSON spelling of an Ion decimal's text: `d` becomes `e`, and a trailing `.` goes.
fn decimal_lexeme(text: &str) -> String {
    text.replace('d', "e")
        .replace(".e", "e")
        .trim_end_matches('.')
        .to_owned()
}

fn json_number(text: &str) -> Result<Number, TransportDiagnostic> {
    Number::from_str(text).map_err(|_| member(format!("'{text}' is not a JSON number")))
}
