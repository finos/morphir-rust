//! The JSON-lines framing loop for the Morphir Compatibility Kit's adapter
//! protocol (`protocol.schema.json` contract version 1; see
//! `protocol.example.json` for a worked exchange). [`run`] is generic over
//! its reader and writer so it can be driven from `main.rs` against real
//! stdin/stdout, or from a test against an in-memory buffer.
//!
//! This stage answers `capabilities` and `exit` directly. `decode`,
//! `readTree` and `writeTree` are accepted by framing but the decode
//! operation is not implemented yet, so each answers with a diagnostic
//! saying so; a later change will route them to [`crate::testee`] instead.

use crate::protocol::{ProtocolDiagnostic, Request, capabilities, parse_line};
use morphir_core::ir::{Diagnostic, DiagnosticCode, DiagnosticStage};
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
            Ok((id, Request::Decode(_) | Request::ReadTree(_) | Request::WriteTree(_))) => {
                write_line(&mut writer, unsupported_response(id))?;
            }
            Err((id, diagnostic)) => {
                write_line(&mut writer, protocol_error_response(id, &diagnostic))?;
            }
        }
    }

    Ok(())
}

/// The diagnostic this stage answers `decode`, `readTree` and `writeTree`
/// with: the request parsed fine, but the decode operation is not
/// implemented yet.
fn unsupported() -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::InvalidJson,
        DiagnosticStage::Syntax,
        "",
        "unsupported in this stage",
    )
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

fn unsupported_response(id: u64) -> Value {
    let mut object = Map::new();
    object.insert("id".to_string(), Value::from(id));
    object.insert("ok".to_string(), Value::from(false));
    object.insert(
        "diagnostic".to_string(),
        serde_json::to_value(unsupported()).expect("diagnostic serialize"),
    );
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
