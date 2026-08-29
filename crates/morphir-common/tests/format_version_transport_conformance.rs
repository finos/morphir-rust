//! Conformance tests for JSON and YAML root probing and replay transport.

use morphir_common::ir_transport::{probe_json_root, probe_json_slice, probe_yaml_slice};
use morphir_core::format_version::SupportTable;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ConformanceFixture {
    #[serde(rename = "supportedVersions")]
    supported_versions: Vec<String>,
    #[serde(rename = "headerOrderCases")]
    header_order_cases: Vec<HeaderOrderCase>,
    #[serde(rename = "rootDiagnosticCases")]
    root_diagnostic_cases: Vec<RootDiagnosticCase>,
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

fn fixture() -> ConformanceFixture {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/format-version-conformance.json"
    ))
    .expect("conformance fixture")
}

fn support_table() -> SupportTable {
    SupportTable::from_releases(
        fixture()
            .supported_versions
            .iter()
            .map(|release| {
                let mut parts = release.split('.');
                morphir_core::format_version::ReleaseTriplet::new(
                    parts.next().unwrap().parse().unwrap(),
                    parts.next().unwrap().parse().unwrap(),
                    parts.next().unwrap().parse().unwrap(),
                )
            })
            .collect::<Vec<_>>(),
    )
}

#[test]
fn header_order_cases_match_parent_conformance_corpus() {
    for case in fixture().header_order_cases {
        let probe = match case.format.as_str() {
            "json" => {
                let (probe, _input) =
                    probe_json_root(&mut case.source.as_bytes(), &support_table())
                        .expect("header order case");
                probe
            }
            "yaml" => probe_yaml_slice(case.source.as_bytes(), &support_table())
                .expect("header order case"),
            other => panic!("unsupported format {other}"),
        };
        if let Some(expected) = case.warning {
            assert!(
                probe
                    .observations
                    .iter()
                    .any(|observation| observation.code == expected)
            );
        } else {
            assert!(probe.observations.is_empty());
        }
    }
}

#[test]
fn root_diagnostic_cases_match_parent_conformance_corpus() {
    for case in fixture().root_diagnostic_cases {
        let error = match case.format.as_str() {
            "json" => probe_json_root(&mut case.source.as_bytes(), &support_table())
                .err()
                .or_else(|| probe_json_slice(case.source.as_bytes(), &support_table()).err())
                .expect("root diagnostic case"),
            "yaml" => probe_yaml_slice(case.source.as_bytes(), &support_table())
                .expect_err("root diagnostic case"),
            other => panic!("unsupported format {other}"),
        };
        assert_eq!(error.code(), case.diagnostic);
    }
}
