//! `mck-adapter-rust`: the testee side of the Morphir Compatibility Kit's
//! JSON-lines adapter protocol for morphir-rust.
//!
//! Stage one answers `capabilities` and `exit` only; every other request
//! that framing accepts (`decode`, `readTree`, `writeTree`) is refused with
//! an `invalid_json` diagnostic until a later stage wires up
//! [`morphir_mck_adapter::testee`].

use anyhow::Result;
use morphir_core::ir::{Diagnostic, DiagnosticCode, DiagnosticStage};
use morphir_mck_adapter::protocol::{Request, capabilities, parse_line};
use serde_json::{Map, Value};
use std::io::{self, BufRead, Write};

fn main() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        match parse_line(&line) {
            Ok((id, Request::Capabilities)) => {
                write_response(&mut stdout, capabilities_response(id))?;
            }
            Ok((_, Request::Exit)) => break,
            Ok((id, Request::Decode(_) | Request::ReadTree(_) | Request::WriteTree(_))) => {
                write_response(&mut stdout, error_response(id, unsupported()))?;
            }
            Err((Some(id), diagnostic)) => {
                write_response(&mut stdout, error_response(id, diagnostic))?;
            }
            Err((None, diagnostic)) => {
                // No `id` was readable at all, so no schema-valid response can
                // name a request to answer; report it on stderr instead.
                eprintln!("{}", serde_json::to_string(&diagnostic)?);
            }
        }
    }

    Ok(())
}

/// The diagnostic stage one answers `decode`, `readTree` and `writeTree`
/// with, until Task 9 gives the adapter a codec to call.
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

fn error_response(id: u64, diagnostic: Diagnostic) -> Value {
    let mut object = Map::new();
    object.insert("id".to_string(), Value::from(id));
    object.insert("ok".to_string(), Value::from(false));
    object.insert(
        "diagnostic".to_string(),
        serde_json::to_value(diagnostic).expect("diagnostic serialize"),
    );
    Value::Object(object)
}

fn write_response(stdout: &mut impl Write, response: Value) -> Result<()> {
    writeln!(stdout, "{response}")?;
    stdout.flush()?;
    Ok(())
}
