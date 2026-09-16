//! Conformance tests for scalar format-version recognition and support tables.

use morphir_core::format_version::{
    Compatibility, NormalizedFormatVersion, ScalarValue, SupportTable,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct ConformanceFixture {
    #[serde(rename = "supportTable")]
    support_table: String,
    #[serde(rename = "scalarCases")]
    scalar_cases: Vec<ScalarCase>,
    #[serde(rename = "supportTableCases")]
    support_table_cases: SupportTableCases,
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

#[derive(Debug, Deserialize)]
struct SupportTableCases {
    parse: Vec<ParseCase>,
    membership: Vec<MembershipCase>,
    render: Vec<RenderCase>,
}

#[derive(Debug, Deserialize)]
struct ParseCase {
    name: String,
    input: String,
    canonical: Option<String>,
    invalid: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct MembershipCase {
    table: String,
    release: String,
    compatibility: String,
}

#[derive(Debug, Deserialize)]
struct RenderCase {
    table: String,
    cargo: Vec<String>,
    elm: Option<Vec<String>>,
    prose: String,
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

fn compatibility_from_str(text: &str) -> Compatibility {
    match text {
        "supported" => Compatibility::Supported,
        "unsupported_format_version_major" => Compatibility::UnsupportedMajor,
        "unsupported_format_version_minor" => Compatibility::UnsupportedMinor,
        other => panic!("unknown compatibility {other}"),
    }
}

fn release(text: &str) -> morphir_core::format_version::ReleaseTriplet {
    let mut parts = text.split('.');
    morphir_core::format_version::ReleaseTriplet::new(
        parts.next().unwrap().parse().unwrap(),
        parts.next().unwrap().parse().unwrap(),
        parts.next().unwrap().parse().unwrap(),
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
            assert_eq!(
                normalized.compatibility,
                compatibility_from_str(&expected),
                "{:?}",
                case.value
            );
        }
    }
}

#[test]
fn support_table_parse_cases() {
    let cases = fixture().support_table_cases.parse;
    assert!(!cases.is_empty(), "the corpus carries parse cases");
    for case in cases {
        let parsed = SupportTable::parse(&case.input);
        if case.invalid.unwrap_or(false) {
            assert!(
                parsed.is_err(),
                "{}: {:?} should not parse, got {:?}",
                case.name,
                case.input,
                parsed.map(|table| table.canonical())
            );
            continue;
        }
        let table = parsed.unwrap_or_else(|error| panic!("{}: {error}", case.name));
        assert_eq!(
            table.canonical(),
            case.canonical
                .expect("a valid case carries a canonical spelling"),
            "{}",
            case.name
        );
    }
}

#[test]
fn support_table_membership_cases() {
    let cases = fixture().support_table_cases.membership;
    assert!(!cases.is_empty(), "the corpus carries membership cases");
    for case in cases {
        let table = SupportTable::parse(&case.table).expect("a membership case table parses");
        assert_eq!(
            table.check(&release(&case.release)),
            compatibility_from_str(&case.compatibility),
            "{} against {}",
            case.release,
            case.table
        );
    }
}

#[test]
fn support_table_render_cases() {
    let cases = fixture().support_table_cases.render;
    assert!(!cases.is_empty(), "the corpus carries render cases");
    for case in cases {
        let table = SupportTable::parse(&case.table).expect("a render case table parses");
        assert_eq!(table.canonical(), case.table, "{} round-trips", case.table);
        assert_eq!(table.render_cargo(), case.cargo, "cargo for {}", case.table);
        match case.elm {
            Some(expected) => assert_eq!(
                table.render_elm().expect("an Elm-renderable table"),
                expected,
                "elm for {}",
                case.table
            ),
            None => assert!(
                table.render_elm().is_err(),
                "{} has no Elm rendering",
                case.table
            ),
        }
        assert_eq!(table.render_prose(), case.prose, "prose for {}", case.table);
    }
}
