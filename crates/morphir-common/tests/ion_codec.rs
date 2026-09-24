use std::collections::VecDeque;
use std::io::Cursor;

use morphir_common::ir_transport::{
    CodecOptions, CodecRegistry, EventSink, EventSource, FormatId, IonCodec, IrCodec, IrVersion,
    JsonCodec, Layout, Stage, TransportDiagnostic,
};
use morphir_core::ir::v4::FormatVersion;
use morphir_core::traversal::{DistributionHeader, IrCursor, SemanticEvent, SemanticEventKind};

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

fn release_string(events: Vec<SemanticEvent>) -> Vec<SemanticEvent> {
    events
        .into_iter()
        .map(|event| {
            let (cursor, kind) = event.into_parts();
            let kind = match kind {
                SemanticEventKind::Begin(DistributionHeader::V4Library {
                    format_version,
                    package,
                }) => SemanticEventKind::Begin(DistributionHeader::V4Library {
                    format_version: canonical_release(format_version),
                    package,
                }),
                other => other,
            };
            SemanticEvent::new(cursor, kind)
        })
        .collect()
}

fn canonical_release(version: FormatVersion) -> FormatVersion {
    match version {
        FormatVersion::Integer(4) => FormatVersion::String("4.0.0".to_owned()),
        other => other,
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
fn v4_library_fixtures_round_trip_through_ion() {
    for fixture in [
        "../../morphir-core/tests/fixtures/ir/v4/v4-library-distribution.json",
        "../../morphir-core/tests/fixtures/ir/v4/complete-example.json",
    ] {
        let json = match fixture {
            path if path.ends_with("v4-library-distribution.json") => {
                include_str!("../../morphir-core/tests/fixtures/ir/v4/v4-library-distribution.json")
            }
            _ => include_str!("../../morphir-core/tests/fixtures/ir/v4/complete-example.json"),
        };
        let json_options = CodecOptions::new(IrVersion::V4, Layout::SingleFile, FormatId::json());
        let ion_options = CodecOptions::new(IrVersion::V4, Layout::SingleFile, FormatId::ion());
        let original = decode(&JsonCodec::new(), json, &json_options)
            .unwrap_or_else(|error| panic!("{fixture} json: {error:?}"));
        let ion = encode(&IonCodec::new(), original.clone(), &ion_options)
            .unwrap_or_else(|error| panic!("{fixture} encode: {error:?}"));
        let from_ion = decode(&IonCodec::new(), &ion, &ion_options)
            .unwrap_or_else(|error| panic!("{fixture} ion: {error:?}\n{ion}"));
        // Ion stores formatVersion as the release string. JSON may store the
        // same release as the integer 4.
        assert_eq!(
            release_string(from_ion),
            release_string(original),
            "{fixture}\n{ion}"
        );
    }
}

#[test]
fn greeting_example_round_trips_through_ion() {
    let json = include_str!("../../morphir-core/tests/fixtures/ir/classic/greeting-example.json");
    let json_options = CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json());
    let ion_options = CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::ion());
    let original = decode(&JsonCodec::new(), json, &json_options).unwrap();

    let ion = encode(&IonCodec::new(), original.clone(), &ion_options).unwrap();
    let from_ion = decode(&IonCodec::new(), &ion, &ion_options)
        .unwrap_or_else(|error| panic!("failed to decode generated Ion: {error:?}\n{ion}"));

    assert_eq!(from_ion, original);
}

#[test]
fn the_builtin_registry_resolves_ion() {
    let registry = CodecRegistry::with_builtins();

    assert_eq!(
        registry.codec(&FormatId::ion()).unwrap().format(),
        &FormatId::ion()
    );
}

const V4_HEADER: &str = r#"
morphir::{
  ionVersion: "0.1.0-draft.1",
  formatVersion: "4.0.0",
  kind: library,
  packageName: "example",
}
"#;

fn v4_ion() -> CodecOptions {
    CodecOptions::new(IrVersion::V4, Layout::SingleFile, FormatId::ion())
}

fn refusal(input: &str) -> String {
    let error = decode(&IonCodec::new(), input, &v4_ion()).expect_err("the datagram is refused");
    format!("{error:?}")
}

#[test]
fn a_v4_module_defined_twice_is_refused() {
    let text = format!(
        "{V4_HEADER}
public::def::module::{{ name: \"eligibility\" }}
public::def::module::{{ name: \"eligibility\" }}
morphir_footer::{{}}"
    );

    assert!(refusal(&text).contains("duplicate_name"));
}

#[test]
fn a_v4_type_defined_twice_in_a_module_is_refused() {
    let text = format!(
        "{V4_HEADER}
public::def::module::{{
  name: \"eligibility\",
  types: [
    public::def::alias::type::{{ name: \"score\", typeExp: \"morphir/SDK:basics#int\" }},
    public::def::alias::type::{{ name: \"score\", typeExp: \"morphir/SDK:basics#int\" }},
  ],
}}
morphir_footer::{{}}"
    );

    assert!(refusal(&text).contains("duplicate_name"));
}

#[test]
fn a_v4_dependency_stated_twice_merges_its_modules() {
    let split = format!(
        "{V4_HEADER}
package::spec::{{ name: \"morphir/SDK\", modules: [ module::spec::{{ name: \"basics\" }} ] }}
package::spec::{{ name: \"morphir/SDK\", modules: [ module::spec::{{ name: \"list\" }} ] }}
morphir_footer::{{}}"
    );
    let merged = format!(
        "{V4_HEADER}
package::spec::{{
  name: \"morphir/SDK\",
  modules: [ module::spec::{{ name: \"basics\" }}, module::spec::{{ name: \"list\" }} ],
}}
morphir_footer::{{}}"
    );

    assert_eq!(
        decode(&IonCodec::new(), &split, &v4_ion()).unwrap(),
        decode(&IonCodec::new(), &merged, &v4_ion()).unwrap()
    );
}

#[test]
fn a_v4_native_description_round_trips() {
    let text = format!(
        "{V4_HEADER}
public::def::module::{{
  name: \"basics\",
  values: [
    public::def::native::value::{{
      name: \"add\",
      inputTypes: {{ a: \"morphir/SDK:basics#int\", b: \"morphir/SDK:basics#int\" }},
      outputType: \"morphir/SDK:basics#int\",
      hint: arithmetic,
      description: \"Integer addition.\",
    }},
  ],
}}
morphir_footer::{{}}"
    );
    let events = decode(&IonCodec::new(), &text, &v4_ion()).unwrap();

    let ion = encode(&IonCodec::new(), events.clone(), &v4_ion()).unwrap();
    let json = encode(
        &JsonCodec::new(),
        events.clone(),
        &CodecOptions::new(IrVersion::V4, Layout::SingleFile, FormatId::json()),
    )
    .unwrap();

    assert!(json.contains("Integer addition."), "{json}");
    assert_eq!(decode(&IonCodec::new(), &ion, &v4_ion()).unwrap(), events);
}

const V3_WITH_DEPENDENCIES: &str = include_str!("fixtures/ion/v3-with-dependencies.json");

fn v3(format: FormatId) -> CodecOptions {
    CodecOptions::new(IrVersion::V3, Layout::SingleFile, format)
}

#[test]
fn v3_dependency_specifications_round_trip_through_ion() {
    let original = decode(
        &JsonCodec::new(),
        V3_WITH_DEPENDENCIES,
        &v3(FormatId::json()),
    )
    .unwrap();

    let ion = encode(&IonCodec::new(), original.clone(), &v3(FormatId::ion()))
        .unwrap_or_else(|error| panic!("encode: {error:?}"));
    let from_ion = decode(&IonCodec::new(), &ion, &v3(FormatId::ion()))
        .unwrap_or_else(|error| panic!("decode: {error:?}\n{ion}"));

    assert_eq!(from_ion, original, "{ion}");
    assert!(ion.contains("package::spec::"), "{ion}");
    assert!(ion.contains("public::spec::derived::type::"), "{ion}");
}

#[test]
fn a_v3_dependency_named_like_the_distribution_is_refused() {
    let text = r#"
morphir::{ formatVersion: "3.0.0", kind: library, packageName: "example" }
package::spec::{ name: "example" }
morphir_footer::{}
"#;

    let error = decode(&IonCodec::new(), text, &v3(FormatId::ion())).unwrap_err();
    assert!(
        format!("{error:?}").contains("distribution package"),
        "{error:?}"
    );
}

fn v4_module(members: &str) -> String {
    format!(
        "{V4_HEADER}
public::def::module::{{ name: \"m\", values: [ {members} ] }}
morphir_footer::{{}}"
    )
}

fn v4_value(body: &str) -> String {
    v4_module(&format!(
        "public::def::value::{{ name: \"v\", outputType: \"morphir/SDK:basics#int\", body: {body} }}"
    ))
}

#[test]
fn a_non_finite_ion_float_is_refused_not_a_panic() {
    for body in ["nan", "+inf", "-inf", "(lambda nan 1)"] {
        let text = v4_value(body);
        let result = std::panic::catch_unwind(|| decode(&IonCodec::new(), &text, &v4_ion()));
        let result = result.unwrap_or_else(|_| panic!("{body} panicked"));
        assert!(result.is_err(), "{body} was accepted");
    }
}

#[test]
fn a_letrec_that_binds_a_name_twice_is_refused() {
    let binding = "(f { outputType: \"morphir/SDK:basics#int\", body: 1 })";
    let text = v4_value(&format!("(letrec [{binding}, {binding}] f)"));

    let refusal = refusal(&text);
    assert!(refusal.contains("twice"), "{refusal}");
}

#[test]
fn a_record_value_that_names_a_field_twice_is_refused() {
    for body in ["(record (a 1) (a 2))", "(update r (a 1) (a 2))"] {
        assert!(refusal(&v4_value(body)).contains("twice"), "{body}");
    }
}

#[test]
fn a_record_type_that_names_a_field_twice_is_refused() {
    let text = format!(
        "{V4_HEADER}
public::def::module::{{
  name: \"m\",
  types: [ public::def::alias::type::{{
    name: \"t\",
    typeExp: record::{{ fields: [ {{ name: \"a\", type: \"x\" }}, {{ name: \"a\", type: \"y\" }} ] }},
  }} ],
}}
morphir_footer::{{}}"
    );

    assert!(refusal(&text).contains("twice"));
}

#[test]
fn an_entry_point_target_is_a_fully_qualified_name() {
    let text = r#"
morphir::{
  formatVersion: "4.0.0",
  kind: application,
  packageName: "example",
  entryPoints: { start: { target: "not an fqname", kind: main } },
}
morphir_footer::{}
"#;

    assert!(decode(&IonCodec::new(), text, &v4_ion()).is_err());
}

#[test]
fn v3_header_dependencies_in_a_datagram_are_refused_not_dropped() {
    let text = r#"
morphir::{
  formatVersion: "3.0.0",
  kind: library,
  packageName: "example",
  dependencies: [ package::spec::{ name: "morphir/SDK" } ],
}
morphir_footer::{}
"#;

    assert!(decode(&IonCodec::new(), text, &v3(FormatId::ion())).is_err());
}
