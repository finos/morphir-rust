use super::validate_claims;
use morphir_extension_sdk::claims::CapabilityClaimSet;
use morphir_extension_sdk::{
    BackendCapability, Extension, ExtensionCapabilities, ExtensionInfo, ExtensionType,
};
use serde_json::{Value, json};

/// A backend whose native metadata the fixture guests report as their own.
#[derive(Default)]
struct BackendFixture;

impl Extension for BackendFixture {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "morphir-claims-fixture".into(),
            name: "Morphir Claims Fixture".into(),
            version: "0.1.0".into(),
            types: vec![ExtensionType::Backend],
            ..Default::default()
        }
    }

    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities {
            backend: Some(BackendCapability {
                targets: vec!["fixture".into()],
                ir_versions: vec!["3".into(), "4".into()],
                generate: true,
            }),
            ..Default::default()
        }
    }
}

#[test]
fn describe_request_is_accepted_by_the_sdk_dispatcher() {
    use morphir_extension_sdk::protocol::{ExtensionRequest, SUPPORTED_MEP_VERSIONS, methods};

    let request = ExtensionRequest::new(methods::DESCRIBE, super::describe_params(), 1).unwrap();
    let response = morphir_extension_sdk::__dispatch_request::<BackendFixture>(
        &request,
        &[],
        &BackendFixture::info().types,
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    let expected = CapabilityClaimSet::from_metadata(
        SUPPORTED_MEP_VERSIONS
            .iter()
            .map(|version| (*version).into())
            .collect(),
        BackendFixture::info(),
        &BackendFixture::capabilities(),
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
