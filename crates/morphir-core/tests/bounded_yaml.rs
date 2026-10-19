use morphir_core::ir::decode::{
    DecodeBudget, DecodeLimit, DecodeLimits, DecodePhase, YamlDecodeError, read_yaml,
};

#[test]
fn frozen_yaml_policy_cases_preserve_profile_semantics_and_enforce_limits() {
    let packet: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/model-originals-yaml-policy.json")).unwrap();
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
                other => panic!("unhandled independent recipe {other}"),
            }
        };
        let limits = &case["limits"];
        let mut budget = DecodeBudget::new(
            DecodeLimits::new(
                64 * 1024 * 1024,
                limits["depth"].as_u64().unwrap_or(128) as usize,
                limits["values"].as_u64().unwrap_or(100_000) as usize,
                limits["stringUnits"].as_u64().unwrap_or(64 * 1024 * 1024) as usize,
            )
            .unwrap(),
        );
        let result = read_yaml(source.as_bytes(), &mut budget);
        if let Some(expected) = case["expectedLimit"].as_str() {
            let Err(YamlDecodeError::Limit(error)) = result else {
                panic!("{id}: {result:?}")
            };
            assert_eq!(format!("{:?}", error.limit()), expected, "{id}");
        } else if let Some(expected) = case["expectedDiagnostic"].as_str() {
            let Err(YamlDecodeError::Profile(error)) = result else {
                panic!("{id}: {result:?}")
            };
            assert_eq!(format!("{:?}", error.code), expected, "{id}");
        } else {
            let value = result.unwrap_or_else(|error| panic!("{id}: {error}"));
            if let Some(expected) = case["expectedJson"].as_str() {
                assert_eq!(value.to_string(), expected, "{id}");
            }
            for (field, actual) in [
                ("expectedValues", budget.usage(DecodePhase::Source).values()),
                (
                    "expectedStringUnits",
                    budget.usage(DecodePhase::Source).string_units(),
                ),
            ] {
                if let Some(expected) = case[field].as_u64() {
                    assert_eq!(actual, expected as usize, "{id} {field}");
                }
            }
        }
    }
}

#[test]
fn value_credit_precedes_plain_scalar_resolution() {
    let mut budget = DecodeBudget::new(DecodeLimits::new(128, 128, 2, 128).unwrap());
    let error = read_yaml(b"[0, 01]", &mut budget).unwrap_err();
    let YamlDecodeError::Limit(error) = error else {
        panic!("{error:?}")
    };
    assert_eq!(error.limit(), DecodeLimit::Values);
    assert_eq!(budget.usage(DecodePhase::Source).values(), 2);
}

#[test]
fn parser_depth_refusal_is_typed_and_legacy_depth_policy_remains_unchanged() {
    let source = format!("{}0{}", "[".repeat(129), "]".repeat(129));
    assert!(morphir_core::ir::yaml::read(&source).is_ok());
    let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
    let YamlDecodeError::Limit(error) = read_yaml(source.as_bytes(), &mut budget).unwrap_err()
    else {
        panic!("bounded depth must refuse");
    };
    assert_eq!(error.limit(), DecodeLimit::Depth);
    let mut budget = DecodeBudget::new(DecodeLimits::new(128, 0, 128, 128).unwrap());
    let YamlDecodeError::Limit(error) = read_yaml(b"[[]]", &mut budget).unwrap_err() else {
        panic!("parser depth must use typed resource refusal");
    };
    assert_eq!(error.limit(), DecodeLimit::Depth);
    for source in [b"[]".as_slice(), b"{}", b"'x'"] {
        let mut budget = DecodeBudget::new(DecodeLimits::new(128, 0, 1, 128).unwrap());
        assert!(read_yaml(source, &mut budget).is_ok());
    }
}

#[test]
fn mixed_json_yaml_sources_share_the_same_aggregate_ledger() {
    use morphir_core::ir::decode::read_json;
    let mut budget = DecodeBudget::new(DecodeLimits::new(128, 128, 3, 128).unwrap());
    read_json(b"[0]", &mut budget).unwrap();
    read_yaml(b"0", &mut budget).unwrap();
    let YamlDecodeError::Limit(error) = read_yaml(b"? invalid", &mut budget).unwrap_err() else {
        panic!("aggregate limit must precede plain-value resolution");
    };
    assert_eq!(error.limit(), DecodeLimit::Values);
    assert_eq!(budget.usage(DecodePhase::Assembled).values(), 0);
}

#[test]
fn exact_64mib_source_and_one_over_keep_byte_policy_before_utf8() {
    let size = 64 * 1024 * 1024;
    let mut source = format!("#{}\n0\n", "x".repeat(size - 4)).into_bytes();
    assert_eq!(source.len(), size);
    let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
    assert_eq!(
        read_yaml(&source, &mut budget).unwrap(),
        serde_json::json!(0)
    );
    source.push(255);
    let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
    let YamlDecodeError::Limit(error) = read_yaml(&source, &mut budget).unwrap_err() else {
        panic!("byte policy must precede UTF8 parsing");
    };
    assert_eq!(error.limit(), DecodeLimit::SourceBytes);
    assert_eq!(budget.source_bytes(), 0);
}
