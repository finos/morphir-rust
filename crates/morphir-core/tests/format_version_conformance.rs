//! Conformance tests for scalar format-version recognition.

use morphir_core::format_version::{
    Compatibility, NormalizedFormatVersion, ScalarValue, SupportTable,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct ConformanceFixture {
    #[serde(rename = "supportedVersions")]
    supported_versions: Vec<String>,
    #[serde(rename = "scalarCases")]
    scalar_cases: Vec<ScalarCase>,
}

#[derive(Debug, Deserialize)]
struct ScalarCase {
    value: Value,
    normalization: NormalizationExpectation,
    compatibility: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NormalizationExpectation {
    normalized: Option<String>,
    diagnostic: Option<String>,
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
fn scalar_cases_match_parent_conformance_corpus() {
    let support = support_table();
    for case in fixture().scalar_cases {
        let scalar_result = ScalarValue::from_json(&case.value);
        if let Some(expected) = case.normalization.diagnostic {
            let code = match scalar_result {
                Err(diagnostic) => diagnostic.code().to_string(),
                Ok(scalar) => NormalizedFormatVersion::from_scalar(&scalar, &support)
                    .unwrap_err()
                    .code()
                    .to_string(),
            };
            assert_eq!(code, expected, "{:?}", case.value);
            continue;
        }
        let scalar = scalar_result.expect("scalar type case");
        let normalized =
            NormalizedFormatVersion::from_scalar(&scalar, &support).expect("normalization");
        assert_eq!(
            normalized.release.to_exact_string(),
            case.normalization.normalized.unwrap()
        );
        if let Some(expected) = case.compatibility {
            let compatibility = match expected.as_str() {
                "supported" => Compatibility::Supported,
                "unsupported_format_version_major" => Compatibility::UnsupportedMajor,
                "unsupported_format_version_revision" => Compatibility::UnsupportedRevision,
                other => panic!("unknown compatibility {other}"),
            };
            assert_eq!(normalized.compatibility, compatibility, "{:?}", case.value);
        }
    }
}
