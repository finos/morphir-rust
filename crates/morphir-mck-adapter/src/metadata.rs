//! Testee side of the exact-draft linked-metadata MCK protocol.
//! Claims are limited to operations actually implemented and independently
//! checked against the parent's literal corpus.

use anyhow::{Result, anyhow};
use base64::Engine as _;
use morphir_core::ir::v4::{DocumentGraphError, DocumentMeta, expand_document_graph};
use morphir_core::metadata::{
    Assertion, AssertionKey, AssertionSource, Carrier, ContextResources, DocumentId, Fact,
    GraphIndex, GraphName, MetadataError, ObjectTerm, expand_properties, resolve_context,
};
use morphir_core::node_address::{NodeUri, Sha256Digest};
use morphir_package::authoring::{AuthoredLibrary, PublicationBindings};
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
                "claims":[
                    {"operation":"compareFacts","profile":"json","layout":"single","irRevision":"4.1.0"},
                    {"operation":"resolveSources","profile":"json","layout":"single","irRevision":"4.1.0"},
                    {"operation":"publish","profile":"json","layout":"single","irRevision":"4.1.0"},
                    {"operation":"sourceEdit","profile":"json","layout":"single","irRevision":"4.1.0"}
                ]})
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
                if operation != "compareFacts"
                    && operation != "resolveSources"
                    && operation != "publish"
                    && operation != "sourceEdit"
                {
                    json!({"id":id,"ok":false,"error":{"code":"unsupported_operation",
                        "message":"this metadata operation is not implemented"}})
                } else {
                    let observed = match operation.as_str() {
                        "compareFacts" => {
                            compare_facts(&case_id, &targets, &given, &schema_closure, &fixtures)
                        }
                        "resolveSources" => {
                            resolve_sources(&case_id, &targets, &given, &schema_closure, &fixtures)
                        }
                        "publish" => {
                            publish(&case_id, &targets, &given, &schema_closure, &fixtures)
                        }
                        "sourceEdit" => {
                            source_edit(&case_id, &targets, &given, &schema_closure, &fixtures)
                        }
                        _ => unreachable!("unsupported operations returned above"),
                    };
                    match observed {
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

fn resolve_sources(
    case_id: &str,
    targets: &[Value],
    given: &Value,
    schema_closure: &str,
    fixtures: &[Fixture],
) -> Result<Value, String> {
    if !valid_case_id(case_id) {
        return Err("invalid metadata case id".into());
    }
    if targets != [json!({"profile":"json","layout":"single","irRevision":"4.1.0"})] {
        return Err("resolveSources target is not supported".into());
    }
    let closure = read_closure(schema_closure, fixtures)?;
    let owner = DocumentId::new(
        given["ownerDocument"]
            .as_str()
            .ok_or("missing owner document")?,
    )
    .map_err(|error| error.to_string())?;
    let metadata = match DocumentMeta::parse(&given["$meta"]) {
        Ok(metadata) => metadata,
        Err(message) if message.contains("duplicate assertion source selector") => {
            return Ok(json!({"outcome":"rejected","diagnostic":"duplicate_assertion_selector"}));
        }
        Err(message) if message.contains("no matching document-graph assertion") => {
            return Ok(json!({"outcome":"rejected","diagnostic":"assertion_selector_unmatched"}));
        }
        Err(message) => return Err(message),
    };
    let graph = match expand_document_graph(
        &metadata,
        &owner,
        &ContextResources::new("metadata-fixtures"),
        |predicate| closure.get(&predicate.to_string()).cloned(),
    ) {
        Ok(graph) => graph,
        Err(DocumentGraphError::Model(MetadataError::UnmatchedSourceSelector(_))) => {
            return Ok(json!({"outcome":"rejected","diagnostic":"assertion_selector_unmatched"}));
        }
        Err(DocumentGraphError::Model(MetadataError::DuplicateSourceSelector(_))) => {
            return Ok(json!({"outcome":"rejected","diagnostic":"duplicate_assertion_selector"}));
        }
        Err(error) => return Err(error.to_string()),
    };
    let sources = graph
        .assertions()
        .iter()
        .map(|assertion| {
            assertion
                .sources()
                .iter()
                .map(|source| match source {
                    AssertionSource::Document(_) => json!({"kind":"document"}),
                    AssertionSource::Compiler {
                        producer,
                        reference,
                    } => match reference {
                        Some(reference) => {
                            json!({"kind":"compiler","producer":producer,"ref":reference})
                        }
                        None => json!({"kind":"compiler","producer":producer}),
                    },
                    AssertionSource::Author { reference } => {
                        json!({"kind":"author","ref":reference})
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    Ok(json!({"outcome":"accepted","sources":sources}))
}

fn valid_case_id(case_id: &str) -> bool {
    case_id.len() == 13
        && case_id.starts_with("metadata-")
        && case_id[9..].bytes().all(|byte| byte.is_ascii_digit())
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
    if !valid_case_id(case_id) {
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

fn publish(
    case_id: &str,
    targets: &[Value],
    given: &Value,
    schema_closure: &str,
    fixtures: &[Fixture],
) -> Result<Value, String> {
    if !valid_case_id(case_id) {
        return Err("invalid metadata case id".into());
    }
    if targets != [json!({"profile":"json","layout":"single","irRevision":"4.1.0"})] {
        return Err("publish target is not supported".into());
    }
    let resources = read_fixtures(fixtures)?;
    if !resources.contains_key(schema_closure) {
        return Err("schema closure fixture missing".into());
    }
    let context_file = given["contextFile"]
        .as_str()
        .ok_or("missing context fixture")?;
    let context_bytes = resources
        .get(context_file)
        .ok_or("context fixture missing")?;
    let context_digest = Sha256Digest::from_bytes(context_bytes).to_string();
    let context_hex = context_digest.strip_prefix("sha256:").unwrap();
    if let Some(declared) = given["declaredSha256"].as_str()
        && declared != context_hex
    {
        return Ok(
            json!({"outcome":"rejected","diagnostic":"context_digest_mismatch",
            "usablePublicationChanged":false}),
        );
    }
    if given["contextDigest"] != context_digest {
        return Err("context content address mismatch".into());
    }
    if given["contextStorage"] != "external"
        || given["archiveOwnsPredicate"] != false
        || given["providerTrusted"] != true
    {
        return Err("unsupported publication fixture".into());
    }
    let archive = resources
        .get(
            given["archiveFile"]
                .as_str()
                .ok_or("missing archive fixture")?,
        )
        .ok_or("archive fixture missing")?;
    let provider = resources
        .get(
            given["providerFile"]
                .as_str()
                .ok_or("missing provider fixture")?,
        )
        .ok_or("provider fixture missing")?;
    let revision = Sha256Digest::parse(
        given["externalPredicateRevision"]
            .as_str()
            .ok_or("missing external predicate revision")?,
    )
    .map_err(|error| error.to_string())?;
    if revision != Sha256Digest::from_bytes(provider) {
        return Err("external provider revision mismatch".into());
    }
    let inventory_path = context_file
        .strip_prefix("metadata-fixtures/")
        .ok_or("invalid context fixture path")?;
    let library = AuthoredLibrary::create_with_contexts(
        &serde_json::to_vec(&given["authoring"]).map_err(|error| error.to_string())?,
        archive,
        vec![(inventory_path.to_owned(), context_bytes.clone())],
    )
    .map_err(|error| error.to_string())?;
    let mut bindings = PublicationBindings::new(&library).map_err(|error| error.to_string())?;
    bindings
        .add_v4_provider(provider, &revision)
        .map_err(|error| error.to_string())?;
    let mut context = morphir_core::ir::json::read(
        std::str::from_utf8(context_bytes).map_err(|_| "context is not UTF-8")?,
    )
    .map_err(|error| format!("invalid context JSON: {error:?}"))?;
    let aliases = context["@context"]
        .as_object_mut()
        .ok_or("context aliases missing")?;
    if aliases.len() != 1 {
        return Err("publication case needs one predicate alias".into());
    }
    let uri = aliases
        .values()
        .next()
        .and_then(Value::as_str)
        .ok_or("predicate alias must be a node URI")?;
    let bound = bindings
        .bind_uri(&NodeUri::parse(uri).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    *aliases.values_mut().next().unwrap() = json!(bound.to_string());
    let mut published = serde_json::to_vec_pretty(&context).map_err(|error| error.to_string())?;
    published.push(b'\n');
    AuthoredLibrary::create_with_contexts(
        &serde_json::to_vec(&given["authoring"]).map_err(|error| error.to_string())?,
        archive,
        vec![(inventory_path.to_owned(), published.clone())],
    )
    .map_err(|error| error.to_string())?;
    let published_digest = Sha256Digest::from_bytes(&published).to_string();
    Ok(
        json!({"outcome":"accepted","inventory":[{"path":inventory_path,
        "sha256":published_digest.strip_prefix("sha256:").unwrap()}],
        "predicate":bound.to_string()}),
    )
}

fn source_edit(
    case_id: &str,
    targets: &[Value],
    given: &Value,
    schema_closure: &str,
    fixtures: &[Fixture],
) -> Result<Value, String> {
    if !valid_case_id(case_id) {
        return Err("invalid metadata case id".into());
    }
    if targets != [json!({"profile":"json","layout":"single","irRevision":"4.1.0"})] {
        return Err("sourceEdit target is not supported".into());
    }
    read_closure(schema_closure, fixtures)?;
    if given["provenanceStorage"] != "document"
        || given["sourceDependentRemoval"] != true
        || given["trustedProducerManifest"] != false
        || given["storedAssertionSources"] != false
        || given["carrier"] != "documentGraph"
    {
        return Err("unsupported source edit fixture".into());
    }
    let owner = DocumentId::new(
        given["ownerDocument"]
            .as_str()
            .ok_or("missing owner document")?,
    )
    .map_err(|error| error.to_string())?;
    let wire = &given["fact"];
    if wire["graph"] != "default" {
        return Err("source edit requires the default graph".into());
    }
    let subject = NodeUri::parse(wire["subject"].as_str().ok_or("missing fact subject")?)
        .map_err(|error| error.to_string())?;
    let predicate = NodeUri::parse(wire["predicate"].as_str().ok_or("missing fact predicate")?)
        .map_err(|error| error.to_string())?;
    let object = wire["object"]
        .get("@value")
        .ok_or("missing fact value")?
        .clone();
    let fact = Fact::new(
        subject,
        predicate,
        ObjectTerm::value(object),
        GraphName::Default,
    );
    let key = AssertionKey::new(owner.clone(), Carrier::DocumentGraph, fact)
        .map_err(|error| error.to_string())?;
    let mut graph = GraphIndex::new();
    graph
        .insert(Assertion::new(key))
        .map_err(|error| error.to_string())?;
    let source = match given["removeSource"]["kind"].as_str() {
        Some("compiler") => AssertionSource::Compiler {
            producer: given["removeSource"]["producer"]
                .as_str()
                .ok_or("missing compiler producer")?
                .to_owned(),
            reference: given["removeSource"]["ref"].as_str().map(ToOwned::to_owned),
        },
        Some("author") => AssertionSource::Author {
            reference: given["removeSource"]["ref"]
                .as_str()
                .ok_or("missing author reference")?
                .to_owned(),
        },
        _ => return Err("unsupported removal source".into()),
    };
    let before = graph.facts().to_vec();
    match graph.remove_source_from_owner(&owner, &source) {
        Err(MetadataError::UnknownSourceOwnership) => Ok(json!({"outcome":"rejected",
            "diagnostic":"assertion_source_unknown","factsChanged":graph.facts() != before})),
        Err(error) => Err(error.to_string()),
        Ok(_) => Err("source edit fixture unexpectedly has known ownership".into()),
    }
}

fn read_fixtures(fixtures: &[Fixture]) -> Result<HashMap<String, Vec<u8>>, String> {
    let mut seen = HashSet::new();
    let mut verified = HashMap::new();
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
        verified.insert(fixture.path.clone(), bytes);
    }
    Ok(verified)
}

fn read_closure(path: &str, fixtures: &[Fixture]) -> Result<HashMap<String, NodeUri>, String> {
    let verified = read_fixtures(fixtures)?;
    let bytes = verified.get(path).ok_or("schema closure fixture missing")?;
    let text = std::str::from_utf8(bytes).map_err(|_| "schema closure is not UTF-8")?;
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
