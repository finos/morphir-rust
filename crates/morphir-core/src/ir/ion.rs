//! The Ion profile of a document tree.
//!
//! A tree file is one Ion value with the same members as the JSON profile. The
//! writer emits that JSON text, which is valid Ion, so a number keeps the
//! lexeme the JSON profile keeps. A reader accepts that text and also Ion text
//! whose field names are symbols.

use std::str::FromStr;

use ion_rs::Element;
use serde_json::{Map, Number, Value};

use crate::ir::diagnostic::{Diagnostic, DiagnosticCode};

/// Reads one Ion document as a JSON value.
pub fn read(text: &str) -> Result<Value, Diagnostic> {
    // The writer emits JSON canonical text. Reading it as JSON keeps number
    // lexemes. Ion text that is not JSON, such as an unquoted field name, is
    // read by the Ion parser below.
    if let Ok(value) = crate::ir::json::read(text) {
        return Ok(value);
    }
    let element = Element::read_one(text)
        .map_err(|error| Diagnostic::syntax(DiagnosticCode::InvalidJson, "/", error.to_string()))?;
    json_value(&element)
}

/// Writes a JSON value as Ion text.
///
/// The text is the JSON profile's canonical spelling. It is one Ion value.
pub fn write_canonical(value: &Value) -> String {
    crate::ir::json::write_canonical(value)
}

fn json_value(element: &Element) -> Result<Value, Diagnostic> {
    if element.is_null() {
        return Ok(Value::Null);
    }
    if let Some(value) = element.as_bool() {
        return Ok(Value::Bool(value));
    }
    if let Some(value) = element.as_int() {
        return number(&value.to_string());
    }
    if let Some(value) = element.as_decimal() {
        return number(&json_decimal(&value.to_string()));
    }
    if let Some(value) = element.as_float() {
        return number(&format!("{value:?}"));
    }
    if let Some(value) = element.as_string() {
        return Ok(Value::String(value.to_owned()));
    }
    if let Some(list) = element.as_list().or_else(|| element.as_sexp()) {
        let mut items = Vec::new();
        for item in list.iter() {
            items.push(json_value(item)?);
        }
        return Ok(Value::Array(items));
    }
    if let Some(fields) = element.as_struct() {
        let mut object = Map::new();
        for (name, field) in fields.fields() {
            let key = name.text().ok_or_else(|| {
                Diagnostic::syntax(
                    DiagnosticCode::InvalidJson,
                    "/",
                    "an Ion field name has no text",
                )
            })?;
            if object.insert(key.to_owned(), json_value(field)?).is_some() {
                return Err(Diagnostic::syntax(
                    DiagnosticCode::InvalidJson,
                    "/",
                    format!("duplicate field '{key}'"),
                ));
            }
        }
        return Ok(Value::Object(object));
    }
    Err(Diagnostic::syntax(
        DiagnosticCode::InvalidJson,
        "/",
        "a document-tree value is a JSON value",
    ))
}

fn number(text: &str) -> Result<Value, Diagnostic> {
    Number::from_str(text).map(Value::Number).map_err(|_| {
        Diagnostic::syntax(
            DiagnosticCode::InvalidJson,
            "/",
            format!("'{text}' is not a JSON number"),
        )
    })
}

/// Turns Ion decimal display (`1.5d2`, `0.`) into a JSON number lexeme.
fn json_decimal(text: &str) -> String {
    let (body, exponent) = text
        .rsplit_once('d')
        .map(|(body, exponent)| (body, exponent.parse::<i32>().unwrap_or(0)))
        .unwrap_or((text, 0));
    let mut body = body.to_string();
    if body.ends_with('.') {
        body.push('0');
    }
    if exponent == 0 {
        body
    } else {
        format!("{body}e{exponent}")
    }
}
