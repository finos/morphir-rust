//! Framing for the draft-2 package resolution protocol.

use crate::package::positive_integer_id;
use anyhow::{Result, ensure};
use morphir_package::{resolution, strict_json};
use serde::Deserialize;
use serde_json::json;
use std::io::{BufRead, Write};

#[derive(Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
enum Request {
    #[serde(rename = "capabilities")]
    Capabilities {},
    #[serde(rename = "resolve-library")]
    ResolveLibrary { input: String },
    #[serde(rename = "exit")]
    Exit {},
}

/// Drive draft-2 package requests over JSON lines.
///
/// Malformed envelopes and execution failures fail the process. Completed
/// domain rejections are ordinary successful protocol responses.
pub fn run(reader: impl BufRead, mut writer: impl Write) -> Result<()> {
    let mut requests = 0usize;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        requests += 1;
        let mut value = strict_json::parse(&line)?;
        let id = value
            .as_object_mut()
            .and_then(|object| object.remove("id"))
            .filter(positive_integer_id)
            .ok_or_else(|| anyhow::anyhow!("package request id must be a positive integer"))?;
        let request: Request = serde_json::from_value(value)?;
        let mut response = match request {
            Request::Capabilities {} => json!({
                "suite": "package",
                "contractVersion": resolution::CONTRACT_VERSION,
                "implementation": "morphir-rust",
                "implementationVersion": env!("CARGO_PKG_VERSION"),
                "operations": ["resolve-library"],
                "profiles": ["flat-library"]
            }),
            Request::ResolveLibrary { input } => {
                serde_json::to_value(resolution::resolve_library(&input)?)?
            }
            Request::Exit {} => break,
        };
        response
            .as_object_mut()
            .expect("response is an object")
            .insert("id".into(), id);
        writeln!(writer, "{response}")?;
        writer.flush()?;
    }
    ensure!(requests > 0, "package protocol input was empty");
    Ok(())
}
