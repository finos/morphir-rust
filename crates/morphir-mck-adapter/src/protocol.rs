//! Wire types and line framing for the Morphir Compatibility Kit's adapter
//! protocol, contract version 1.
//!
//! The normative source is `spec/ir/mck/protocol.schema.json` in the parent
//! repository (see `protocol.example.json` alongside it for a worked
//! exchange); every type here mirrors a definition from that schema, field
//! names and casing included. Every request payload struct rejects unknown
//! fields on the wire, matching the schema's `additionalProperties: false`.

use morphir_core::ir::{Diagnostic, DiagnosticCode, Warning};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// An incoming line's envelope: the request id, and everything else in the
/// object (including `op`) left un-interpreted so [`parse_line`] can dispatch
/// on `op` before validating the rest against the matching request shape.
#[derive(Debug, Clone, Deserialize)]
pub struct Envelope {
    pub id: u64,
    #[serde(flatten)]
    pub body: Map<String, Value>,
}

/// A parsed request, tagged by its `op` on the wire.
#[derive(Debug, Clone)]
pub enum Request {
    Capabilities,
    Decode(DecodeRequest),
    ReadTree(ReadTreeRequest),
    WriteTree(WriteTreeRequest),
    Exit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecodeRequest {
    pub version: u32,
    pub profile: Profile,
    pub path: PathMode,
    pub strip: bool,
    pub node: NodeKind,
    pub input: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadTreeRequest {
    pub version: u32,
    pub profile: Profile,
    pub path: PathMode,
    pub strip: bool,
    pub node: NodeKind,
    pub files: Vec<TreeFile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteTreeRequest {
    pub version: u32,
    pub path: PathMode,
    pub policy: WritePolicy,
    pub input: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WritePolicy {
    pub profile: Profile,
    pub path_budget: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    Json,
    Yaml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PathMode {
    Current,
    Pinned,
}

/// Node kinds the kit's driver names, spelled exactly as the kit spells them
/// (including the `FQName` and `IRFile` aliases).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    Name,
    Path,
    FQName,
    FormatVersion,
    Type,
    Literal,
    Pattern,
    Value,
    TypeSpecification,
    TypeDefinition,
    ValueSpecification,
    ValueDefinition,
    AccessControlledTypeDefinition,
    AccessControlledValueDefinition,
    ModuleDefinition,
    ModuleSpecification,
    IRFile,
    Distribution,
}

/// The stage-one capabilities this binding reports, without the envelope
/// `id` (the caller adds that; see [`capabilities`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub contract_version: u32,
    pub binding: String,
    pub language: String,
    pub versions: Vec<u32>,
    pub profiles: Vec<Profile>,
    pub layouts: Vec<String>,
    pub paths: Vec<PathMode>,
    pub nodes: Vec<NodeKind>,
}

/// The answer to `decode` or `readTree`.
#[derive(Debug, Clone)]
pub enum DecodeResponse {
    Ok {
        kind: NodeKind,
        canonical: BTreeMap<String, String>,
        warnings: Vec<Warning>,
    },
    Err {
        diagnostic: Diagnostic,
    },
}

impl Serialize for DecodeResponse {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        match self {
            DecodeResponse::Ok {
                kind,
                canonical,
                warnings,
            } => {
                let mut map = serializer.serialize_map(Some(4))?;
                map.serialize_entry("ok", &true)?;
                map.serialize_entry("kind", kind)?;
                map.serialize_entry("canonical", canonical)?;
                map.serialize_entry("warnings", warnings)?;
                map.end()
            }
            DecodeResponse::Err { diagnostic } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("ok", &false)?;
                map.serialize_entry("diagnostic", diagnostic)?;
                map.end()
            }
        }
    }
}

/// The stage-one capabilities this binding reports (spec section 4.2): IR
/// versions 3 and 4, the `json` profile only, the `single` layout, both path
/// modes, and every node kind the kit names.
pub fn capabilities() -> Capabilities {
    Capabilities {
        contract_version: 1,
        binding: "morphir-rust".to_string(),
        language: "rust".to_string(),
        versions: vec![3, 4],
        profiles: vec![Profile::Json],
        layouts: vec!["single".to_string()],
        paths: vec![PathMode::Current, PathMode::Pinned],
        nodes: vec![
            NodeKind::Name,
            NodeKind::Path,
            NodeKind::FQName,
            NodeKind::FormatVersion,
            NodeKind::Type,
            NodeKind::Literal,
            NodeKind::Pattern,
            NodeKind::Value,
            NodeKind::TypeSpecification,
            NodeKind::TypeDefinition,
            NodeKind::ValueSpecification,
            NodeKind::ValueDefinition,
            NodeKind::AccessControlledTypeDefinition,
            NodeKind::AccessControlledValueDefinition,
            NodeKind::ModuleDefinition,
            NodeKind::ModuleSpecification,
            NodeKind::IRFile,
            NodeKind::Distribution,
        ],
    }
}

/// A diagnostic for a line the adapter could not turn into a request: not
/// JSON at all, missing `id`, an unrecognized `op`, or a request payload
/// with a missing, mistyped or extra field. The cursor is the line's root
/// (`""`), since framing failures have no member to point at yet.
fn invalid_json(message: impl Into<String>) -> Diagnostic {
    Diagnostic::syntax(DiagnosticCode::InvalidJson, "", message)
}

/// Parses one line of the adapter protocol into its request id and request.
///
/// A line that is not valid JSON at all carries no id, so the error side
/// carries `None`. A line that is valid JSON with a readable `id` but an
/// invalid body (unknown `op`, a missing field, or an extra field the
/// matching request shape does not declare) carries `Some(id)`, so the
/// caller can still answer the request that asked.
pub fn parse_line(line: &str) -> Result<(u64, Request), (Option<u64>, Diagnostic)> {
    let value: Value =
        serde_json::from_str(line).map_err(|err| (None, invalid_json(err.to_string())))?;

    let id_hint = value.get("id").and_then(Value::as_u64);

    let envelope: Envelope =
        serde_json::from_value(value).map_err(|err| (id_hint, invalid_json(err.to_string())))?;

    let request = request_from_body(envelope.body)
        .map_err(|message| (Some(envelope.id), invalid_json(message)))?;

    Ok((envelope.id, request))
}

/// Dispatches on `op`, then validates the remaining fields against the
/// shape that op requires. `op` itself is removed before that validation so
/// a request struct's `deny_unknown_fields` only sees fields the wire
/// actually adds beyond `id` and `op`.
fn request_from_body(mut body: Map<String, Value>) -> Result<Request, String> {
    let op = body
        .remove("op")
        .and_then(|value| value.as_str().map(str::to_string))
        .ok_or_else(|| "missing \"op\"".to_string())?;

    match op.as_str() {
        "capabilities" => {
            reject_extra(&body)?;
            Ok(Request::Capabilities)
        }
        "exit" => {
            reject_extra(&body)?;
            Ok(Request::Exit)
        }
        "decode" => serde_json::from_value(Value::Object(body))
            .map(Request::Decode)
            .map_err(|err| err.to_string()),
        "readTree" => serde_json::from_value(Value::Object(body))
            .map(Request::ReadTree)
            .map_err(|err| err.to_string()),
        "writeTree" => serde_json::from_value(Value::Object(body))
            .map(Request::WriteTree)
            .map_err(|err| err.to_string()),
        other => Err(format!("unsupported op \"{other}\"")),
    }
}

/// `capabilities` and `exit` carry no fields beyond `id` and `op`; anything
/// left over is unknown, matching the schema's `additionalProperties: false`.
fn reject_extra(body: &Map<String, Value>) -> Result<(), String> {
    match body.keys().next() {
        None => Ok(()),
        Some(key) => Err(format!("unexpected field \"{key}\"")),
    }
}
