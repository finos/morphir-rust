use std::collections::VecDeque;
use std::io::Cursor;

use morphir_common::ir_transport::{
    CodecOptions, DocumentTreeSink, EventSink, EventSource, FormatId, IonCodec, IrCodec, IrVersion,
    JsonCodec, Layout, TransportDiagnostic, YamlCodec,
};
use morphir_common::vfs::memory_root;
use morphir_core::traversal::SemanticEvent;
use serde_json::{Value, json};

#[derive(Default)]
struct Events(Vec<SemanticEvent>);

impl EventSink for Events {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        self.0.push(event);
        Ok(())
    }
}

struct Source(VecDeque<SemanticEvent>);

impl EventSource for Source {
    fn next_event(&mut self) -> Result<Option<SemanticEvent>, TransportDiagnostic> {
        Ok(self.0.pop_front())
    }
}

fn example() -> Value {
    let predicate =
        "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/value/deprecated";
    let subject = "morphir://ir/pkg/regulation?format=4.1.0#/module/api/value/calculate-total";
    let mut document: Value = serde_json::from_str(include_str!(
        "../../morphir-core/tests/fixtures/ir/v4/complete-example.json"
    ))
    .unwrap();
    document["formatVersion"] = json!("4.1.0");
    document["distribution"]["Library"]["def"]["modules"]["u-s/f-r-2052-a/data-tables"]["value"]
        ["types"]["data-tables"]["TypeAliasDefinition"]["typeExp"]["Record"]["attributes"] =
        json!({"@context": {"deprecated": predicate}, "facts": {"deprecated": true}});
    let body = &mut document["distribution"]["Library"]["def"]["modules"]["u-s/f-r-2052-a/data-tables"]
        ["value"]["values"]["calculate-total"]["ExpressionBody"]["body"]["Literal"];
    *body = json!({
        "attributes": {"@context": {"deprecated": predicate}, "facts": {"deprecated": true}},
        "literal": {"FloatLiteral": 0.0}
    });
    document["distribution"]["Library"]["dependencies"]["morphir/SDK"]["modules"]["basics"]["values"]
        ["add"]["annotations"] = json!({
        "@context": {
            "deprecated": predicate,
            "publicApi": "morphir://ir/pkg/acme/annotation-vocab?format=4.0.0#/module/annotations/value/public-api"
        },
        "entries": ["publicApi", {
            "name": "morphir/SDK:basics#add",
            "arguments": [
                {"Literal": {"StringLiteral": "since 4.1"}},
                {"name": "reason", "value": {"Literal": {"StringLiteral": "reviewed"}}}
            ]
        }],
        "facts": {"deprecated": true}
    });
    document["$meta"] = json!({
        "@context": {"deprecated": predicate},
        "@graph": [{"@id": subject, "deprecated": true}],
        "assertionSources": [{
            "selector": {
                "carrier": "documentGraph",
                "subject": subject,
                "predicate": predicate,
                "object": {"@value": true}
            },
            "sources": [{"kind": "author", "ref": "review/42"}]
        }]
    });
    document
}

fn options(format: FormatId) -> CodecOptions {
    CodecOptions::new(IrVersion::V4, Layout::SingleFile, format).with_linked_metadata()
}

fn roundtrip(codec: &dyn IrCodec, text: &str, format: FormatId) -> Value {
    let options = options(format);
    let mut events = Events::default();
    codec
        .decode(&mut Cursor::new(text.as_bytes()), &options, &mut events)
        .unwrap();
    let mut output = Vec::new();
    codec
        .encode(&mut Source(events.0.into()), &mut output, &options)
        .unwrap();
    let output = String::from_utf8(output).unwrap();
    if codec.format() == &FormatId::json() {
        serde_json::from_str(&output).unwrap()
    } else {
        morphir_core::ir::yaml::read(&output).unwrap()
    }
}

#[test]
fn metadata_carriers_survive_json_and_yaml_single_file_transport() {
    let expected = example();
    let json_text = serde_json::to_string(&expected).unwrap();
    let yaml_text = morphir_core::ir::yaml::write_canonical(&expected);
    for actual in [
        roundtrip(&JsonCodec::new(), &json_text, FormatId::json()),
        roundtrip(&YamlCodec::new(), &yaml_text, FormatId::yaml()),
    ] {
        let module = &actual["distribution"]["Library"]["def"]["modules"]["u-s/f-r-2052-a/data-tables"]
            ["Public"];
        assert_eq!(actual["$meta"], expected["$meta"]);
        assert_eq!(
            actual["distribution"]["Library"]["dependencies"]["morphir/SDK"]["modules"]["basics"]["values"]
                ["add"]["annotations"],
            expected["distribution"]["Library"]["dependencies"]["morphir/SDK"]["modules"]["basics"]
                ["values"]["add"]["annotations"]
        );
        assert_eq!(
            module["types"]["data-tables"]["Public"]["TypeAliasDefinition"]["typeExp"]["Record"]["attributes"]
                ["facts"],
            json!({"deprecated": true})
        );
        assert_eq!(
            module["values"]["calculate-total"]["Public"]["ExpressionBody"]["body"]["Literal"]["attributes"]
                ["facts"],
            json!({"deprecated": true})
        );
    }
}

#[test]
fn metadata_carriers_require_4_1_0() {
    let mut file: morphir_core::ir::v4::IRFile = serde_json::from_value(example()).unwrap();
    file.format_version = morphir_core::ir::v4::FormatVersion::String("4.0.0".to_owned());
    assert!(serde_json::to_value(&file).is_err());

    let mut document = example();
    document["formatVersion"] = json!("4.0.0");
    let text = serde_json::to_string(&document).unwrap();
    let mut events = Events::default();
    assert!(
        JsonCodec::new()
            .decode(
                &mut Cursor::new(text.as_bytes()),
                &options(FormatId::json()),
                &mut events
            )
            .is_err()
    );

    let mut document = example();
    document.as_object_mut().unwrap().remove("$meta");
    document["formatVersion"] = json!("4.0.0");
    let text = serde_json::to_string(&document).unwrap();
    assert!(
        JsonCodec::new()
            .decode(
                &mut Cursor::new(text.as_bytes()),
                &options(FormatId::json()),
                &mut Events::default()
            )
            .is_err()
    );
    let yaml = morphir_core::ir::yaml::write_canonical(&document);
    assert!(
        YamlCodec::new()
            .decode(
                &mut Cursor::new(yaml.as_bytes()),
                &options(FormatId::yaml()),
                &mut Events::default()
            )
            .is_err()
    );

    let mut document = example();
    document["formatVersion"] = json!("4.1.1");
    let text = serde_json::to_string(&document).unwrap();
    assert!(
        JsonCodec::new()
            .decode(
                &mut Cursor::new(text.as_bytes()),
                &options(FormatId::json()),
                &mut Events::default()
            )
            .is_err()
    );
}

#[test]
fn node_facts_do_not_move_to_the_top_level() {
    let mut document = example();
    document["distribution"]["Library"]["def"]["modules"]["u-s/f-r-2052-a/data-tables"]["value"]
        ["values"]["calculate-total"]["ExpressionBody"]["body"]["Literal"]["facts"] =
        json!({"deprecated": true});
    let text = serde_json::to_string(&document).unwrap();
    assert!(
        JsonCodec::new()
            .decode(
                &mut Cursor::new(text.as_bytes()),
                &options(FormatId::json()),
                &mut Events::default()
            )
            .is_err()
    );
}

#[test]
fn proposed_revision_requires_opt_in_even_without_metadata() {
    let mut document: Value = serde_json::from_str(include_str!(
        "../../morphir-core/tests/fixtures/ir/v4/complete-example.json"
    ))
    .unwrap();
    let original = serde_json::to_string(&document).unwrap();
    let default_options = CodecOptions::new(IrVersion::V4, Layout::SingleFile, FormatId::json());
    let mut old_events = Events::default();
    JsonCodec::new()
        .decode(
            &mut Cursor::new(original.as_bytes()),
            &default_options,
            &mut old_events,
        )
        .unwrap();
    let mut old_output = Vec::new();
    JsonCodec::new()
        .encode(
            &mut Source(old_events.0.into()),
            &mut old_output,
            &default_options,
        )
        .unwrap();
    let stable: Value = serde_json::from_slice(&old_output).unwrap();
    assert_eq!(stable["formatVersion"], json!(4));
    assert!(stable.get("$meta").is_none());

    document["formatVersion"] = json!("4.1.0");
    let proposed = serde_json::to_string(&document).unwrap();
    let mut events = Events::default();
    let error = JsonCodec::new()
        .decode(
            &mut Cursor::new(proposed.as_bytes()),
            &default_options,
            &mut events,
        )
        .unwrap_err();
    assert_eq!(error.code(), "unsupported_format_version_minor");
    assert_eq!(
        roundtrip(&JsonCodec::new(), &proposed, FormatId::json())["formatVersion"],
        json!("4.1.0")
    );
}

#[test]
fn ion_does_not_claim_the_proposed_revision() {
    let text = serde_json::to_string(&example()).unwrap();
    let mut events = Events::default();
    JsonCodec::new()
        .decode(
            &mut Cursor::new(text.as_bytes()),
            &options(FormatId::json()),
            &mut events,
        )
        .unwrap();
    let mut output = Vec::new();
    let ion = CodecOptions::new(IrVersion::V4, Layout::SingleFile, FormatId::ion());
    assert!(
        IonCodec::new()
            .encode(&mut Source(events.0.into()), &mut output, &ion)
            .is_err()
    );
}

#[test]
fn document_source_selector_must_match_an_authored_graph_assertion() {
    let mut document = example();
    document["$meta"]["assertionSources"][0]["selector"]["predicate"] =
        json!("morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/value/other");
    let text = serde_json::to_string(&document).unwrap();
    assert!(
        JsonCodec::new()
            .decode(
                &mut Cursor::new(text.as_bytes()),
                &options(FormatId::json()),
                &mut Events::default()
            )
            .is_err()
    );
}

#[test]
fn unresolved_node_carrier_source_selector_is_rejected() {
    let mut document = example();
    document["$meta"]["assertionSources"][0]["selector"]["carrier"] = json!("attributesFacts");
    let text = serde_json::to_string(&document).unwrap();
    assert!(
        JsonCodec::new()
            .decode(
                &mut Cursor::new(text.as_bytes()),
                &options(FormatId::json()),
                &mut Events::default(),
            )
            .is_err()
    );
}

#[test]
fn proposed_revision_is_not_emitted_as_a_document_tree() {
    let tree_options = CodecOptions::new(IrVersion::V4, Layout::DocumentTree, FormatId::json());
    assert!(
        DocumentTreeSink::new(memory_root(), tree_options.clone().with_linked_metadata()).is_err()
    );
    let mut document = example();
    document.as_object_mut().unwrap().remove("$meta");
    let mut events = Events::default();
    JsonCodec::new()
        .decode(
            &mut Cursor::new(serde_json::to_vec(&document).unwrap()),
            &options(FormatId::json()),
            &mut events,
        )
        .unwrap();
    let mut sink = DocumentTreeSink::new(memory_root(), tree_options).unwrap();
    assert!(sink.accept(events.0.remove(0)).is_err());
}
