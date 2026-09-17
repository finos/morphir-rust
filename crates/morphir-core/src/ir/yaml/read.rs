//! The YAML profile reader.
//!
//! See `docs/spec/ir/schemas/v4/yaml-profile.md`: this walks `granit-parser`'s events and builds
//! the value tree the JSON reader builds, refusing what the profile forbids with the kit's
//! diagnostic codes and JSON-pointer cursors. Numbers keep their lexeme through serde_json's
//! `arbitrary_precision`.

use super::scalar::{Resolved, resolve_plain};
use crate::ir::{Diagnostic, DiagnosticCode, DiagnosticStage};
use granit_parser::{Event, Marker, Options, Parser, ScalarStyle, Span};
use serde_json::{Map, Value};

/// The nesting bound the reader enforces, the same bound the JSON probe uses.
pub const MAX_DEPTH: usize = 1000;

enum Frame {
    Seq(Vec<Value>),
    Map {
        members: Map<String, Value>,
        pending_key: Option<String>,
    },
}

/// What [`Reader::place`] found wrong, decided while the frame stack is borrowed and reported
/// once that borrow has ended and the cursor can be read.
enum Fault {
    DuplicateMember(String),
    NonStringKey(&'static str),
}

struct Reader {
    stack: Vec<Frame>,
    root: Option<Value>,
    documents: usize,
}

fn diagnostic(
    code: DiagnosticCode,
    cursor: &str,
    message: impl Into<String>,
    at: Option<&Marker>,
) -> Diagnostic {
    let mut d = Diagnostic::new(code, DiagnosticStage::Syntax, cursor, message);
    if let Some(marker) = at {
        d.line = Some(marker.line() as u32);
        d.column = Some(marker.col() as u32 + 1);
    }
    d
}

fn at(span: &Span) -> Option<&Marker> {
    Some(&span.start)
}

/// Whether the stream opens with a `%YAML` or `%TAG` directive.
///
/// The parser reports a `%YAML` version on its `DocumentStart` event but resolves `%TAG` silently,
/// so the profile's blanket refusal of directives is decided on the source text.
fn has_directive(text: &str) -> bool {
    // A stream may open with a byte order mark, which is not part of the first line's content: a
    // `%TAG` after it is still a directive, and the parser resolves one silently, so a document
    // whose declared handle is never used would otherwise slip past this refusal.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    for line in text.lines() {
        // A directive is only ever unindented, so the `%` has to be the line's first byte. An
        // indented `%` opens a plain scalar, which the profile has no quarrel with.
        if line.starts_with('%') {
            return true;
        }
        let t = line.trim_start();
        if t.starts_with("---") || (!t.is_empty() && !t.starts_with('#')) {
            return false;
        }
    }
    false
}

fn kind_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a sequence",
        Value::Object(_) => "a mapping",
    }
}

impl Reader {
    /// The JSON pointer of the node about to be placed, with raw member names.
    fn cursor(&self) -> String {
        let mut out = String::new();
        for frame in &self.stack {
            match frame {
                Frame::Seq(items) => {
                    out.push('/');
                    out.push_str(&items.len().to_string());
                }
                Frame::Map {
                    pending_key: Some(key),
                    ..
                } => {
                    out.push('/');
                    out.push_str(key);
                }
                Frame::Map {
                    pending_key: None, ..
                } => {}
            }
        }
        out
    }

    /// Whether the next node lands in a mapping's key position.
    fn in_key_position(&self) -> bool {
        matches!(
            self.stack.last(),
            Some(Frame::Map {
                pending_key: None,
                ..
            })
        )
    }

    fn place(&mut self, value: Value, span: &Span) -> Result<(), Diagnostic> {
        // `place` runs once per node, so the cursor — which walks the frame stack building a
        // string — is computed only when one of the two faults is being reported. The stack is
        // left exactly as the cursor of the offending node needs it: the frame holding a repeated
        // member keeps its pending key, and a non-string key never took one.
        let fault = match self.stack.last_mut() {
            None => {
                self.root = Some(value);
                None
            }
            Some(Frame::Seq(items)) => {
                items.push(value);
                None
            }
            Some(Frame::Map {
                members,
                pending_key,
            }) => match pending_key.take() {
                Some(key) => {
                    if members.contains_key(&key) {
                        *pending_key = Some(key.clone());
                        Some(Fault::DuplicateMember(key))
                    } else {
                        members.insert(key, value);
                        None
                    }
                }
                None => match value {
                    Value::String(key) => {
                        *pending_key = Some(key);
                        None
                    }
                    other => Some(Fault::NonStringKey(kind_name(&other))),
                },
            },
        };
        match fault {
            None => Ok(()),
            Some(Fault::DuplicateMember(key)) => Err(diagnostic(
                DiagnosticCode::DuplicateMember,
                &self.cursor(),
                format!("member \"{key}\" appears twice"),
                at(span),
            )),
            Some(Fault::NonStringKey(kind)) => Err(diagnostic(
                DiagnosticCode::InvalidType,
                &self.cursor(),
                format!("mapping keys must be strings, got {kind}"),
                at(span),
            )),
        }
    }
}

/// Reads one profile-conforming YAML document into the JSON value tree.
pub fn read(text: &str) -> Result<Value, Diagnostic> {
    if has_directive(text) {
        return Err(diagnostic(
            DiagnosticCode::UnsupportedYamlFeature,
            "",
            "%YAML and %TAG directives are not part of the profile",
            None,
        ));
    }
    let mut options = Options::default();
    options.emit_comments = false;
    // The reader's own bound is the one that has to bite, so the parser's limits sit above it.
    options.flow_nesting_limit = MAX_DEPTH + 8;
    options.block_nesting_limit = MAX_DEPTH + 8;

    let mut r = Reader {
        stack: Vec::new(),
        root: None,
        documents: 0,
    };
    for item in Parser::new_from_str_with_options(text, options) {
        let (event, span) = item.map_err(|e| {
            // `info()` is the parser's own description; its `Display` would repeat the position
            // that `line` and `column` already carry.
            diagnostic(
                DiagnosticCode::InvalidYaml,
                &r.cursor(),
                e.info(),
                Some(e.marker()),
            )
        })?;
        match event {
            Event::StreamStart | Event::StreamEnd | Event::DocumentEnd | Event::Comment(..) => {}
            Event::DocumentStart(_, version) => {
                r.documents += 1;
                if version.is_some() {
                    return Err(diagnostic(
                        DiagnosticCode::UnsupportedYamlFeature,
                        "",
                        "%YAML directives are not part of the profile",
                        at(&span),
                    ));
                }
                if r.documents > 1 {
                    return Err(diagnostic(
                        DiagnosticCode::InvalidYaml,
                        "",
                        "expected exactly one document",
                        at(&span),
                    ));
                }
            }
            Event::Alias(_) => {
                return Err(diagnostic(
                    DiagnosticCode::UnsupportedYamlFeature,
                    &r.cursor(),
                    "anchors and aliases are not part of the profile",
                    at(&span),
                ));
            }
            Event::Scalar(scalar, style, anchor, tag) => {
                if anchor != 0 {
                    return Err(diagnostic(
                        DiagnosticCode::UnsupportedYamlFeature,
                        &r.cursor(),
                        "anchors and aliases are not part of the profile",
                        at(&span),
                    ));
                }
                if tag.is_some() {
                    return Err(diagnostic(
                        DiagnosticCode::UnsupportedYamlFeature,
                        &r.cursor(),
                        "tags are not part of the profile",
                        at(&span),
                    ));
                }
                if r.in_key_position() {
                    // A quoted or block scalar is its own text; a plain one has to resolve to a
                    // string, so `1:` or `true:` is a non-string key rather than a key spelled
                    // `"1"`. Only a plain `<<` is a merge key.
                    let key = match style {
                        ScalarStyle::Plain => {
                            if scalar == "<<" {
                                // The key's own position, as the reference reader reports it.
                                let cursor = format!("{}/<<", r.cursor());
                                return Err(diagnostic(
                                    DiagnosticCode::UnsupportedYamlFeature,
                                    &cursor,
                                    "merge keys are not part of the profile",
                                    at(&span),
                                ));
                            }
                            match resolve_plain(&scalar) {
                                Ok(Resolved::Str(text)) => text,
                                Ok(_) => {
                                    return Err(diagnostic(
                                        DiagnosticCode::InvalidType,
                                        &r.cursor(),
                                        "mapping keys must be strings",
                                        at(&span),
                                    ));
                                }
                                Err(message) => {
                                    return Err(diagnostic(
                                        DiagnosticCode::InvalidLiteral,
                                        &r.cursor(),
                                        message,
                                        at(&span),
                                    ));
                                }
                            }
                        }
                        _ => scalar.into_owned(),
                    };
                    r.place(Value::String(key), &span)?;
                    continue;
                }
                let value = match style {
                    ScalarStyle::Plain => match resolve_plain(&scalar) {
                        Ok(Resolved::Null) => Value::Null,
                        Ok(Resolved::Bool(b)) => Value::Bool(b),
                        Ok(Resolved::Number(n)) => Value::Number(n),
                        Ok(Resolved::Str(s)) => Value::String(s),
                        Err(message) => {
                            return Err(diagnostic(
                                DiagnosticCode::InvalidLiteral,
                                &r.cursor(),
                                message,
                                at(&span),
                            ));
                        }
                    },
                    _ => Value::String(scalar.into_owned()),
                };
                r.place(value, &span)?;
            }
            Event::SequenceStart(_, anchor, ref tag) | Event::MappingStart(_, anchor, ref tag) => {
                if anchor != 0 || tag.is_some() {
                    return Err(diagnostic(
                        DiagnosticCode::UnsupportedYamlFeature,
                        &r.cursor(),
                        "anchors and tags are not part of the profile",
                        at(&span),
                    ));
                }
                if r.in_key_position() {
                    return Err(diagnostic(
                        DiagnosticCode::InvalidType,
                        &r.cursor(),
                        "mapping keys must be strings",
                        at(&span),
                    ));
                }
                if r.stack.len() >= MAX_DEPTH {
                    return Err(diagnostic(
                        DiagnosticCode::NestingTooDeep,
                        &r.cursor(),
                        format!("nesting deeper than {MAX_DEPTH} levels"),
                        at(&span),
                    ));
                }
                r.stack.push(match event {
                    Event::SequenceStart(..) => Frame::Seq(Vec::new()),
                    _ => Frame::Map {
                        members: Map::new(),
                        pending_key: None,
                    },
                });
            }
            Event::SequenceEnd | Event::MappingEnd => {
                let frame = r.stack.pop().expect("the parser balances its events");
                let value = match frame {
                    Frame::Seq(items) => Value::Array(items),
                    Frame::Map { members, .. } => Value::Object(members),
                };
                r.place(value, &span)?;
            }
            _ => {}
        }
    }
    match r.root {
        Some(value) if r.documents == 1 => Ok(value),
        _ => Err(diagnostic(
            DiagnosticCode::InvalidYaml,
            "",
            "expected exactly one document",
            None,
        )),
    }
}
