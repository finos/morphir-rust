use morphir_common::ir_transport::checked_json::{
    JsonProjectionError, ProjectionByteLimit, write_literal_projection,
};
use morphir_core::ir::decode::{DecodeBudget, DecodeLimits};
#[test]
fn output_is_refused_before_writing_beyond_its_reservation() {
    let mut output = Vec::new();
    let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
    let error = write_literal_projection(
        br#"{"x":"1"}"#,
        &mut budget,
        &mut output,
        ProjectionByteLimit::new(8).unwrap(),
    )
    .unwrap_err();
    assert!(matches!(error, JsonProjectionError::OutputLimit));
    assert!(output.len() <= 8);
}
#[test]
fn reports_actual_effective_headers_without_typed_ir_validation() {
    use morphir_common::ir_transport::checked_json::HeaderEvidence;
    let mut output = Vec::new();
    let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
    let headers = write_literal_projection(
        br#"{"unknown":true,"formatVersion":3,"formatVersion":4}"#,
        &mut budget,
        &mut output,
        ProjectionByteLimit::new(1024).unwrap(),
    )
    .unwrap();
    for header in [headers.source(), headers.projection()] {
        let HeaderEvidence::Reported(version) = header else {
            panic!("actual header not reported")
        };
        assert_eq!(version.release.to_string(), "4.0.0");
    }
    assert_eq!(output, br#"{"unknown":true,"formatVersion":4}"#);
}
#[test]
fn missing_or_invalid_headers_do_not_invent_releases_or_reject_generic_json() {
    use morphir_common::ir_transport::checked_json::HeaderEvidence;
    for source in [
        br#"{"opaque":true}"#.as_slice(),
        br#"{"formatVersion":{},"unknown":true}"#,
        br#"{"formatVersion":4.0}"#,
    ] {
        let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
        let mut output = Vec::new();
        let headers = write_literal_projection(
            source,
            &mut budget,
            &mut output,
            ProjectionByteLimit::new(1024).unwrap(),
        )
        .unwrap();
        assert_eq!(headers.source(), &HeaderEvidence::NotReported);
        assert_eq!(headers.projection(), &HeaderEvidence::NotReported);
    }
}
#[test]
fn exact_output_limit_and_writer_failure_remain_distinct() {
    use std::io::{self, Write};
    let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
    let mut output = Vec::new();
    write_literal_projection(
        b"[0]",
        &mut budget,
        &mut output,
        ProjectionByteLimit::new(3).unwrap(),
    )
    .unwrap();
    assert_eq!(output, b"[0]");
    struct Fails;
    impl Write for Fails {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "controlled writer failure",
            ))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
    assert!(matches!(
        write_literal_projection(
            b"0",
            &mut budget,
            &mut Fails,
            ProjectionByteLimit::new(3).unwrap()
        ),
        Err(JsonProjectionError::Output(_))
    ));
}
#[test]
fn oversized_unreportable_header_remains_generic_json_data() {
    use morphir_common::ir_transport::checked_json::HeaderEvidence;
    let source = format!(
        r#"{{"formatVersion":"{}","opaque":true}}"#,
        "x".repeat(128 * 1024)
    );
    let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
    let mut output = Vec::new();
    let headers = write_literal_projection(
        source.as_bytes(),
        &mut budget,
        &mut output,
        ProjectionByteLimit::model_originals(),
    )
    .unwrap();
    assert_eq!(headers.source(), &HeaderEvidence::NotReported);
    assert_eq!(output, source.as_bytes());
}
#[test]
fn actual64_mib_serialized_output_fits_and_one_less_byte_refuses() {
    let size = 64 * 1024 * 1024;
    let source = format!("\"{}\"", "x".repeat(size - 2));
    let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
    let mut output = Vec::new();
    write_literal_projection(
        source.as_bytes(),
        &mut budget,
        &mut output,
        ProjectionByteLimit::model_originals(),
    )
    .unwrap();
    assert_eq!(output.len(), size);
    assert_eq!(output.as_slice(), source.as_bytes());
    drop(output);
    let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
    let mut output = Vec::new();
    assert!(matches!(
        write_literal_projection(
            source.as_bytes(),
            &mut budget,
            &mut output,
            ProjectionByteLimit::new(size - 1).unwrap()
        ),
        Err(JsonProjectionError::OutputLimit)
    ));
    assert!(output.len() < size);
}
#[test]
fn frozen_policy_header_expectations_bind_actual_projection_results() {
    use morphir_common::ir_transport::checked_json::HeaderEvidence;
    let packet: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/model-originals-json-policy.json")).unwrap();
    let mut asserted = 0;
    for case in packet["cases"].as_array().unwrap() {
        let fields = [
            "expectedSourceHeader",
            "expectedProjectionHeader",
            "expectedSourceRelease",
            "expectedProjectionRelease",
        ];
        if !fields.iter().any(|name| case.get(name).is_some()) {
            continue;
        }
        let id = case["id"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
        let mut output = Vec::new();
        let headers = write_literal_projection(
            source.as_bytes(),
            &mut budget,
            &mut output,
            ProjectionByteLimit::model_originals(),
        )
        .unwrap();
        for (field, actual) in [
            ("expectedSourceHeader", headers.source()),
            ("expectedProjectionHeader", headers.projection()),
        ] {
            let Some(expected) = case.get(field) else {
                continue;
            };
            match expected.as_str().unwrap() {
                "not-reported" => assert_eq!(actual, &HeaderEvidence::NotReported, "{id} {field}"),
                other => panic!("unhandled frozen header expectation {id} {field} {other}"),
            }
            asserted += 1;
        }
        for (field, actual) in [
            ("expectedSourceRelease", headers.source()),
            ("expectedProjectionRelease", headers.projection()),
        ] {
            let Some(expected) = case.get(field) else {
                continue;
            };
            let expected = expected.as_str().unwrap();
            let HeaderEvidence::Reported(version) = actual else {
                panic!("{id} {field}: expected release {expected}, got {actual:?}")
            };
            assert_eq!(version.release.to_string(), expected, "{id} {field}");
            asserted += 1;
        }
    }
    assert_eq!(
        asserted, 4,
        "all four independently authored header/release fields must be exercised"
    );
}
