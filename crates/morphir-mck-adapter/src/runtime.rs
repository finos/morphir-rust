//! The JSON-lines framing loop for the Morphir Compatibility Kit's adapter
//! protocol (`protocol.schema.json` contract version 1; see
//! `protocol.example.json` for a worked exchange). [`run`] is generic over
//! its reader and writer so it can be driven from `main.rs` against real
//! stdin/stdout, or from a test against an in-memory buffer.
//!
//! `capabilities` and `exit` are answered here; `decode`, `readTree` and
//! `writeTree` all go to [`crate::testee`].

use crate::protocol::{ProtocolDiagnostic, Request, capabilities, parse_line};
use crate::testee::{decode, read_tree, write_tree};
use serde_json::{Map, Value};
use std::io::{self, BufRead, Write};

/// Reads JSON lines from `reader` until `exit` or end of input, writing one
/// JSON line per request to `writer`. Blank lines are skipped. A line that
/// cannot be turned into a request at all (not JSON, not an object, a bad
/// `id`, an unknown `op`, or a malformed request payload) still gets a
/// response line — `{ "id": <id or null>, "ok": false, "diagnostic": {...} }`
/// — rather than being reported anywhere else, so the driver on the other
/// end of the pipe always sees an answer for the line it sent.
pub fn run<R: BufRead, W: Write>(reader: R, mut writer: W) -> io::Result<()> {
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        match parse_line(&line) {
            Ok((id, Request::Capabilities)) => {
                write_line(&mut writer, capabilities_response(id))?;
            }
            Ok((_, Request::Exit)) => break,
            Ok((id, Request::Decode(request))) => {
                write_line(&mut writer, decode_response(id, &decode(&request)))?;
            }
            Ok((id, Request::ReadTree(request))) => {
                write_line(&mut writer, decode_response(id, &read_tree(&request)))?;
            }
            Ok((id, Request::WriteTree(request))) => {
                write_line(&mut writer, write_tree_response(id, &write_tree(&request)))?;
            }
            Err((id, diagnostic)) => {
                write_line(&mut writer, protocol_error_response(id, &diagnostic))?;
            }
        }
    }

    Ok(())
}

fn capabilities_response(id: u64) -> Value {
    let mut object = Map::new();
    object.insert("id".to_string(), Value::from(id));
    if let Value::Object(fields) =
        serde_json::to_value(capabilities()).expect("capabilities serialize")
    {
        object.extend(fields);
    }
    Value::Object(object)
}

/// The `decode` answer with the envelope's `id` merged in, the way
/// `capabilities` is: the response types carry the content, and the wire
/// envelope's `id` is added where the line is written.
fn decode_response(id: u64, response: &crate::protocol::DecodeResponse) -> Value {
    let mut object = Map::new();
    object.insert("id".to_string(), Value::from(id));
    if let Value::Object(fields) = serde_json::to_value(response).expect("decode response") {
        object.extend(fields);
    }
    Value::Object(object)
}

/// The `writeTree` answer with the envelope's `id` merged in, the same way [`decode_response`]
/// merges `decode`'s and `readTree`'s.
fn write_tree_response(id: u64, response: &crate::protocol::WriteTreeResponse) -> Value {
    let mut object = Map::new();
    object.insert("id".to_string(), Value::from(id));
    if let Value::Object(fields) = serde_json::to_value(response).expect("write tree response") {
        object.extend(fields);
    }
    Value::Object(object)
}

fn protocol_error_response(id: Option<u64>, diagnostic: &ProtocolDiagnostic) -> Value {
    let mut object = Map::new();
    object.insert("id".to_string(), id.map(Value::from).unwrap_or(Value::Null));
    object.insert("ok".to_string(), Value::from(false));
    object.insert(
        "diagnostic".to_string(),
        serde_json::to_value(diagnostic).expect("diagnostic serialize"),
    );
    Value::Object(object)
}

fn write_line(writer: &mut impl Write, response: Value) -> io::Result<()> {
    writeln!(writer, "{response}")?;
    writer.flush()
}
