//! The separate experimental package JSON-lines protocol.

use anyhow::{Result, bail, ensure};
use morphir_package::library::{LibraryInput, VerifiedLibrarySet};
use morphir_package::{
    CONTRACT_VERSION,
    digest::Digest,
    metadata::NormalizedMetadata,
    schema::{Artifact, PackageSchemas},
    strict_json,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{BufRead, Write};

#[derive(Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
enum Request {
    #[serde(rename = "capabilities")]
    Capabilities {},
    #[serde(rename = "exit")]
    Exit {},
    #[serde(rename = "normalize")]
    Normalize { input: String },
    #[serde(rename = "hash-bytes")]
    HashBytes { hex: String },
    #[serde(rename = "validate")]
    Validate {
        artifact: WireArtifact,
        input: String,
        schemas: Schemas,
    },
    #[serde(rename = "verify-library-set")]
    Verify {
        lock: String,
        libraries: Vec<WireLibrary>,
        schemas: Schemas,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireLibrary {
    manifest: String,
    files: Vec<WireFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFile {
    path: String,
    hex: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum WireArtifact {
    Manifest,
    Lock,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Schemas {
    manifest: Value,
    lock: Value,
}

/// Drive package requests. Malformed protocol and schema errors fail the process.
/// Valid requests about invalid documents return document results on stdout.
pub fn run(reader: impl BufRead, mut writer: impl Write) -> Result<()> {
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let mut value = strict_json::parse(&line)?;
        let id = value
            .as_object_mut()
            .and_then(|object| object.remove("id"))
            .filter(positive_integer_id)
            .ok_or_else(|| anyhow::anyhow!("package request id must be a positive integer"))?;
        let request: Request = serde_json::from_value(value)?;
        let mut response = match request {
            Request::Verify {
                lock,
                libraries,
                schemas,
            } => {
                let libraries = libraries
                    .into_iter()
                    .map(|library| {
                        let files = library
                            .files
                            .into_iter()
                            .map(|file| {
                                ensure!(!file.path.is_empty(), "file path must be nonempty");
                                Ok((file.path, decode_hex(&file.hex)?))
                            })
                            .collect::<Result<Vec<_>>>()?;
                        Ok(LibraryInput::new(library.manifest, files))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let schemas = PackageSchemas::compile(&schemas.manifest, &schemas.lock)?;
                json!({"ok":true,"valid":VerifiedLibrarySet::verify(&schemas,&lock,&libraries).is_ok()})
            }
            Request::Exit {} => break,
            Request::Capabilities {} => {
                json!({"suite":"package","contractVersion":CONTRACT_VERSION,"implementation":"morphir-rust","implementationVersion":env!("CARGO_PKG_VERSION"),"operations":["normalize","hash-bytes","validate","verify-library-set"]})
            }
            Request::Normalize { input } => match NormalizedMetadata::parse(&input) {
                Ok(metadata) => {
                    json!({"ok":true,"canonical":metadata.as_str(),"manifestDigest":metadata.manifest_digest().to_string(),"packageContentDigest":metadata.content_digest().to_string()})
                }
                Err(_) => json!({"ok":false,"error":"invalid-document"}),
            },
            Request::HashBytes { hex } => {
                json!({"ok":true,"digest":Digest::of_bytes(&decode_hex(&hex)?).to_string()})
            }
            Request::Validate {
                artifact,
                input,
                schemas,
            } => {
                let schemas = PackageSchemas::compile(&schemas.manifest, &schemas.lock)?;
                let artifact = match artifact {
                    WireArtifact::Manifest => Artifact::Manifest,
                    WireArtifact::Lock => Artifact::Lock,
                };
                json!({"ok":true,"valid":schemas.validate(artifact,&input)})
            }
        };
        response
            .as_object_mut()
            .expect("response is an object")
            .insert("id".into(), id);
        writeln!(writer, "{response}")?;
        writer.flush()?;
    }
    Ok(())
}

// Inspect the exact JSON number lexeme. An integer has enough trailing mantissa
// zeroes plus exponent to cover its fractional digits. Exponents are compared as
// decimal strings, so even a huge exponent neither overflows nor expands digits.
fn positive_integer_id(id: &Value) -> bool {
    let Value::Number(number) = id else {
        return false;
    };
    let text = number.as_str();
    if text.starts_with('-') {
        return false;
    }
    let (mantissa, exponent) = text.split_once(['e', 'E']).unwrap_or((text, "0"));
    if !mantissa.bytes().any(|byte| (b'1'..=b'9').contains(&byte)) {
        return false;
    }
    let fractional = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let trailing_zeroes = mantissa
        .bytes()
        .rev()
        .filter(|byte| *byte != b'.')
        .take_while(|byte| *byte == b'0')
        .count();
    if let Some(magnitude) = exponent.strip_prefix('-') {
        trailing_zeroes >= fractional
            && compare_decimal(magnitude, trailing_zeroes - fractional).is_le()
    } else {
        trailing_zeroes >= fractional
            || compare_decimal(
                exponent.trim_start_matches('+'),
                fractional - trailing_zeroes,
            )
            .is_ge()
    }
}

fn compare_decimal(digits: &str, limit: usize) -> std::cmp::Ordering {
    let digits = digits.trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let limit = limit.to_string();
    digits
        .len()
        .cmp(&limit.len())
        .then_with(|| digits.cmp(&limit))
}

fn decode_hex(hex: &str) -> Result<Vec<u8>> {
    if !hex.len().is_multiple_of(2)
        || !hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        bail!("expected even-length lowercase hexadecimal bytes");
    }
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| Ok(u8::from_str_radix(std::str::from_utf8(pair)?, 16)?))
        .collect()
}
