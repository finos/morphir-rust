//! Testee side of the exact-draft linked-metadata MCK protocol.
//! Claims are limited to operations actually implemented and independently
//! checked against the parent's literal corpus.

use anyhow::{Result, anyhow};
use base64::Engine as _;
use morphir_core::metadata::{
    ContextResources, Fact, ObjectTerm, expand_properties, resolve_context,
};
use morphir_core::node_address::NodeUri;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};

const CONTRACT: &str = "0.1.0-draft.1";

#[derive(Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
enum Request {
    #[serde(rename = "capabilities")]
    Capabilities { id: u64 },
    #[serde(rename = "run")]
    Run {
        id: u64,
        #[serde(rename = "caseId")]
        case_id: String,
        operation: String,
        targets: Vec<Value>,
        given: Value,
        #[serde(rename = "schemaClosure")]
        schema_closure: String,
        fixtures: Vec<Fixture>,
    },
    #[serde(rename = "exit")]
    Exit { id: u64 },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Fixture {
    path: String,
    sha256: String,
    content_base64: String,
}

/// Process JSON-lines requests without leaking diagnostics to stdout.
pub fn run(reader: impl BufRead, mut writer: impl Write) -> Result<()> {
    for line in reader.lines() {
        let line = line?;
        if line.is_empty() {
            return Err(anyhow!("blank metadata request line"));
        }
        let wire = morphir_core::ir::json::read(&line)
            .map_err(|error| anyhow!("invalid metadata JSON request: {error:?}"))?;
        let request: Request = serde_json::from_value(wire)?;
        let response = match request {
            Request::Capabilities { id } => {
                check_id(id)?;
                json!({"id":id,"suite":"metadata","contractVersion":CONTRACT,
                    "implementation":"morphir-rust","implementationVersion":env!("CARGO_PKG_VERSION"),
                    "claims":[{"operation":"compareFacts","profile":"json","layout":"single","irRevision":"4.1.0"}]})
            }
            Request::Run {
                id,
                case_id,
                operation,
                targets,
                given,
                schema_closure,
                fixtures,
            } => {
                check_id(id)?;
                if operation != "compareFacts" {
                    json!({"id":id,"ok":false,"error":{"code":"unsupported_operation",
                        "message":"this adapter currently implements compareFacts only"}})
                } else {
                    match compare_facts(&case_id, &targets, &given, &schema_closure, &fixtures) {
                        Ok(observation) => json!({"id":id,"ok":true,"observation":observation}),
                        Err(message) => {
                            json!({"id":id,"ok":false,"error":{"code":"invalid_request","message":message}})
                        }
                    }
                }
            }
            Request::Exit { id } => {
                check_id(id)?;
                break;
            }
        };
        writeln!(writer, "{response}")?;
        writer.flush()?;
    }
    Ok(())
}

fn check_id(id: u64) -> Result<()> {
    if id == 0 {
        Err(anyhow!("metadata request id must be positive"))
    } else {
        Ok(())
    }
}

fn compare_facts(
    case_id: &str,
    targets: &[Value],
    given: &Value,
    schema_closure: &str,
    fixtures: &[Fixture],
) -> Result<Value, String> {
    if case_id.len() != 13
        || !case_id.starts_with("metadata-")
        || !case_id[9..].bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("invalid metadata case id".into());
    }
    if targets != [json!({"profile":"json","layout":"single","irRevision":"4.1.0"})] {
        return Err("compareFacts target is not supported".into());
    }
    let closure = read_closure(schema_closure, fixtures)?;
    let left = expand_side(&given["left"], &closure)?;
    let right = expand_side(&given["right"], &closure)?;
    let equal = same_facts(&left, &right);
    let mut distinct = left;
    for fact in right {
        if !distinct.contains(&fact) {
            distinct.push(fact);
        }
    }
    let facts = distinct.iter().map(fact_wire).collect::<Vec<_>>();
    Ok(json!({"outcome":"accepted","equal":equal,"distinctFacts":facts.len(),"facts":facts}))
}

fn read_closure(path: &str, fixtures: &[Fixture]) -> Result<HashMap<String, NodeUri>, String> {
    let mut seen = HashSet::new();
    let mut selected = None;
    for fixture in fixtures {
        if !fixture.path.starts_with("metadata-fixtures/")
            || fixture
                .path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || !seen.insert(&fixture.path)
        {
            return Err("invalid or duplicate fixture path".into());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&fixture.content_base64)
            .map_err(|_| "invalid fixture base64")?;
        let actual = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if fixture.sha256 != actual {
            return Err("fixture digest mismatch".into());
        }
        if fixture.path == path {
            selected = Some(bytes);
        }
    }
    let bytes = selected.ok_or("schema closure fixture missing")?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "schema closure is not UTF-8")?;
    let value = morphir_core::ir::json::read(text).map_err(|_| "invalid schema closure JSON")?;
    let predicates = value["predicates"]
        .as_array()
        .ok_or("schema closure predicates missing")?;
    let mut result = HashMap::new();
    for declaration in predicates {
        if declaration["object"]["kind"] == "json" {
            let uri = declaration["uri"]
                .as_str()
                .ok_or("invalid predicate declaration")?;
            let datatype = declaration["object"]["type"]
                .as_str()
                .ok_or("invalid JSON datatype")?;
            let datatype = NodeUri::parse(datatype).map_err(|_| "invalid JSON datatype URI")?;
            if result.insert(uri.to_owned(), datatype).is_some() {
                return Err("duplicate predicate declaration".into());
            }
        }
    }
    Ok(result)
}

fn expand_side(side: &Value, json_types: &HashMap<String, NodeUri>) -> Result<Vec<Fact>, String> {
    let subject = NodeUri::parse(side["owner"].as_str().ok_or("missing fact owner")?)
        .map_err(|_| "invalid fact owner")?;
    if !matches!(
        side["carrier"].as_str(),
        Some("attributesFacts" | "annotationsFacts" | "documentGraph")
    ) {
        return Err("invalid fact carrier".into());
    }
    let resources = ContextResources::new("metadata-fixtures");
    let context = resolve_context(None, &side["context"], &resources, None)
        .map_err(|error| error.to_string())?;
    let facts = side["facts"].as_object().ok_or("missing facts object")?;
    let mut expanded = expand_properties(
        &subject,
        facts.iter().map(|(key, value)| (key.as_str(), value)),
        &context,
        |predicate| json_types.get(&predicate.to_string()).cloned(),
    )
    .map_err(|error| error.to_string())?;
    expanded.dedup();
    Ok(expanded)
}

fn same_facts(left: &[Fact], right: &[Fact]) -> bool {
    left.len() == right.len() && left.iter().all(|fact| right.contains(fact))
}

fn fact_wire(fact: &Fact) -> Value {
    let object = match fact.object() {
        ObjectTerm::NodeRef(uri) => json!({"@id":uri.to_string()}),
        ObjectTerm::Value(value) => match value.datatype() {
            Some(_) => json!({"@value":value.value(),"@type":"@json"}),
            None => json!({"@value":value.value()}),
        },
    };
    json!({"subject":fact.subject().to_string(),"predicate":fact.predicate().to_string(),
        "object":object,"graph":"default"})
}
