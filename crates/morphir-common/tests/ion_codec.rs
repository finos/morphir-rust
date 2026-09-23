use std::collections::VecDeque;
use std::io::Cursor;

use morphir_common::ir_transport::{
    CodecOptions, CodecRegistry, EventSink, EventSource, FormatId, IonCodec, IrCodec, IrVersion,
    JsonCodec, Layout, Stage, TransportDiagnostic,
};
use morphir_core::traversal::{IrCursor, SemanticEvent};

const V3_JSON: &str = r#"{
  "formatVersion": 3,
  "distribution": ["Library", [["example"]], [], {"modules": []}]
}"#;

const V3_EMPTY_ION: &str = r#"
morphir::{
  ionVersion: "0.1.0-draft.1",
  formatVersion: "3.0.0",
  kind: library,
  packageName: "example",
}
morphir_footer::{}
"#;

const V3_EMPTY_RECORD: &str = r#"
morphir::{
  ionVersion: "0.1.0-draft.1",
  formatVersion: "3.0.0",
  kind: library,
  packageName: "example",
}
"#;

const V3_MODULE_JSON: &str = r#"{
  "formatVersion": 3,
  "distribution": ["Library", [["example"]], [], {
    "modules": [
      [
        [["eligibility"]],
        {
          "access": "Public",
          "value": {
            "types": [],
            "values": [],
            "doc": "Credit eligibility."
          }
        }
      ]
    ]
  }]
}"#;

const V3_MODULE_ION: &str = r#"
morphir::{
  ionVersion: "0.1.0-draft.1",
  formatVersion: "3.0.0",
  kind: library,
  packageName: "example",
}
public::def::module::{
  name: "eligibility",
  doc: "Credit eligibility.",
}
morphir_footer::{}
"#;

const V3_ALIAS_JSON: &str = r#"{
  "formatVersion": 3,
  "distribution": ["Library", [["example"]], [], {
    "modules": [
      [
        [["eligibility"]],
        {
          "access": "Public",
          "value": {
            "types": [
              [
                ["decision"],
                {
                  "access": "Public",
                  "value": {
                    "doc": "",
                    "value": [
                      "TypeAliasDefinition",
                      [],
                      [
                        "Reference",
                        {},
                        [[["morphir"], ["s", "d", "k"]], [["basics"]], ["bool"]],
                        []
                      ]
                    ]
                  }
                }
              ]
            ],
            "values": [],
            "doc": null
          }
        }
      ]
    ]
  }]
}"#;

const V3_ALIAS_ION: &str = r#"
morphir::{
  ionVersion: "0.1.0-draft.1",
  formatVersion: "3.0.0",
  kind: library,
  packageName: "example",
}
public::def::module::{
  name: "eligibility",
}
public::def::alias::type::{
  module: "eligibility",
  name: "decision",
  typeExp: "morphir/SDK:basics#bool",
}
morphir_footer::{}
"#;

const V3_ALIAS_RECORD: &str = r#"
morphir::{
  ionVersion: "0.1.0-draft.1",
  formatVersion: "3.0.0",
  kind: library,
  packageName: "example",
  modules: [
    public::def::module::{
      name: "eligibility",
      types: [
        public::def::alias::type::{
          name: "decision",
          typeExp: "morphir/SDK:basics#bool",
        },
      ],
    },
  ],
}
"#;

const V3_MODULE_RECORD: &str = r#"
morphir::{
  ionVersion: "0.1.0-draft.1",
  formatVersion: "3.0.0",
  kind: library,
  packageName: "example",
  modules: [
    public::def::module::{
      name: "eligibility",
      doc: "Credit eligibility.",
    },
  ],
}
"#;

#[derive(Default)]
struct CollectingSink(Vec<SemanticEvent>);

impl EventSink for CollectingSink {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        self.0.push(event);
        Ok(())
    }
}

struct QueueSource(VecDeque<SemanticEvent>);

impl EventSource for QueueSource {
    fn next_event(&mut self) -> Result<Option<SemanticEvent>, TransportDiagnostic> {
        Ok(self.0.pop_front())
    }
}

fn decode(
    codec: &dyn IrCodec,
    input: &str,
    options: &CodecOptions,
) -> Result<Vec<SemanticEvent>, TransportDiagnostic> {
    let mut reader = Cursor::new(input.as_bytes());
    let mut sink = CollectingSink::default();
    codec.decode(&mut reader, options, &mut sink)?;
    Ok(sink.0)
}

fn encode(
    codec: &dyn IrCodec,
    events: Vec<SemanticEvent>,
    options: &CodecOptions,
) -> Result<String, TransportDiagnostic> {
    let mut source = QueueSource(events.into());
    let mut output = Vec::new();
    codec.encode(&mut source, &mut output, options)?;
    String::from_utf8(output).map_err(|error| {
        TransportDiagnostic::error(
            "morphir::ir::test::invalid_utf8",
            Stage::Encoding,
            IrCursor::root(),
            error.to_string(),
        )
    })
}

#[test]
fn an_empty_v3_library_datagram_matches_the_json_ir() {
    let ion = decode(
        &IonCodec::new(),
        V3_EMPTY_ION,
        &CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::ion()),
    )
    .unwrap();
    let json = decode(
        &JsonCodec::new(),
        V3_JSON,
        &CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json()),
    )
    .unwrap();

    assert_eq!(ion, json);
}

#[test]
fn a_public_v3_alias_datagram_matches_the_json_ir() {
    let ion = decode(
        &IonCodec::new(),
        V3_ALIAS_ION,
        &CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::ion()),
    )
    .unwrap();
    let json = decode(
        &JsonCodec::new(),
        V3_ALIAS_JSON,
        &CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json()),
    )
    .unwrap();

    assert_eq!(ion, json);
}

#[test]
fn a_public_v3_alias_record_matches_the_json_ir() {
    let ion = decode(
        &IonCodec::new(),
        V3_ALIAS_RECORD,
        &CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::ion()),
    )
    .unwrap();
    let json = decode(
        &JsonCodec::new(),
        V3_ALIAS_JSON,
        &CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json()),
    )
    .unwrap();

    assert_eq!(ion, json);
}

#[test]
fn a_public_v3_alias_round_trips_through_ion() {
    let json_options = CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json());
    let ion_options = CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::ion());
    let original = decode(&JsonCodec::new(), V3_ALIAS_JSON, &json_options).unwrap();

    let ion = encode(&IonCodec::new(), original.clone(), &ion_options).unwrap();
    let from_ion = decode(&IonCodec::new(), &ion, &ion_options)
        .unwrap_or_else(|error| panic!("failed to decode generated Ion: {error:?}\n{ion}"));

    assert_eq!(from_ion, original);
    assert!(ion.contains("public::def::alias::type"), "{ion}");
    assert!(
        ion.contains(r#"typeExp: "morphir/SDK:basics#bool""#),
        "{ion}"
    );
}

#[test]
fn a_public_v3_module_datagram_matches_the_json_ir() {
    let ion = decode(
        &IonCodec::new(),
        V3_MODULE_ION,
        &CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::ion()),
    )
    .unwrap();
    let json = decode(
        &JsonCodec::new(),
        V3_MODULE_JSON,
        &CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json()),
    )
    .unwrap();

    assert_eq!(ion, json);
}

#[test]
fn a_public_v3_module_record_matches_the_json_ir() {
    let ion = decode(
        &IonCodec::new(),
        V3_MODULE_RECORD,
        &CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::ion()),
    )
    .unwrap();
    let json = decode(
        &JsonCodec::new(),
        V3_MODULE_JSON,
        &CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json()),
    )
    .unwrap();

    assert_eq!(ion, json);
}

#[test]
fn a_public_v3_module_round_trips_through_ion() {
    let json_options = CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json());
    let ion_options = CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::ion());
    let original = decode(&JsonCodec::new(), V3_MODULE_JSON, &json_options).unwrap();

    let ion = encode(&IonCodec::new(), original.clone(), &ion_options).unwrap();
    let from_ion = decode(&IonCodec::new(), &ion, &ion_options)
        .unwrap_or_else(|error| panic!("failed to decode generated Ion: {error:?}\n{ion}"));

    assert_eq!(from_ion, original);
    assert!(ion.contains("public::def::module"), "{ion}");
    assert!(ion.contains("morphir_footer::"), "{ion}");
}

#[test]
fn an_empty_v3_library_record_matches_the_json_ir() {
    let ion = decode(
        &IonCodec::new(),
        V3_EMPTY_RECORD,
        &CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::ion()),
    )
    .unwrap();
    let json = decode(
        &JsonCodec::new(),
        V3_JSON,
        &CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json()),
    )
    .unwrap();

    assert_eq!(ion, json);
}

#[test]
fn an_empty_v3_library_round_trips_through_ion() {
    let json_options = CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json());
    let ion_options = CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::ion());
    let original = decode(&JsonCodec::new(), V3_JSON, &json_options).unwrap();

    let ion = encode(&IonCodec::new(), original.clone(), &ion_options).unwrap();
    let from_ion = decode(&IonCodec::new(), &ion, &ion_options)
        .unwrap_or_else(|error| panic!("failed to decode generated Ion: {error:?}\n{ion}"));

    assert_eq!(from_ion, original);
    assert!(ion.contains("morphir_footer::"), "{ion}");
    assert!(ion.contains(r#"ionVersion:"#), "{ion}");
}

#[test]
fn the_builtin_registry_resolves_ion() {
    let registry = CodecRegistry::with_builtins();

    assert_eq!(
        registry.codec(&FormatId::ion()).unwrap().format(),
        &FormatId::ion()
    );
}
