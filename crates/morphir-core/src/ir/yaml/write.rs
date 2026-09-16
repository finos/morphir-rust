//! The canonical YAML writer.
//!
//! See `docs/spec/ir/schemas/v4/yaml-profile.md`, "Canonical writer", in finos/morphir. The
//! kit's `yaml canonical` fences are the byte oracle, and this mirrors the reference writer
//! (`packages/ir/src/codec/yaml/write.ts` in finos/morphir-typescript) decision for decision:
//!
//!  - block mappings, two-space indentation, members in writer order, an empty mapping inline
//!    as `{}`;
//!  - a sequence is flow style when it holds no mapping at any depth, so `[a]` and
//!    `[[value, a]]` are one line; a sequence holding a mapping is a block sequence, and an
//!    empty one is `[]`;
//!  - scalars are plain unless plain resolution would change their meaning or an indicator
//!    would appear. Outside flow a `:` is an indicator only before a space and a `#` only
//!    after one, so `morphir/SDK:basics#int` stays plain; inside a flow collection `[]{},:#`
//!    force double quotes;
//!  - numbers keep the lexeme they carry.
//!
//! A block scalar is never written: a string carrying a newline is double-quoted with the break
//! escaped.

use super::scalar::resolves_non_string;
use serde_json::Value;

const INDENT: &str = "  ";

/// Writes a JSON value tree as canonical profile YAML, with one trailing newline.
pub fn write_canonical(value: &Value) -> String {
    let lines = match spelling_of(value) {
        Spelling::Inline(text) => vec![text],
        Spelling::Block(collection) => block(collection, ""),
    };
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// A value that needs no block: every scalar, and every sequence whose items are themselves
/// flowable. A mapping is never flowable — an empty one is written `{}`, which is a spelling
/// rather than a flow collection.
fn is_flowable(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.iter().all(is_flowable),
        Value::Object(_) => false,
        _ => true,
    }
}

/// A code point YAML would have to escape rather than print.
fn is_control(c: char) -> bool {
    let code = c as u32;
    code < 0x20 || code == 0x7f
}

fn needs_quotes(text: &str, in_flow: bool) -> bool {
    if text.is_empty() || resolves_non_string(text) {
        return true;
    }
    let first = text
        .chars()
        .next()
        .expect("a non-empty string has a first character");
    if "-?:,[]{}#&*!|>'\"%@` ".contains(first) {
        return true;
    }
    if text.ends_with(' ') || text.ends_with(':') {
        return true;
    }
    if text.contains(": ") || text.contains(" #") {
        return true;
    }
    if text.chars().any(is_control) {
        return true;
    }
    in_flow && text.chars().any(|c| "[]{},:#".contains(c))
}

fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if is_control(c) => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn scalar(value: &Value, in_flow: bool) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(true) => "true".to_owned(),
        Value::Bool(false) => "false".to_owned(),
        // `arbitrary_precision`: a number parsed from text keeps the lexeme it was written with.
        Value::Number(number) => number.to_string(),
        Value::String(text) => string_scalar(text, in_flow),
        Value::Array(_) | Value::Object(_) => {
            unreachable!("a collection is never written as a scalar")
        }
    }
}

fn string_scalar(text: &str, in_flow: bool) -> String {
    if needs_quotes(text, in_flow) {
        quote(text)
    } else {
        text.to_owned()
    }
}

/// A flowable value on one line. Nested sequences stay nested, and every scalar inside is quoted
/// by the flow rules, because the whole line is flow context.
fn flow(value: &Value) -> String {
    match value {
        Value::Array(items) if items.is_empty() => "[]".to_owned(),
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(flow).collect();
            format!("[{}]", inner.join(", "))
        }
        other => scalar(other, true),
    }
}

/// How a value is spelled: on the line it starts on, or as the block that follows it.
enum Spelling<'a> {
    Inline(String),
    Block(&'a Value),
}

fn spelling_of(value: &Value) -> Spelling<'_> {
    match value {
        Value::Object(members) if members.is_empty() => Spelling::Inline("{}".to_owned()),
        Value::Object(_) => Spelling::Block(value),
        Value::Array(_) if is_flowable(value) => Spelling::Inline(flow(value)),
        Value::Array(_) => Spelling::Block(value),
        other => Spelling::Inline(scalar(other, false)),
    }
}

/// The lines of `value` as a collection at `indent`. Each line already carries its indent; a
/// block-sequence item places its first line after `- ` and keeps the rest, which is exactly the
/// one-level-deeper indent `- ` occupies.
fn block(value: &Value, indent: &str) -> Vec<String> {
    let inner_indent = format!("{indent}{INDENT}");
    let mut lines = Vec::new();
    match value {
        Value::Object(members) => {
            for (key, member) in members {
                let key = string_scalar(key, false);
                match spelling_of(member) {
                    Spelling::Inline(text) => lines.push(format!("{indent}{key}: {text}")),
                    Spelling::Block(collection) => {
                        lines.push(format!("{indent}{key}:"));
                        lines.extend(block(collection, &inner_indent));
                    }
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                match spelling_of(item) {
                    Spelling::Inline(text) => lines.push(format!("{indent}- {text}")),
                    Spelling::Block(collection) => {
                        let mut inner = block(collection, &inner_indent).into_iter();
                        let first = inner.next().unwrap_or_default();
                        lines.push(format!("{indent}- {}", first.trim_start()));
                        lines.extend(inner);
                    }
                }
            }
        }
        _ => unreachable!("only a collection reaches the block writer"),
    }
    lines
}
