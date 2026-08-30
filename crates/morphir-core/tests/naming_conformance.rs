//! Runs the shared name-encoding conformance corpus from finos/morphir.
//!
//! The fixture is vendored byte-identically at `tests/fixtures/`, the same way
//! `format-version-conformance.json` is. It covers both canonical encodings, so
//! it does not change when `CANONICAL_STYLE` is flipped.

use morphir_core::naming::{Name, NameStyle, Segment};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Corpus {
    round_trip_cases: Vec<RoundTripCase>,
    legacy_decode_cases: Vec<LegacyCase>,
    reject_cases: Vec<RejectCase>,
    path_cases: Vec<PathCase>,
    fq_name_cases: Vec<FqNameCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoundTripCase {
    name: String,
    segments: Vec<SegmentSpec>,
    canonical: Canonical,
    escaped_stem: String,
    rendered: Rendered,
    legacy_array: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct SegmentSpec {
    kind: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct Canonical {
    uppercase: String,
    #[serde(rename = "doubledHyphen")]
    doubled_hyphen: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rendered {
    camel_case: String,
    pascal_case_upper_initialism: String,
    pascal_case_pascal_initialism: String,
    snake_case: String,
    kebab_case: String,
    screaming_snake_case: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyCase {
    legacy_array: Vec<String>,
    segments: Vec<SegmentSpec>,
    canonical: Canonical,
}

#[derive(Debug, Deserialize)]
struct RejectCase {
    input: String,
    #[serde(rename = "validAs")]
    valid_as: ValidAs,
}

#[derive(Debug, Deserialize)]
struct ValidAs {
    uppercase: bool,
    #[serde(rename = "doubledHyphen")]
    doubled_hyphen: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PathCase {
    canonical: Canonical,
    escaped_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FqNameCase {
    canonical: Canonical,
    document_tree_path: String,
}

fn corpus() -> Corpus {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/naming-conformance.json"
    ))
    .expect("naming conformance fixture")
}

fn to_segments(specs: &[SegmentSpec]) -> Vec<Segment> {
    specs
        .iter()
        .map(|spec| match spec.kind.as_str() {
            "word" => Segment::Word(spec.text.clone()),
            "initialism" => Segment::Initialism(spec.text.clone()),
            other => panic!("unknown segment kind {other:?}"),
        })
        .collect()
}

#[test]
fn round_trip_cases_match_the_corpus() {
    for case in corpus().round_trip_cases {
        let label = &case.name;
        let name =
            Name::from_segments(to_segments(&case.segments)).expect("corpus segments are valid");

        assert_eq!(
            name.to_canonical_string_in(NameStyle::Uppercase),
            case.canonical.uppercase,
            "uppercase encoding for {label}"
        );
        assert_eq!(
            name.to_canonical_string_in(NameStyle::DoubledHyphen),
            case.canonical.doubled_hyphen,
            "doubled-hyphen encoding for {label}"
        );

        // Both encodings decode back to the same name.
        assert_eq!(
            Name::from_canonical_string(&case.canonical.uppercase).unwrap(),
            name,
            "uppercase decode for {label}"
        );
        assert_eq!(
            Name::from_canonical_string(&case.canonical.doubled_hyphen).unwrap(),
            name,
            "doubled-hyphen decode for {label}"
        );

        assert_eq!(
            name.to_file_stem(),
            case.escaped_stem,
            "file stem for {label}"
        );
        assert_eq!(
            Name::from_file_stem(&case.escaped_stem).unwrap(),
            name,
            "file stem round trip for {label}"
        );

        assert_eq!(
            name.to_camel_case(),
            case.rendered.camel_case,
            "camelCase for {label}"
        );
        assert_eq!(
            name.to_pascal_case(),
            case.rendered.pascal_case_upper_initialism,
            "PascalCase (upper initialism) for {label}"
        );
        assert_eq!(
            name.to_pascal_case_pascal_initialism(),
            case.rendered.pascal_case_pascal_initialism,
            "PascalCase (pascal initialism) for {label}"
        );
        assert_eq!(
            name.to_snake_case(),
            case.rendered.snake_case,
            "snake_case for {label}"
        );
        assert_eq!(
            name.to_kebab_case(),
            case.rendered.kebab_case,
            "kebab-case for {label}"
        );
        assert_eq!(
            name.to_screaming_snake_case(),
            case.rendered.screaming_snake_case,
            "SCREAMING_SNAKE for {label}"
        );

        if let Some(legacy) = case.legacy_array {
            assert_eq!(
                Name::from_words(legacy.clone()),
                name,
                "legacy decode for {label}"
            );
            assert_eq!(name.words(), legacy, "legacy encode for {label}");
        }
    }
}

#[test]
fn legacy_decode_cases_match_the_corpus() {
    for case in corpus().legacy_decode_cases {
        let label = case.legacy_array.join(",");
        let name = Name::from_words(case.legacy_array.clone());

        assert_eq!(
            name.segments(),
            to_segments(&case.segments).as_slice(),
            "segments for [{label}]"
        );
        assert_eq!(
            name.to_canonical_string_in(NameStyle::Uppercase),
            case.canonical.uppercase,
            "uppercase encoding for [{label}]"
        );
        assert_eq!(
            name.to_canonical_string_in(NameStyle::DoubledHyphen),
            case.canonical.doubled_hyphen,
            "doubled-hyphen encoding for [{label}]"
        );
    }
}

#[test]
fn reject_cases_match_the_corpus() {
    for case in corpus().reject_cases {
        let input = &case.input;
        // The union decoder accepts an input legal under either style.
        let accepted = Name::from_canonical_string(input).is_ok();
        let expected = case.valid_as.uppercase || case.valid_as.doubled_hyphen;

        // The empty string is a legal empty Name rather than a parse failure.
        if input.is_empty() {
            assert!(Name::from_canonical_string(input).unwrap().is_empty());
            continue;
        }

        assert_eq!(
            accepted,
            expected,
            "input {input:?} should be {} by the union decoder",
            if expected { "accepted" } else { "rejected" }
        );
    }
}

#[test]
fn path_and_fqname_cases_match_the_corpus() {
    use morphir_core::naming::{FQName, Path};

    for case in corpus().path_cases {
        let path = Path::from_canonical_string(&case.canonical.uppercase).unwrap();
        assert_eq!(path.to_canonical_string(), case.canonical.uppercase);
        assert_eq!(
            Path::from_canonical_string(&case.canonical.doubled_hyphen).unwrap(),
            path,
            "doubled-hyphen path decode for {}",
            case.canonical.doubled_hyphen
        );

        let escaped = path
            .segments
            .iter()
            .map(Name::to_file_stem)
            .collect::<Vec<_>>()
            .join("/");
        assert_eq!(escaped, case.escaped_path);
    }

    for case in corpus().fq_name_cases {
        let fq = FQName::from_canonical_string(&case.canonical.uppercase).unwrap();
        assert_eq!(fq.to_canonical_string(), case.canonical.uppercase);
        assert_eq!(
            FQName::from_canonical_string(&case.canonical.doubled_hyphen).unwrap(),
            fq,
            "doubled-hyphen fqname decode"
        );
        assert!(
            case.document_tree_path.starts_with("pkg/"),
            "document tree path shape"
        );
    }
}
