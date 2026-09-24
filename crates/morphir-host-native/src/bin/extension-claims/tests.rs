use super::validate_claims;
use morphir_extension_sdk::claims::CapabilityClaimSet;
use morphir_extension_sdk::{ExtensionCapabilities, ExtensionInfo};
use serde_json::{Value, json};

#[test]
fn describe_request_is_accepted_by_the_sdk_dispatcher() {
    use morphir_avro_extension::AvroExtension;
    use morphir_extension_sdk::Extension;
    use morphir_extension_sdk::protocol::{ExtensionRequest, SUPPORTED_MEP_VERSIONS, methods};

    let request = ExtensionRequest::new(methods::DESCRIBE, super::describe_params(), 1).unwrap();
    let response = morphir_extension_sdk::__dispatch_request::<AvroExtension>(
        &request,
        &[],
        &AvroExtension::info().types,
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    let expected = CapabilityClaimSet::from_metadata(
        SUPPORTED_MEP_VERSIONS
            .iter()
            .map(|version| (*version).into())
            .collect(),
        AvroExtension::info(),
        &AvroExtension::capabilities(),
    )
    .unwrap();
    assert_eq!(
        response.result.unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

fn a_claim_set() -> Value {
    serde_json::to_value(
        CapabilityClaimSet::from_metadata(
            vec!["0.1".into()],
            ExtensionInfo::default(),
            &ExtensionCapabilities::default(),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn preserves_guest_members_without_normalizing_them() {
    let mut claims = a_claim_set();
    claims["requires"] = json!({"host": [">=0.4.0"], "future": {"enabled": true}});
    claims["critical"] = json!(["requires.host"]);
    claims["capabilities"]["future"] = json!({"enabled": true});
    claims["future"] = json!([1, 2, 3]);
    assert_eq!(validate_claims(claims.clone()).unwrap(), claims);
}

#[test]
fn rejects_an_invalid_describe_result() {
    for result in [Value::Null, json!({}), json!({"capabilities": {}})] {
        assert!(validate_claims(result).is_err());
    }
}

#[test]
fn rejects_unsupported_claims_versions() {
    let mut claims = a_claim_set();
    claims["claimsVersion"] = json!("99.0.0");
    assert!(validate_claims(claims).is_err());
}
