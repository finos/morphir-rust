use std::collections::VecDeque;
use std::io::Cursor;

use morphir_common::ir_transport::{
    CodecOptions, EventSink, EventSource, FormatId, IrCodec, IrVersion, JsonCodec, Layout, Stage,
    TransportDiagnostic, YamlCodec,
};
use morphir_core::traversal::{IrCursor, SemanticEvent};

const V3_JSON: &str = r#"{
  "formatVersion": 3,
  "distribution": ["Library", [["example"]], [], {"modules": []}]
}"#;

const V4_JSON: &str = r#"{
  "formatVersion": 4,
  "distribution": {
    "Library": {
      "packageName": "example",
      "dependencies": {},
      "def": {
        "modules": {
          "domain": {
            "access": "Public",
            "value": {
              "types": {
                "user-id": {
                  "access": "Public",
                  "TypeAliasDefinition": {
                    "typeParams": [],
                    "typeExp": "morphir/SDK:string#string"
                  }
                }
              },
              "values": {}
            }
          }
        }
      }
    }
  }
}"#;

const V3_YAML: &str = include_str!("fixtures/yaml/v3-explicit.yaml");
const V4_EXPLICIT_YAML: &str = include_str!("fixtures/yaml/v4-explicit.yaml");
const V4_READABLE_YAML: &str = include_str!("fixtures/yaml/v4-readable.yaml");
const TIMESTAMP_YAML: &str = include_str!("fixtures/yaml/accepted/timestamp-is-a-string.yaml");

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

fn options(version: IrVersion, format: FormatId) -> CodecOptions {
    CodecOptions::new(version, Layout::SingleFile, format)
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
fn explicit_v3_yaml_matches_the_concrete_v3_json_ir() {
    let yaml = decode(
        &YamlCodec::new(),
        V3_YAML,
        &options(IrVersion::V3, FormatId::yaml()),
    )
    .unwrap();
    let json = decode(
        &JsonCodec::new(),
        V3_JSON,
        &options(IrVersion::V3, FormatId::json()),
    )
    .unwrap();

    assert_eq!(yaml, json);
}

#[test]
fn explicit_and_readable_v4_yaml_normalize_to_the_same_ir() {
    let yaml_options = options(IrVersion::V4, FormatId::yaml());

    let explicit = decode(&YamlCodec::new(), V4_EXPLICIT_YAML, &yaml_options).unwrap();
    let readable = decode(&YamlCodec::new(), V4_READABLE_YAML, &yaml_options).unwrap();
    let json = decode(
        &JsonCodec::new(),
        V4_JSON,
        &options(IrVersion::V4, FormatId::json()),
    )
    .unwrap();

    assert_eq!(explicit, readable);
    assert_eq!(readable, json);
}

#[test]
fn json_to_yaml_to_json_preserves_v3_semantics() {
    let json_options = options(IrVersion::V3, FormatId::json());
    let yaml_options = options(IrVersion::V3, FormatId::yaml());
    let original = decode(&JsonCodec::new(), V3_JSON, &json_options).unwrap();

    let yaml = encode(&YamlCodec::new(), original.clone(), &yaml_options).unwrap();
    assert!(yaml.ends_with('\n'));
    assert!(!yaml.contains("!!"));
    let from_yaml = decode(&YamlCodec::new(), &yaml, &yaml_options)
        .unwrap_or_else(|error| panic!("failed to decode generated YAML: {error:?}\n{yaml}"));

    let json = encode(&JsonCodec::new(), from_yaml.clone(), &json_options).unwrap();
    let from_json = decode(&JsonCodec::new(), &json, &json_options).unwrap();
    assert_eq!(from_yaml, original);
    assert_eq!(from_json, original);
}

fn assert_json_yaml_semantic_round_trip(input: &str, version: IrVersion) {
    let json_options = options(version, FormatId::json());
    let yaml_options = options(version, FormatId::yaml());
    let original = decode(&JsonCodec::new(), input, &json_options).unwrap();

    let yaml = encode(&YamlCodec::new(), original.clone(), &yaml_options).unwrap();
    let from_yaml = decode(&YamlCodec::new(), &yaml, &yaml_options).unwrap_or_else(|error| {
        let line = error.source_span().map_or(1, |span| span.line);
        let excerpt = yaml
            .lines()
            .enumerate()
            .filter(|(index, _)| index + 3 >= line && index < &(line + 2))
            .map(|(index, source)| format!("{:>5} | {source}", index + 1))
            .collect::<Vec<_>>()
            .join("\n");
        panic!("failed to decode generated YAML: {error:?}\n{excerpt}")
    });
    let json = encode(&JsonCodec::new(), from_yaml.clone(), &json_options).unwrap();
    let from_json = decode(&JsonCodec::new(), &json, &json_options).unwrap();

    assert_eq!(from_yaml, original);
    assert_eq!(from_json, original);
}

#[test]
fn real_morphir_elm_v3_fixture_round_trips_through_yaml() {
    assert_json_yaml_semantic_round_trip(
        include_str!("../../morphir-core/tests/fixtures/ir/classic/greeting-example.json"),
        IrVersion::V3,
    );
}

#[test]
fn complete_v4_fixture_round_trips_through_yaml() {
    assert_json_yaml_semantic_round_trip(
        include_str!("../../morphir-core/tests/fixtures/ir/v4/complete-example.json"),
        IrVersion::V4,
    );
}

#[test]
fn v4_library_distribution_round_trips_through_yaml() {
    assert_json_yaml_semantic_round_trip(
        include_str!("../../morphir-core/tests/fixtures/ir/v4/v4-library-distribution.json"),
        IrVersion::V4,
    );
}

#[test]
fn quoted_and_block_scalars_are_not_treated_as_yaml_syntax() {
    let source = r#"
formatVersion: 4
distribution:
  Library:
    packageName: example
    dependencies:
      "2026-08-28":
        modules: {}
    def:
      modules:
        notes:
          access: Public
          value:
            types: {}
            values: {}
            doc: |
              Literal * alias, ! tag, & anchor, and 2026-08-28 text.
        release-notes:
          access: Public
          value:
            types: {}
            values: {}
            doc: "released on 2026-08-28 today"
"#;

    decode(
        &YamlCodec::new(),
        source,
        &options(IrVersion::V4, FormatId::yaml()),
    )
    .unwrap();
}

#[test]
fn v4_json_encoder_rejects_a_dependency_before_begin() {
    let events = decode(
        &JsonCodec::new(),
        include_str!("../../morphir-core/tests/fixtures/ir/v4/complete-example.json"),
        &options(IrVersion::V4, FormatId::json()),
    )
    .unwrap();
    let dependency = events
        .into_iter()
        .find(|event| {
            matches!(
                event.kind(),
                morphir_core::traversal::SemanticEventKind::Dependency(_)
            )
        })
        .expect("fixture should contain a dependency");
    let mut output = Vec::new();
    let mut encoder = JsonCodec::new()
        .encoder(&mut output, &options(IrVersion::V4, FormatId::json()))
        .unwrap();

    let diagnostic = encoder.accept(dependency).unwrap_err();
    drop(encoder);

    assert_eq!(diagnostic.code(), "morphir::ir::json::missing_begin");
    assert!(output.is_empty());
}

#[test]
fn rejected_yaml_has_stable_located_diagnostics() {
    let cases = [
        (
            "fixtures/yaml/rejected/alias-expansion.yaml",
            include_str!("fixtures/yaml/rejected/alias-expansion.yaml"),
            "morphir::ir::yaml::unsupported_yaml_feature",
        ),
        (
            "fixtures/yaml/rejected/custom-tag.yaml",
            include_str!("fixtures/yaml/rejected/custom-tag.yaml"),
            "morphir::ir::yaml::unsupported_yaml_feature",
        ),
        (
            "fixtures/yaml/rejected/cyclic-alias.yaml",
            include_str!("fixtures/yaml/rejected/cyclic-alias.yaml"),
            "morphir::ir::yaml::unsupported_yaml_feature",
        ),
        (
            "fixtures/yaml/rejected/duplicate-key.yaml",
            include_str!("fixtures/yaml/rejected/duplicate-key.yaml"),
            "morphir::ir::yaml::duplicate_member",
        ),
        (
            "fixtures/yaml/rejected/multiple-documents.yaml",
            include_str!("fixtures/yaml/rejected/multiple-documents.yaml"),
            "morphir::ir::yaml::invalid_yaml",
        ),
        (
            "fixtures/yaml/rejected/non-finite-number.yaml",
            include_str!("fixtures/yaml/rejected/non-finite-number.yaml"),
            "morphir::ir::yaml::invalid_literal",
        ),
    ];
    let codec = YamlCodec::new();
    let options = options(IrVersion::V4, FormatId::yaml());

    for (fixture, input, expected_code) in cases {
        let diagnostic = decode(&codec, input, &options).unwrap_err();
        assert_eq!(diagnostic.code(), expected_code, "{fixture}");
        assert!(
            matches!(
                diagnostic.stage(),
                Stage::Syntax | Stage::Normalization | Stage::Detection
            ),
            "{fixture}: {:?}",
            diagnostic.stage()
        );
        assert_eq!(diagnostic.cursor(), &IrCursor::root(), "{fixture}");
        let source_span = diagnostic
            .source_span()
            .unwrap_or_else(|| panic!("{fixture}: missing source span"));
        assert!(source_span.line > 0, "{fixture}");
        assert!(source_span.column > 0, "{fixture}");
        assert!(diagnostic.guidance().is_some(), "{fixture}");
    }
}

/// A date-looking plain scalar is a string, not a timestamp: the profile's scalar resolution has
/// booleans, null, integers, floats and nothing else, so `2026-08-28` is text. This fixture used
/// to be `rejected/timestamp-coercion.yaml`, refused as an ambiguous scalar.
#[test]
fn a_date_looking_scalar_decodes_as_a_string() {
    decode(
        &YamlCodec::new(),
        TIMESTAMP_YAML,
        &options(IrVersion::V4, FormatId::yaml()),
    )
    .expect("the timestamp fixture decodes");

    let value = morphir_core::ir::yaml::read(TIMESTAMP_YAML).expect("the fixture reads");
    assert_eq!(
        value
            .pointer("/distribution/Library/def/modules/notes/value/doc")
            .expect("the module doc"),
        &serde_json::Value::String("2026-08-28".to_owned())
    );
}

/// YAML output is the profile's canonical spelling, byte for byte with the kit's writer.
#[test]
fn yaml_output_is_canonical() {
    let input =
        include_str!("../../morphir-core/tests/fixtures/ir/v4/v4-library-distribution.json");
    let version = IrVersion::V4;
    let events = decode(
        &JsonCodec::new(),
        input,
        &options(version, FormatId::json()),
    )
    .unwrap();
    let yaml = encode(
        &YamlCodec::new(),
        events,
        &options(version, FormatId::yaml()),
    )
    .unwrap();

    let file: morphir_core::ir::v4::IRFile = serde_json::from_str(input).unwrap();
    let expected = morphir_core::ir::yaml::write_canonical(&serde_json::to_value(&file).unwrap());
    assert_eq!(yaml, expected);

    // Flow sequences for scalar-only sequences, and exactly one trailing newline.
    assert!(yaml.contains("typeParams: []"), "{yaml}");
    assert!(yaml.ends_with('\n') && !yaml.ends_with("\n\n"), "{yaml}");
    assert!(!yaml.contains('\r'), "{yaml}");
}
