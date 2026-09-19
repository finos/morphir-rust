//! Exact Gleam spellings for Morphir scalar literals.
use morphir_core::ir::v4::Literal;
use std::fmt::Write;
use std::io::{Error, ErrorKind, Result};

pub(super) fn generate(output: &mut String, literal: &Literal) -> Result<()> {
    match literal {
        Literal::Bool(value) => output.push_str(if *value { "True" } else { "False" }),
        Literal::Integer(value) => output.push_str(&value.to_string()),
        Literal::Float(value) => {
            // Gleam requires a decimal point even with an exponent. Rust's
            // shortest round-trip spelling preserves IEEE values and signed zero.
            let spelling = format!("{:?}", value.value());
            let (mantissa, exponent) = spelling.split_once('e').unwrap_or((&spelling, ""));
            output.push_str(mantissa);
            if !mantissa.contains('.') {
                output.push_str(".0");
            }
            if !exponent.is_empty() {
                output.push('e');
                output.push_str(exponent.trim_start_matches('+'));
            }
        }
        Literal::String(value) => {
            output.push('"');
            for character in value.chars() {
                match character {
                    '"' => output.push_str("\\\""),
                    '\\' => output.push_str("\\\\"),
                    '\n' => output.push_str("\\n"),
                    '\r' => output.push_str("\\r"),
                    '\t' => output.push_str("\\t"),
                    control if control.is_control() => {
                        write!(output, "\\u{{{:x}}}", u32::from(control))
                            .expect("writing to a String cannot fail");
                    }
                    character => output.push(character),
                }
            }
            output.push('"');
        }
        Literal::Char(_) => return Err(unsupported("character")),
        Literal::Decimal(_) => return Err(unsupported("decimal")),
        Literal::Document(_) => return Err(unsupported("document")),
    }
    Ok(())
}

fn unsupported(kind: &str) -> Error {
    Error::new(
        ErrorKind::Unsupported,
        format!("a {kind} literal has no exact Gleam type or spelling"),
    )
}
