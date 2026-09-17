//! Wire types and line framing for the Morphir Compatibility Kit's adapter
//! protocol, contract version 1.
//!
//! The normative source is `spec/ir/mck/protocol.schema.json` in the parent
//! repository (see `protocol.example.json` alongside it for a worked
//! exchange); every type here mirrors a definition from that schema, field
//! names and casing included. Every request payload struct rejects unknown
//! fields on the wire, matching the schema's `additionalProperties: false`.

use morphir_core::format_version::SupportTable;
use morphir_core::ir::{Diagnostic, Warning};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

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
    // The four files a document tree is made of. They are node kinds in their own right, so a kit
    // case can pin one on its own without a tree around it.
    DistributionManifestFile,
    ModuleManifestFile,
    TypeDefinitionFile,
    ValueDefinitionFile,
}

/// The stage-one capabilities this binding reports, without the envelope
/// `id` (the caller adds that; see [`capabilities`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub contract_version: u32,
    pub binding: String,
    pub language: String,
    /// The canonical spelling of this binding's format-version support table,
    /// e.g. `[3.0.0,3.1.0),[4.0.0,4.1.0)`. `rename_all = "camelCase"` spells
    /// the member `formatVersions`, as `protocol.schema.json` requires.
    pub format_versions: String,
    pub versions: Vec<u32>,
    pub profiles: Vec<Profile>,
    pub layouts: Vec<String>,
    pub paths: Vec<PathMode>,
    pub nodes: Vec<NodeKind>,
}

/// The answer to `decode` or `readTree`.
///
/// `kind` is a string rather than a [`NodeKind`] because it answers a different question from
/// the request's `node`: the kit's `rejected expect=<Kind>` fences name the *variant* a node
/// decoded to (a `List` at a value position, say), not the node kind that was asked for, and
/// `protocol.schema.json` types the member as a plain string for that reason.
#[derive(Debug, Clone)]
pub enum DecodeResponse {
    Ok {
        kind: String,
        canonical: BTreeMap<String, String>,
        warnings: Vec<Warning>,
    },
    Err {
        diagnostic: Diagnostic,
    },
    /// The request asked for something this binding said it could not do (a profile or an IR
    /// version outside its `capabilities`). That is not a statement about the document, so it
    /// answers `protocol_error` rather than spending one of the kit's diagnostic codes. On the
    /// wire it is the same `{ ok: false, diagnostic }` shape as [`DecodeResponse::Err`].
    Refused {
        diagnostic: ProtocolDiagnostic,
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
                map.serialize_entry("diagnostic", &WireDiagnostic(diagnostic))?;
                map.end()
            }
            DecodeResponse::Refused { diagnostic } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("ok", &false)?;
                map.serialize_entry("diagnostic", diagnostic)?;
                map.end()
            }
        }
    }
}

/// The answer to `writeTree`: a `WriteTreeSuccess` (`{ ok: true, files: [{ path, content }] }`) or
/// the shared `ErrorResponse` shape, exactly like [`DecodeResponse`].
#[derive(Debug, Clone)]
pub enum WriteTreeResponse {
    Ok {
        files: Vec<TreeFile>,
    },
    Err {
        diagnostic: Diagnostic,
    },
    /// The request asked for an IR version this binding does not write trees for. See
    /// [`DecodeResponse::Refused`].
    Refused {
        diagnostic: ProtocolDiagnostic,
    },
}

impl Serialize for WriteTreeResponse {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        match self {
            WriteTreeResponse::Ok { files } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("ok", &true)?;
                map.serialize_entry("files", files)?;
                map.end()
            }
            WriteTreeResponse::Err { diagnostic } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("ok", &false)?;
                map.serialize_entry("diagnostic", &WireDiagnostic(diagnostic))?;
                map.end()
            }
            WriteTreeResponse::Refused { diagnostic } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("ok", &false)?;
                map.serialize_entry("diagnostic", diagnostic)?;
                map.end()
            }
        }
    }
}

/// A [`Diagnostic`] as the protocol carries one: `code`, `stage`, `cursor` and `message`, and
/// nothing else.
///
/// morphir-core's diagnostic also carries the `line` and `column` a syntax failure was found at,
/// which are for a person reading a file, not for the driver — and `protocol.schema.json`'s
/// `Diagnostic` is `additionalProperties: false`, so sending them makes the whole response
/// unreadable to the driver rather than merely verbose. The YAML reader locates every diagnostic
/// it raises, so this is the difference between the kit's yaml fences being adjudicated and the
/// adapter being declared unavailable.
struct WireDiagnostic<'a>(&'a Diagnostic);

impl Serialize for WireDiagnostic<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        // Destructured rather than read member by member: a new field on the core `Diagnostic`
        // has to be decided about here — sent or deliberately dropped — and this makes that a
        // compile error instead of a response the driver silently cannot read.
        let Diagnostic {
            code,
            stage,
            cursor,
            message,
            line: _,
            column: _,
        } = self.0;
        let mut map = serializer.serialize_map(Some(4))?;
        map.serialize_entry("code", code)?;
        map.serialize_entry("stage", stage)?;
        map.serialize_entry("cursor", cursor)?;
        map.serialize_entry("message", message)?;
        map.end()
    }
}

/// The stage-one capabilities this binding reports, per `protocol.schema.json`
/// contract version 1 and the worked exchange in `protocol.example.json`: IR
/// versions 3 and 4 with this reader's support table as `formatVersions`,
/// the `json` and `yaml` profiles, the `single` layout, both path
/// modes, and every node kind the kit names.
pub fn capabilities() -> Capabilities {
    Capabilities {
        contract_version: 1,
        binding: "morphir-rust".to_string(),
        language: "rust".to_string(),
        format_versions: SupportTable::reference().canonical(),
        versions: vec![3, 4],
        profiles: vec![Profile::Json, Profile::Yaml],
        layouts: vec!["single".to_string(), "tree".to_string()],
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
            NodeKind::DistributionManifestFile,
            NodeKind::ModuleManifestFile,
            NodeKind::TypeDefinitionFile,
            NodeKind::ValueDefinitionFile,
        ],
    }
}

/// A protocol-level failure: the line could not be turned into a request at
/// all (not JSON, not an object, a missing or non-integer `id`, an unknown
/// `op`, or a request payload with a missing or extra field). This is a
/// framing failure, not one of morphir-core's kit diagnostic codes, so it
/// gets its own `protocol_error` code rather than widening
/// [`morphir_core::ir::DiagnosticCode`]. The cursor is the line's root
/// (`"/"`), since a framing failure has no member to point at yet.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProtocolDiagnostic {
    pub code: String,
    pub stage: String,
    pub cursor: String,
    pub message: String,
}

impl ProtocolDiagnostic {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            code: "protocol_error".to_string(),
            stage: "syntax".to_string(),
            cursor: "/".to_string(),
            message: message.into(),
        }
    }
}

/// Parses one line of the adapter protocol into its request id and request.
///
/// An `id` is only ever reported once it is fully valid (an integer of at
/// least 1, per `protocol.schema.json`'s `"id": { "minimum": 1 }`), matching
/// the reference adapter's rule that a bad envelope answers with `id: null`
/// even when a number happened to be present. Once the envelope's `id` is
/// valid, a further failure (an unknown `op`, or a request payload with a
/// missing or extra field) still names that `id`, so the caller can answer
/// the request that asked.
pub fn parse_line(line: &str) -> Result<(u64, Request), (Option<u64>, ProtocolDiagnostic)> {
    let value: Value = serde_json::from_str(line).map_err(|err| {
        (
            None,
            ProtocolDiagnostic::new(format!("not a JSON line: {err}")),
        )
    })?;

    let mut object = match value {
        Value::Object(map) => map,
        _ => {
            return Err((
                None,
                ProtocolDiagnostic::new("message must be a JSON object"),
            ));
        }
    };

    let id = extract_id(&object).map_err(|message| (None, ProtocolDiagnostic::new(message)))?;
    object.remove("id");

    let request = request_from_body(object)
        .map_err(|message| (Some(id), ProtocolDiagnostic::new(message)))?;

    Ok((id, request))
}

/// `id` must be present and an integer, and at least 1 (ids start at 1 and
/// increase by one, so 0 and negatives are not ids at all — the same rule
/// `protocol.schema.json` states as `"id": { "minimum": 1 }`).
fn extract_id(object: &Map<String, Value>) -> Result<u64, String> {
    match object.get("id") {
        None => Err("missing id".to_string()),
        Some(value) => match value.as_i64() {
            None => Err("missing id".to_string()),
            Some(id) if id < 1 => Err(format!("\"id\" must be at least 1, got {id}")),
            Some(id) => Ok(id as u64),
        },
    }
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
