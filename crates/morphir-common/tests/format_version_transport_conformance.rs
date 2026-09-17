//! Conformance tests for JSON and YAML root probing and replay transport.
//!
//! The JSON path is probed before it is decoded, so a `probe_*` call is what the corpus's header
//! and root cases exercise. The YAML path has no streaming probe: `morphir_core::ir::yaml::read`
//! builds the whole value tree and `YamlCodec` takes `formatVersion` from it, so the corpus's
//! root cases are exercised through the codec itself and its header observations through
//! `probe_yaml_header`, which reads them off that tree.

use morphir_common::ir_transport::{
    CodecOptions, EventSink, FormatId, IrCodec, IrVersion, Layout, TransportDiagnostic, YamlCodec,
    probe_json_root, probe_json_slice, probe_yaml_header,
};
use morphir_core::format_version::SupportTable;
use morphir_core::traversal::SemanticEvent;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
struct ConformanceFixture {
    #[serde(rename = "supportTable")]
    support_table: String,
    #[serde(rename = "scalarCases")]
    scalar_cases: Vec<ScalarCase>,
    #[serde(rename = "headerOrderCases")]
    header_order_cases: Vec<HeaderOrderCase>,
    #[serde(rename = "rootDiagnosticCases")]
    root_diagnostic_cases: Vec<RootDiagnosticCase>,
}

#[derive(Debug, Deserialize)]
struct ScalarCase {
    name: String,
    value: Value,
    normalization: Normalization,
    compatibility: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Normalization {
    #[serde(default)]
    normalized: Option<String>,
    #[serde(default)]
    diagnostic: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HeaderOrderCase {
    format: String,
    source: String,
    warning: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RootDiagnosticCase {
    format: String,
    source: String,
    diagnostic: String,
}

struct NullSink;

impl EventSink for NullSink {
    fn accept(&mut self, _event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        Ok(())
    }
}

fn fixture() -> ConformanceFixture {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/format-version-conformance.json"
    ))
    .expect("conformance fixture")
}

fn support_table() -> SupportTable {
    SupportTable::parse(&fixture().support_table).expect("the corpus support table parses")
}

fn decode_yaml(source: &str, version: IrVersion) -> Result<(), TransportDiagnostic> {
    YamlCodec::new().decode(
        &mut source.as_bytes(),
        &CodecOptions::new(version, Layout::SingleFile, FormatId::yaml()),
        &mut NullSink,
    )
}

/// The v3 and v4 distribution bodies the scalar cases are wrapped in.
fn distribution_body(major: u32) -> Value {
    if major == 3 {
        json!(["Library", [["example"]], [], { "modules": [] }])
    } else {
        json!({
            "Library": {
                "packageName": "example",
                "dependencies": {},
                "def": { "modules": {} }
            }
        })
    }
}

/// The codec answers `SupportTable::reference()`, so the corpus's table has to be that table for
/// these cases to mean what they say.
#[test]
fn the_corpus_support_table_is_the_reference_table() {
    assert_eq!(
        support_table().canonical(),
        SupportTable::reference().canonical()
    );
}

/// Every scalar case, written as a YAML document and decoded through `YamlCodec`.
///
/// This is the YAML counterpart of the JSON root probe: the same corpus values reach the same
/// normalization and support check, and answer the same bare format-version codes.
#[test]
fn scalar_cases_decode_through_the_yaml_codec() {
    for case in fixture().scalar_cases {
        let major = case
            .normalization
            .normalized
            .as_deref()
            .and_then(|release| release.split('.').next())
            .and_then(|major| major.parse::<u32>().ok())
            .unwrap_or(4);
        let version = if major == 3 {
            IrVersion::V3
        } else {
            IrVersion::V4
        };
        let document = morphir_core::ir::yaml::write_canonical(&json!({
            "formatVersion": case.value,
            "distribution": distribution_body(major),
        }));

        let outcome = decode_yaml(&document, version);
        let expected =
            case.normalization
                .diagnostic
                .clone()
                .or_else(|| match case.compatibility.as_deref() {
                    Some("supported") | None => None,
                    Some(other) => Some(other.to_owned()),
                });
        match expected {
            None => {
                outcome.unwrap_or_else(|error| {
                    panic!(
                        "{}: expected a decode, got {error:?}\n{document}",
                        case.name
                    )
                });
            }
            Some(expected) => {
                let error =
                    outcome.expect_err(&format!("{}: expected {expected}\n{document}", case.name));
                assert_eq!(error.code(), expected, "{}", case.name);
            }
        }
    }
}

#[test]
fn header_order_cases_match_parent_conformance_corpus() {
    for case in fixture().header_order_cases {
        match case.format.as_str() {
            "json" => {
                let (probe, _input) =
                    probe_json_root(&mut case.source.as_bytes(), &support_table())
                        .expect("header order case");
                match case.warning {
                    Some(expected) => assert!(
                        probe
                            .observations
                            .iter()
                            .any(|observation| observation.code == expected)
                    ),
                    None => assert!(probe.observations.is_empty()),
                }
            }
            // The YAML reader builds the whole value tree, which preserves member order, so the
            // corpus's observation is answerable here exactly as it is on the JSON path. The
            // corpus pins both halves: the observation, and that either order decodes.
            "yaml" => {
                let observations =
                    probe_yaml_header(case.source.as_bytes()).expect("header order case");
                match case.warning {
                    Some(expected) => assert!(
                        observations
                            .iter()
                            .any(|observation| observation.code == expected)
                    ),
                    None => assert!(observations.is_empty()),
                }
                decode_yaml(&case.source, IrVersion::V3).expect("header order case");
            }
            other => panic!("unsupported format {other}"),
        }
    }
}

#[test]
fn root_diagnostic_cases_match_parent_conformance_corpus() {
    for case in fixture().root_diagnostic_cases {
        let (error, expected) = match case.format.as_str() {
            "json" => (
                probe_json_root(&mut case.source.as_bytes(), &support_table())
                    .err()
                    .or_else(|| probe_json_slice(case.source.as_bytes(), &support_table()).err())
                    .expect("root diagnostic case"),
                case.diagnostic.clone(),
            ),
            "yaml" => (
                decode_yaml(&case.source, IrVersion::V3).expect_err("root diagnostic case"),
                // A repeated root member is a profile violation the YAML reader settles before
                // any member means anything, so the duplicate cases answer the kit's
                // `duplicate_member` rather than a format-version code.
                match case.diagnostic.as_str() {
                    "duplicate_format_version" => "morphir::ir::yaml::duplicate_member".to_owned(),
                    other => other.to_owned(),
                },
            ),
            other => panic!("unsupported format {other}"),
        };
        assert_eq!(error.code(), expected);
    }
}
