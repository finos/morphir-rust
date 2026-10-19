use morphir_core::ir::decode::{DecodeBudget, DecodeLimits, read_json};
#[test]
fn new_profile_retains_private_number_keys_and_last_wins_members() {
    let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
    let value = read_json(
        br#"{"formatVersion":4,"extra":{"$serde_json::private::Number":"1"},"a":1,"a":2}"#,
        &mut budget,
    )
    .unwrap();
    assert!(value["extra"].is_object());
    assert_eq!(
        value["extra"]["$serde_json::private::Number"].as_str(),
        Some("1")
    );
    assert_eq!(value["a"].as_u64(), Some(2));
}
#[test]
fn source_value_budget_refuses_before_parsing_the_forbidden_child() {
    let source = format!("[{}?]", "0,".repeat(99_999));
    let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
    let error = read_json(source.as_bytes(), &mut budget).unwrap_err();
    assert!(format!("{error:?}").contains("Values"), "{error:?}");
}
#[test]
fn separately_frozen_policy_cases_match_the_public_source_decoder() {
    use morphir_core::ir::decode::{DecodeLimit, JsonDecodeError};
    let packet: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/model-originals-json-policy.json")).unwrap();
    for case in packet["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let source = if let Some(source) = case["source"].as_str() {
            source.to_owned()
        } else {
            let recipe = &case["sourceRecipe"];
            match recipe["kind"].as_str().unwrap() {
                "nested-array-around-zero" => {
                    let n = recipe["arrays"].as_u64().unwrap() as usize;
                    format!("{}0{}", "[".repeat(n), "]".repeat(n))
                }
                "array-of-zero" => {
                    let n = recipe["elements"].as_u64().unwrap() as usize;
                    format!("[{}0]", "0,".repeat(n - 1))
                }
                other => panic!("unknown independently authored recipe {other}"),
            }
        };
        let string_units = case["limits"]["stringUnits"]
            .as_u64()
            .unwrap_or(64 * 1024 * 1024) as usize;
        let mut budget = DecodeBudget::new(
            DecodeLimits::new(64 * 1024 * 1024, 128, 100_000, string_units).unwrap(),
        );
        let result = read_json(source.as_bytes(), &mut budget);
        let status = case["expectedStatus"].as_str().unwrap_or("accepted");
        if status == "accepted" {
            let value = result.unwrap_or_else(|e| panic!("{id}: {e}"));
            if let Some(expected) = case["expectedJson"].as_str() {
                assert_eq!(value.to_string(), expected, "{id}");
            }
            if let Some(expected) = case["expectedNumericLexemes"].as_array() {
                let actual: Vec<_> = value
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                let expected: Vec<_> = expected.iter().map(|v| v.as_str().unwrap()).collect();
                assert_eq!(actual, expected, "{id}");
            }
        } else {
            let error = result.unwrap_err();
            match (status, error) {
                ("syntax", JsonDecodeError::Syntax(_)) => (),
                ("depth-limit", JsonDecodeError::Limit(e)) => {
                    assert_eq!(e.limit(), DecodeLimit::Depth, "{id}")
                }
                ("value-limit", JsonDecodeError::Limit(e)) => {
                    assert_eq!(e.limit(), DecodeLimit::Values, "{id}")
                }
                ("string-unit-limit", JsonDecodeError::Limit(e)) => {
                    assert_eq!(e.limit(), DecodeLimit::StringUnits, "{id}")
                }
                (status, e) => panic!("{id}: expected {status}, got {e}"),
            }
        }
    }
}
#[test]
fn source_and_assembled_ledgers_are_aggregate_and_independent() {
    use morphir_core::ir::decode::{DecodeLimit, DecodePhase, check_assembled};
    let mut budget = DecodeBudget::new(DecodeLimits::new(256, 128, 3, 64).unwrap());
    let array = read_json(b"[0]", &mut budget).unwrap();
    check_assembled(&array, &mut budget).unwrap();
    let scalar = read_json(b"0", &mut budget).unwrap();
    check_assembled(&scalar, &mut budget).unwrap();
    assert_eq!(budget.usage(DecodePhase::Source).values(), 3);
    assert_eq!(budget.usage(DecodePhase::Assembled).values(), 3);
    let e = check_assembled(&scalar, &mut budget).unwrap_err();
    assert_eq!(e.phase(), DecodePhase::Assembled);
    assert_eq!(e.limit(), DecodeLimit::Values);
    assert_eq!(budget.usage(DecodePhase::Source).values(), 3);
}
#[test]
fn exact64_mib_source_remains_possible_and_one_over_refuses_before_syntax() {
    use morphir_core::ir::decode::{DecodeLimit, JsonDecodeError};
    let mut source = vec![b' '; 64 * 1024 * 1024 - 3];
    source.extend_from_slice(b"[0]");
    let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
    assert_eq!(
        read_json(&source, &mut budget).unwrap(),
        serde_json::json!([0])
    );
    source.push(255);
    let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
    let JsonDecodeError::Limit(error) = read_json(&source, &mut budget).unwrap_err() else {
        panic!("byte limit must precede UTF8/syntax")
    };
    assert_eq!(error.limit(), DecodeLimit::SourceBytes);
    assert_eq!(budget.source_bytes(), 0);
}
#[test]
fn invalid_limits_cannot_construct_an_unbounded_policy() {
    for limits in [
        (0, 128, 1, 1),
        (usize::MAX, 128, 1, 1),
        (1, 129, 1, 1),
        (1, 0, 0, 0),
        (1, 0, 100_001, 0),
        (1, 0, 1, 64 * 1024 * 1024 + 1),
    ] {
        assert!(DecodeLimits::new(limits.0, limits.1, limits.2, limits.3).is_err());
    }
    assert!(DecodeLimits::new(1, 0, 1, 0).is_ok());
}
