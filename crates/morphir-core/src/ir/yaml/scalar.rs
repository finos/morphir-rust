//! Plain-scalar resolution for the IR YAML profile.
//!
//! See `docs/spec/ir/schemas/v4/yaml-profile.md`, "Scalar resolution": booleans and null in their
//! three spellings, integers first, then the YAML 1.2 core float grammar with the lexeme rewritten
//! to its shortest JSON spelling, and everything else a string. Quoted and block scalars never
//! come here.

use serde_json::Number;
use std::str::FromStr;

/// What a plain scalar resolves to under the profile.
pub(crate) enum Resolved {
    Null,
    Bool(bool),
    Number(Number),
    Str(String),
}

fn is_int(s: &str) -> bool {
    let digits = s.strip_prefix(['-', '+']).unwrap_or(s);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// YAML 1.2 core float: `[-+]? ( \. [0-9]+ | [0-9]+ ( \. [0-9]* )? ) ( [eE] [-+]? [0-9]+ )?`
fn is_core_float(s: &str) -> bool {
    let s = s.strip_prefix(['-', '+']).unwrap_or(s);
    let (mantissa, exponent) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let mantissa_ok = match mantissa.split_once('.') {
        Some((int, frac)) => {
            (!int.is_empty() || !frac.is_empty())
                && int.bytes().all(|b| b.is_ascii_digit())
                && frac.bytes().all(|b| b.is_ascii_digit())
        }
        None => !mantissa.is_empty() && mantissa.bytes().all(|b| b.is_ascii_digit()),
    };
    let exponent_ok = match exponent {
        Some(e) => {
            let d = e.strip_prefix(['-', '+']).unwrap_or(e);
            !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())
        }
        None => true,
    };
    mantissa_ok && exponent_ok && (mantissa.contains('.') || exponent.is_some())
}

fn has_leading_zero(s: &str) -> bool {
    let d = s.strip_prefix(['-', '+']).unwrap_or(s);
    d.len() > 1 && d.starts_with('0') && d.as_bytes()[1].is_ascii_digit()
}

fn is_non_finite(s: &str) -> bool {
    let d = s.strip_prefix(['-', '+']).unwrap_or(s);
    matches!(d, ".inf" | ".Inf" | ".INF" | ".nan" | ".NaN" | ".NAN")
}

fn is_octal_or_hex(s: &str) -> bool {
    let d = s.strip_prefix(['-', '+']).unwrap_or(s);
    d.starts_with("0o") || d.starts_with("0x") || d.starts_with("0O") || d.starts_with("0X")
}

/// The shortest JSON spelling of a YAML float lexeme that preserves its digits.
pub(crate) fn json_lexeme(text: &str) -> String {
    let (sign, rest) = match text.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", text.strip_prefix('+').unwrap_or(text)),
    };
    let (mantissa, exponent) = match rest.find(['e', 'E']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let mantissa = match mantissa.split_once('.') {
        Some(("", frac)) => format!("0.{frac}"),
        Some((int, "")) => format!("{int}.0"),
        _ => mantissa.to_owned(),
    };
    format!("{sign}{mantissa}{exponent}")
}

/// True when a plain spelling of `text` would be read back as something other than a string.
///
/// The canonical writer uses this to decide whether a string has to be quoted.
#[allow(dead_code)] // Used by the canonical writer (task 3).
pub(crate) fn resolves_non_string(text: &str) -> bool {
    matches!(
        text,
        "true"
            | "True"
            | "TRUE"
            | "false"
            | "False"
            | "FALSE"
            | "null"
            | "Null"
            | "NULL"
            | "~"
            | ""
    ) || is_int(text)
        || is_core_float(text)
        || is_non_finite(text)
        || is_octal_or_hex(text)
}

/// Resolves a plain scalar, or refuses it with the message an `invalid_literal` carries.
pub(crate) fn resolve_plain(text: &str) -> Result<Resolved, &'static str> {
    match text {
        "true" | "True" | "TRUE" => return Ok(Resolved::Bool(true)),
        "false" | "False" | "FALSE" => return Ok(Resolved::Bool(false)),
        "null" | "Null" | "NULL" | "~" | "" => return Ok(Resolved::Null),
        _ => {}
    }
    if is_non_finite(text) {
        return Err("non-finite numbers are not part of the profile");
    }
    if is_octal_or_hex(text) {
        return Err("octal and hexadecimal spellings are not part of the profile; write decimal");
    }
    if is_int(text) {
        if has_leading_zero(text) {
            return Err("leading zeros are not part of the profile");
        }
        let lexeme = text.strip_prefix('+').unwrap_or(text);
        return Ok(Resolved::Number(
            Number::from_str(lexeme).map_err(|_| "this integer is not a JSON number")?,
        ));
    }
    if is_core_float(text) {
        if has_leading_zero(text) {
            return Err("leading zeros are not part of the profile");
        }
        return Ok(Resolved::Number(
            Number::from_str(&json_lexeme(text)).map_err(|_| "this number is not a JSON number")?,
        ));
    }
    Ok(Resolved::Str(text.to_owned()))
}
