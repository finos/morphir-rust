//! The JSON storage profile: a strict reader and the canonical writer.
//!
//! See `docs/spec/ir/schemas/v4/yaml-profile.md`'s JSON twin: this reader refuses what
//! `serde_json::Value` alone cannot see — a repeated object member, and a document nested past
//! the ceiling every reader in this crate shares — before it lets serde_json fold the text into a
//! value tree.

use std::collections::HashSet;
use std::fmt;

use serde::Deserialize;
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::Value as Json;

use crate::ir::v4::{IRFile, TypeEncoding, with_type_encoding};
use crate::ir::{Diagnostic, DiagnosticCode, DiagnosticError, Warning};

/// How many nested containers a document may carry, matching the reference reader's own ceiling
/// (`MAX_DEPTH` in `packages/ir/src/codec/json/value.ts`) and the YAML reader's
/// [`crate::ir::yaml::MAX_DEPTH`].
///
/// A reader that follows arbitrary nesting turns a small input into a deep recursion, so the
/// profile puts a ceiling on it and reports `nesting_too_deep` rather than failing some other way
/// at some other depth.
pub const MAX_DEPTH: usize = 1000;

/// The stack `read` grows onto when it needs to, rather than assumes it already has.
///
/// [`MAX_DEPTH`] is a promise: a document nesting that many containers is conforming, and the
/// answer to one nesting a container more is `nesting_too_deep`, not a crashed process. Both the
/// syntax probe and the parser recurse once per level, and 1000 levels of an unoptimized build's
/// frames do not fit in the stack a thread is given by default — on Windows the main thread's
/// stack is whatever the linker reserved, which is 1 MiB unless someone says otherwise. This is
/// the size of the stack [`stacker::maybe_grow`] allocates when [`RED_ZONE`] says the caller's own
/// stack is too shallow to recurse that far, matching `morphir-common`'s own
/// `IR_RECURSION_STACK_BYTES` and the mck adapter's own decode-thread stack.
pub(crate) const READ_STACK_BYTES: usize = 64 * 1024 * 1024;

/// How much headroom `read` demands before it recurses, below which [`stacker::maybe_grow`] grows
/// a fresh [`READ_STACK_BYTES`] stack rather than running the probe and the parse on what the
/// caller's stack has left.
///
/// This has to be large enough that an ordinary caller — a test thread, a plain function call from
/// the document-tree layout reading one file among hundreds, the process's main thread — always
/// grows before recursing [`MAX_DEPTH`] levels: with less than this much free, a document at the
/// ceiling could run out of native stack partway through the probe or the parse, on a build
/// without optimizations giving each recursive frame its least economical layout. It also has to
/// be small enough that a caller already running on a stack [`READ_STACK_BYTES`] or larger — the
/// mck adapter's own decode thread, or a document-tree read that already grew once for the whole
/// tree — is not made to grow again for every file.
pub(crate) const RED_ZONE: usize = 16 * 1024 * 1024;

/// Reads a JSON document under the storage profile: no repeated object member, no more than
/// [`MAX_DEPTH`] nested containers, and otherwise whatever `serde_json` accepts.
///
/// Grows onto a [`READ_STACK_BYTES`] stack via [`stacker::maybe_grow`] when the caller's own stack
/// is shallower than [`RED_ZONE`], so a document at the nesting ceiling answers `nesting_too_deep`
/// rather than overflowing whatever stack the caller happens to be on — without paying for a
/// spawned thread on every call, which matters here because the document-tree layout calls this
/// once per file of a tree.
pub fn read(text: &str) -> Result<Json, Diagnostic> {
    stacker::maybe_grow(RED_ZONE, READ_STACK_BYTES, || read_here(text))
}

fn read_here(text: &str) -> Result<Json, Diagnostic> {
    // A repeated member and a document nested past the ceiling are properties of the text, not of
    // any value: `serde_json::Value` folds a repeated member onto the last one written and would
    // hide it, so both are settled before the text becomes a value.
    if let Some(diagnostic) = probe_syntax(text) {
        return Err(diagnostic);
    }
    parse_json(text)
}

/// Reads a profile-conforming JSON document as an [`IRFile`], with the decoder's warnings.
pub fn read_ir_file(text: &str) -> Result<(IRFile, Vec<Warning>), DiagnosticError> {
    let value = read(text).map_err(DiagnosticError)?;
    crate::ir::v4::decode_ir_file_with_warnings(&value)
}

/// Writes an [`IRFile`] as canonical profile JSON, with no trailing newline.
///
/// The value tree is built under [`TypeEncoding::Compact`], which is the canonical spelling of a
/// type expression (decision 0005): a reference with no arguments and no attributes is
/// `morphir/SDK:basics#int`, not an expanded wrapper. The thread-local defaults to `Expanded`, so
/// a canonical writer has to say so — mirroring [`crate::ir::yaml::write_ir_file`].
pub fn write_ir_file(file: &IRFile) -> String {
    let value = with_type_encoding(TypeEncoding::Compact, || serde_json::to_value(file))
        .expect("an IRFile serialises");
    write_canonical(&value)
}

/// Writes a value in the JSON profile's canonical text form.
///
/// The profile's writer is not `serde_json::to_string`: a non-empty object is padded inside its
/// braces and its members separated by `, `, while an array is not padded. The driver compares
/// canonicals as strings (kit README, "What the driver does with a case"), so this is part of
/// the contract rather than a style.
pub fn write_canonical(value: &Json) -> String {
    match value {
        Json::Null => "null".to_string(),
        Json::Bool(true) => "true".to_string(),
        Json::Bool(false) => "false".to_string(),
        // What `arbitrary_precision` buys is that a number *parsed from text* keeps the lexeme
        // it was written with, which is what a `DocumentLiteral` payload needs. `Literal::Float`
        // now carries its lexeme too, so `1.0e2` comes back out as `1.0e2` rather than `100.0`.
        Json::Number(number) => number.to_string(),
        Json::String(_) => serde_json::to_string(value).expect("a string always serializes"),
        Json::Array(elements) if elements.is_empty() => "[]".to_string(),
        Json::Array(elements) => format!(
            "[{}]",
            elements
                .iter()
                .map(write_canonical)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Json::Object(members) if members.is_empty() => "{}".to_string(),
        Json::Object(members) => format!(
            "{{ {} }}",
            members
                .iter()
                .map(|(key, member)| format!(
                    "{}: {}",
                    serde_json::to_string(key).expect("a member name always serializes"),
                    write_canonical(member)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

// =============================================================================
// Parsing
// =============================================================================

/// Parses the input as JSON, with the ceiling this reader states rather than serde_json's own.
///
/// `disable_recursion_limit` needs the `unbounded_depth` feature; without it serde_json stops at
/// its own default of 128, which would report `invalid_json` for a document the profile admits.
/// The depth that matters is [`MAX_DEPTH`], and [`probe_syntax`] has already enforced it.
fn parse_json(text: &str) -> Result<Json, Diagnostic> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    deserializer.disable_recursion_limit();
    let value = Json::deserialize(&mut deserializer).map_err(invalid_json)?;
    deserializer.end().map_err(invalid_json)?;
    Ok(value)
}

fn invalid_json(error: serde_json::Error) -> Diagnostic {
    let mut diagnostic = Diagnostic::syntax(DiagnosticCode::InvalidJson, "/", error.to_string());
    diagnostic.line = u32::try_from(error.line()).ok();
    diagnostic.column = u32::try_from(error.column()).ok();
    diagnostic
}

// =============================================================================
// Duplicate members and nesting
// =============================================================================

/// The two syntactic rules a `serde_json::Value` cannot carry: no repeated object member, and no
/// more than [`MAX_DEPTH`] nested containers.
///
/// `Value` keeps one entry per key, so the second `"a"` in `{"a":1,"a":2}` is gone by the time a
/// value exists, and serde_json's own recursion limit fails before this reader's ceiling is
/// reached. The probe below walks the token stream instead, carrying the JSON pointer of where
/// it is, and stops at the first thing it finds: `duplicate_member` at the second occurrence, or
/// `nesting_too_deep` at the container that crossed the ceiling.
fn probe_syntax(text: &str) -> Option<Diagnostic> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    deserializer.disable_recursion_limit();
    match (Probe {
        cursor: String::new(),
        depth: 0,
    })
    .deserialize(&mut deserializer)
    {
        Ok(()) => None,
        // A syntax error is not this probe's to report: `parse_json` reports it with the line
        // and column serde_json gives, as `invalid_json`.
        Err(error) => Diagnostic::from_serde_error(&error),
    }
}

/// One position in the token stream: the JSON pointer of the value about to be read and how
/// many containers are already open around it.
///
/// The cursor is built the way the reference reader builds it: empty at the root, then
/// `<parent>/<member or index>` with the member name written out as it appears. A diagnostic at
/// the root reports `/` (see [`cursor_or_root`]).
///
/// This is a [`DeserializeSeed`] rather than a [`Deserialize`] because the cursor and the depth
/// have to travel *into* each member, and a `Deserialize` impl is handed nothing but the
/// deserializer.
struct Probe {
    cursor: String,
    depth: usize,
}

/// serde_json's `arbitrary_precision` feature carries a number through `deserialize_any` as a
/// one-member map under this reserved key, so the probe would otherwise count every number as a
/// container and read its lexeme as a member name.
///
/// The key alone does not make a map the token: a document literal is free to spell a member this
/// way. The whole shape does — exactly this one member, holding a string — and [`Probe::visit_map`]
/// checks the shape before it takes a map for a number. A map that is only shaped like the token
/// is indistinguishable from one at this layer, because it is exactly what serde_json emits for a
/// number, but anything else is walked like the ordinary object it is.
const NUMBER_TOKEN: &str = "$serde_json::private::Number";

impl<'de> DeserializeSeed<'de> for Probe {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Probe {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_map<A>(self, mut map: A) -> Result<(), A::Error>
    where
        A: MapAccess<'de>,
    {
        // The reserved-number check comes before the depth guard, not inside the loop: a number
        // is a scalar the profile counts at no depth at all, and charging it a nesting level
        // would make the ceiling depend on whether the innermost value happened to be a number.
        let Some(first) = map.next_key::<String>()? else {
            return self.enter::<A::Error>().map(|_| ());
        };

        let mut seen: HashSet<String> = HashSet::new();
        let depth;
        let mut key;

        if first == NUMBER_TOKEN {
            // Only the token's whole shape is the token. The value decides the first half of it,
            // and reading it also walks it when it turns out to belong to a user object, so the
            // level that object owes is charged there rather than here.
            match map.next_value_seed(NumberTokenValue { outer: &self })? {
                TokenValue::Lexeme => match map.next_key::<String>()? {
                    // Exactly one member, holding a string: serde_json's number token.
                    None => return Ok(()),
                    // A user object whose first member is spelled like the token and holds a
                    // string. The string carried nothing to walk, so only the level is still
                    // owed, and the rest of the members are read like any other object's.
                    Some(next) => {
                        depth = self.enter::<A::Error>()?;
                        seen.insert(first);
                        key = next;
                    }
                },
                TokenValue::Walked(walked) => {
                    depth = walked;
                    seen.insert(first);
                    match map.next_key::<String>()? {
                        Some(next) => key = next,
                        None => return Ok(()),
                    }
                }
            }
        } else {
            depth = self.enter::<A::Error>()?;
            key = first;
        }

        loop {
            // The member name goes in raw, not JSON-Pointer-escaped: the reference reader
            // (`packages/ir/src/codec/json/value.ts`) builds the cursor this way and the kit
            // README makes that reader the convention a binding mirrors.
            let cursor = format!("{}/{}", self.cursor, key);
            if !seen.insert(key.clone()) {
                return Err(carry(Diagnostic::syntax(
                    DiagnosticCode::DuplicateMember,
                    cursor,
                    format!("duplicate member \"{key}\""),
                )));
            }
            map.next_value_seed(Probe { cursor, depth })?;
            match map.next_key::<String>()? {
                Some(next) => key = next,
                None => return Ok(()),
            }
        }
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<(), A::Error>
    where
        A: SeqAccess<'de>,
    {
        let depth = self.enter::<A::Error>()?;
        let mut index = 0usize;
        while seq
            .next_element_seed(Probe {
                cursor: format!("{}/{index}", self.cursor),
                depth,
            })?
            .is_some()
        {
            index += 1;
        }
        Ok(())
    }

    fn visit_bool<E: serde::de::Error>(self, _value: bool) -> Result<(), E> {
        Ok(())
    }

    fn visit_i64<E: serde::de::Error>(self, _value: i64) -> Result<(), E> {
        Ok(())
    }

    fn visit_u64<E: serde::de::Error>(self, _value: u64) -> Result<(), E> {
        Ok(())
    }

    fn visit_f64<E: serde::de::Error>(self, _value: f64) -> Result<(), E> {
        Ok(())
    }

    fn visit_str<E: serde::de::Error>(self, _value: &str) -> Result<(), E> {
        Ok(())
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
}

/// What the value under a [`NUMBER_TOKEN`] key turned out to be.
enum TokenValue {
    /// A string, which is what serde_json puts a number's lexeme in.
    Lexeme,
    /// Anything else, so the map holding it is a user object. The value has already been walked,
    /// and the level that object owes has already been charged; this is the depth it was charged.
    Walked(usize),
}

/// Reads the value under a [`NUMBER_TOKEN`] key and says which of the two it was.
///
/// It cannot just look, because a value read is a value consumed: whatever this finds has to be
/// walked here or not at all. So the two answers are "a string, nothing to walk" and "walked it,
/// here is the depth I charged the object for".
struct NumberTokenValue<'probe> {
    /// The map the key was read from, at its own position — not yet entered.
    outer: &'probe Probe,
}

impl<'probe> NumberTokenValue<'probe> {
    /// The probe for the value, with the level the object owes charged.
    fn walker<E: serde::de::Error>(&self) -> Result<Probe, E> {
        Ok(Probe {
            cursor: format!("{}/{NUMBER_TOKEN}", self.outer.cursor),
            depth: self.outer.enter::<E>()?,
        })
    }
}

impl<'de, 'probe> DeserializeSeed<'de> for NumberTokenValue<'probe> {
    type Value = TokenValue;

    fn deserialize<D>(self, deserializer: D) -> Result<TokenValue, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

impl<'de, 'probe> Visitor<'de> for NumberTokenValue<'probe> {
    type Value = TokenValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_str<E: serde::de::Error>(self, _value: &str) -> Result<TokenValue, E> {
        Ok(TokenValue::Lexeme)
    }

    fn visit_map<A>(self, map: A) -> Result<TokenValue, A::Error>
    where
        A: MapAccess<'de>,
    {
        let walker = self.walker::<A::Error>()?;
        let depth = walker.depth;
        walker.visit_map(map)?;
        Ok(TokenValue::Walked(depth))
    }

    fn visit_seq<A>(self, seq: A) -> Result<TokenValue, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let walker = self.walker::<A::Error>()?;
        let depth = walker.depth;
        walker.visit_seq(seq)?;
        Ok(TokenValue::Walked(depth))
    }

    fn visit_bool<E: serde::de::Error>(self, _value: bool) -> Result<TokenValue, E> {
        self.walker::<E>()
            .map(|walker| TokenValue::Walked(walker.depth))
    }

    fn visit_i64<E: serde::de::Error>(self, _value: i64) -> Result<TokenValue, E> {
        self.walker::<E>()
            .map(|walker| TokenValue::Walked(walker.depth))
    }

    fn visit_u64<E: serde::de::Error>(self, _value: u64) -> Result<TokenValue, E> {
        self.walker::<E>()
            .map(|walker| TokenValue::Walked(walker.depth))
    }

    fn visit_f64<E: serde::de::Error>(self, _value: f64) -> Result<TokenValue, E> {
        self.walker::<E>()
            .map(|walker| TokenValue::Walked(walker.depth))
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<TokenValue, E> {
        self.walker::<E>()
            .map(|walker| TokenValue::Walked(walker.depth))
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<TokenValue, E> {
        self.walker::<E>()
            .map(|walker| TokenValue::Walked(walker.depth))
    }
}

impl Probe {
    /// Opens the container at this position, refusing the one that crosses the ceiling.
    fn enter<E: serde::de::Error>(&self) -> Result<usize, E> {
        let depth = self.depth + 1;
        if depth > MAX_DEPTH {
            return Err(carry(Diagnostic::syntax(
                DiagnosticCode::NestingTooDeep,
                cursor_or_root(&self.cursor),
                format!("nesting deeper than {MAX_DEPTH} is not accepted"),
            )));
        }
        Ok(depth)
    }
}

/// A cursor as a diagnostic reports it: the root is the whole document, spelled `/`.
fn cursor_or_root(cursor: &str) -> &str {
    if cursor.is_empty() { "/" } else { cursor }
}

fn carry<E: serde::de::Error>(diagnostic: Diagnostic) -> E {
    E::custom(DiagnosticError(diagnostic))
}
