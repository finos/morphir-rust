//! The Morphir Compatibility Kit's canonical v4 nodes, round-tripped through Ion.
//!
//! `fixtures/ion/mck-canonical-cases.json` holds the canonical JSON fence of each kit case in
//! `spec/ir/mck/{values,types,patterns-and-literals,definitions,distributions}.md`, with the
//! finos/morphir commit it was taken from. Each node is placed in the smallest v4 library that can
//! hold it. The JSON codec reads that library, the Ion codec writes and reads it back, and the two
//! event streams must be equal.
//!
//! A node the Ion writer does not encode yet must be refused, never dropped: those cases are
//! listed in `NOT_WRITTEN_YET` with the bead that tracks them.

use std::collections::VecDeque;
use std::io::Cursor;

use morphir_common::ir_transport::{
    CodecOptions, EventSink, EventSource, FormatId, IonCodec, IrCodec, IrVersion, JsonCodec,
    Layout, TransportDiagnostic, read_document_tree_with_options, write_document_tree_with_options,
};
use morphir_common::vfs::memory_root;
use morphir_core::ir::v4::{FormatVersion, IRFile};
use morphir_core::traversal::{DistributionHeader, SemanticEvent, SemanticEventKind};
use serde_json::{Value, json};

const CASES: &str = include_str!("fixtures/ion/mck-canonical-cases.json");

/// Cases whose node the Ion writer refuses until the named bead lands.
const NOT_WRITTEN_YET: &[(&str, &str)] = &[
    ("values-0022", "morphir-vvgi.6 attributes"),
    ("types-0010", "morphir-vvgi.6 attributes"),
    ("patterns-and-literals-0012", "morphir-vvgi.6 attributes"),
    (
        "patterns-and-literals-0005",
        "morphir-vvgi.7 document literal",
    ),
    (
        "patterns-and-literals-0006",
        "morphir-vvgi.7 document literal",
    ),
    ("definitions-0020", "morphir-vvgi.7 annotations"),
    ("definitions-0021", "morphir-vvgi.7 annotations"),
    ("definitions-0022", "morphir-vvgi.7 annotations"),
];

#[derive(Default)]
struct Collect(Vec<SemanticEvent>);

impl EventSink for Collect {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        self.0.push(event);
        Ok(())
    }
}

struct Replay(VecDeque<SemanticEvent>);

impl EventSource for Replay {
    fn next_event(&mut self) -> Result<Option<SemanticEvent>, TransportDiagnostic> {
        Ok(self.0.pop_front())
    }
}

fn options(format: FormatId) -> CodecOptions {
    CodecOptions::new(IrVersion::V4, Layout::SingleFile, format)
}

fn decode(codec: &dyn IrCodec, text: &str, format: FormatId) -> Result<Vec<SemanticEvent>, String> {
    let mut sink = Collect::default();
    codec
        .decode(
            &mut Cursor::new(text.as_bytes()),
            &options(format),
            &mut sink,
        )
        .map_err(|error| format!("{error:?}"))?;
    Ok(sink.0)
}

fn encode_ion(events: Vec<SemanticEvent>) -> Result<String, String> {
    let mut out = Vec::new();
    IonCodec::new()
        .encode(
            &mut Replay(events.into()),
            &mut out,
            &options(FormatId::ion()),
        )
        .map_err(|error| format!("{error:?}"))?;
    Ok(String::from_utf8(out).expect("Ion text is UTF-8"))
}

/// Ion keeps `formatVersion` as the release string; JSON may keep the same release as `4`.
fn release_string(events: Vec<SemanticEvent>) -> Vec<SemanticEvent> {
    events
        .into_iter()
        .map(|event| {
            let (cursor, kind) = event.into_parts();
            let kind = match kind {
                SemanticEventKind::Begin(header) => SemanticEventKind::Begin(match header {
                    DistributionHeader::V4Library { package, .. } => {
                        DistributionHeader::V4Library {
                            format_version: FormatVersion::String("4.0.0".to_owned()),
                            package,
                        }
                    }
                    DistributionHeader::V4Specs { package, .. } => DistributionHeader::V4Specs {
                        format_version: FormatVersion::String("4.0.0".to_owned()),
                        package,
                    },
                    DistributionHeader::V4Application {
                        package,
                        entry_points,
                        ..
                    } => DistributionHeader::V4Application {
                        format_version: FormatVersion::String("4.0.0".to_owned()),
                        package,
                        entry_points,
                    },
                    other => other,
                }),
                other => other,
            };
            SemanticEvent::new(cursor, kind)
        })
        .collect()
}

/// The smallest v4 library that holds `node` as a case of `kind`.
fn wrap(kind: &str, node: &Value) -> Option<Value> {
    let unit = json!("morphir/SDK:basics#unit");
    let expression = |body: Value| {
        json!({ "Public": { "ExpressionBody": {
            "inputTypes": {}, "outputType": unit, "body": body
        } } })
    };
    let mut types = json!({});
    let mut values = json!({});
    let mut dependency = json!({ "types": {}, "values": {} });
    let mut dependency_module = None;
    match kind {
        "Value" => values["v"] = expression(node.clone()),
        "Literal" => values["v"] = expression(json!({ "Literal": node })),
        "Pattern" => {
            values["v"] =
                expression(json!({ "Lambda": { "pattern": node, "body": { "Unit": {} } } }))
        }
        "Type" => {
            types["t"] = json!({ "Public": { "TypeAliasDefinition": {
                "typeParams": [], "typeExp": node
            } } })
        }
        "TypeDefinition" => types["t"] = json!({ "Public": node }),
        "AccessControlledTypeDefinition" => types["t"] = node.clone(),
        "ValueDefinition" => values["v"] = json!({ "Public": node }),
        "AccessControlledValueDefinition" => values["v"] = node.clone(),
        "TypeSpecification" => dependency["types"]["t"] = node.clone(),
        "ValueSpecification" => dependency["values"]["v"] = node.clone(),
        "ModuleSpecification" => dependency_module = Some(node.clone()),
        "Distribution" => return Some(node.clone()),
        _ => return None,
    }
    let dependency = dependency_module.unwrap_or(dependency);
    Some(json!({
        "formatVersion": "4.0.0",
        "distribution": { "Library": {
            "packageName": "example",
            "dependencies": { "dep/pkg": { "modules": { "m": dependency } } },
            "def": { "modules": { "m": { "Public": { "types": types, "values": values } } } }
        } }
    }))
}

/// What happened to one case, or `None` when it round-tripped.
fn run(id: &str, kind: &str, node: &Value) -> Option<String> {
    let document = wrap(kind, node)?.to_string();
    let original = match decode(&JsonCodec::new(), &document, FormatId::json()) {
        Ok(events) => release_string(events),
        Err(error) => return Some(format!("{id}: the JSON codec refused the wrapper: {error}")),
    };
    let deferred = NOT_WRITTEN_YET.iter().find(|(case, _)| *case == id);
    let ion = match (encode_ion(original.clone()), deferred) {
        (Err(_), Some(_)) => return None,
        (Ok(_), Some((_, bead))) => {
            return Some(format!(
                "{id}: written although {bead} is open; refuse or finish it"
            ));
        }
        (Err(error), None) => return Some(format!("{id}: encode: {error}")),
        (Ok(ion), None) => ion,
    };
    match decode(&IonCodec::new(), &ion, FormatId::ion()) {
        Ok(events) if events == original => None,
        Ok(_) => Some(format!("{id}: the Ion round trip changed the node\n{ion}")),
        Err(error) => Some(format!("{id}: decode: {error}\n{ion}")),
    }
}

#[test]
fn every_canonical_mck_node_round_trips_through_ion() {
    let cases: Value = serde_json::from_str(CASES).unwrap();
    let mut failures = Vec::new();
    let mut ran = 0;
    for case in cases["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let kind = case["node"].as_str().unwrap();
        if wrap(kind, &case["json"]).is_none() {
            continue;
        }
        ran += 1;
        if let Some(failure) = run(id, kind, &case["json"]) {
            failures.push(failure);
        }
    }
    assert!(ran > 70, "only {ran} cases ran");
    assert!(
        failures.is_empty(),
        "{} of {ran} cases failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// What happened to one case written as an Ion document tree, or `None` when it round-tripped.
fn run_tree(id: &str, kind: &str, node: &Value) -> Option<String> {
    let mut expected: IRFile = match serde_json::from_value(wrap(kind, node)?) {
        Ok(file) => file,
        Err(error) => return Some(format!("{id}: the model refused the wrapper: {error}")),
    };
    expected.format_version = FormatVersion::String("4.0.0".to_owned());
    let options = CodecOptions::new(IrVersion::V4, Layout::DocumentTree, FormatId::ion());
    let root = memory_root();
    let deferred = NOT_WRITTEN_YET.iter().find(|(case, _)| *case == id);
    match (
        write_document_tree_with_options(&root, &expected, &options),
        deferred,
    ) {
        (Err(_), Some(_)) => return None,
        (Ok(()), Some((_, bead))) => {
            return Some(format!("{id}: tree written although {bead} is open"));
        }
        (Err(error), None) => return Some(format!("{id}: tree write: {error:?}")),
        (Ok(()), None) => {}
    }
    match read_document_tree_with_options(&root, &options) {
        Ok(mut read) => {
            read.format_version = FormatVersion::String("4.0.0".to_owned());
            (read != expected).then(|| format!("{id}: the Ion tree changed the node"))
        }
        Err(error) => Some(format!("{id}: tree read: {error:?}")),
    }
}

#[test]
fn every_canonical_mck_node_round_trips_through_an_ion_tree() {
    let cases: Value = serde_json::from_str(CASES).unwrap();
    let mut failures = Vec::new();
    let mut ran = 0;
    for case in cases["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let kind = case["node"].as_str().unwrap();
        if wrap(kind, &case["json"]).is_none() {
            continue;
        }
        ran += 1;
        if let Some(failure) = run_tree(id, kind, &case["json"]) {
            failures.push(failure);
        }
    }
    assert!(ran > 70, "only {ran} cases ran");
    assert!(
        failures.is_empty(),
        "{} of {ran} cases failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
