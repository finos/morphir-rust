use morphir_extension_sdk::{
    Extension, ExtensionCapabilities, ExtensionInfo, ExtensionType, NativeExtension,
    claims::CapabilityClaimSet,
    native::doc_fixtures::DocFrontend,
    protocol::{ExtensionRequest, ExtensionResponse, methods},
};
use serde_json::json;

struct MetadataOnly;
impl Default for MetadataOnly {
    fn default() -> Self {
        panic!("describe must not construct the guest")
    }
}
impl Extension for MetadataOnly {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "metadata".into(),
            ..ExtensionInfo::default()
        }
    }
    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities::default()
    }
}

fn describe_request() -> ExtensionRequest {
    ExtensionRequest::new(
        "morphir.extension.describe",
        json!({"protocolVersions":["0.1"]}),
        1,
    )
    .unwrap()
}

fn assert_description_matches_session(handle: impl Fn(ExtensionRequest) -> ExtensionResponse) {
    let described = handle(describe_request());
    assert!(described.error.is_none(), "{:?}", described.error);
    let wire = described.result.unwrap();
    assert_eq!(wire["claimsVersion"], "0.1.0-draft.2");
    assert!(wire.get("statementVersion").is_none());
    let claims: CapabilityClaimSet = serde_json::from_value(wire).unwrap();
    let initialized = handle(
        ExtensionRequest::new(
            methods::INITIALIZE,
            json!({
                "protocolVersions": ["0.1"], "host": {"name": "test", "version": "1.0.0"}
            }),
            2,
        )
        .unwrap(),
    )
    .result
    .unwrap();
    let capabilities = handle(ExtensionRequest::new(methods::CAPABILITIES, json!({}), 3).unwrap())
        .result
        .unwrap();
    assert_eq!(json!(claims.extension), initialized["extension"]);
    assert_eq!(json!(claims.capabilities), initialized["capabilities"]);
    assert_eq!(json!(claims.capabilities), capabilities);
    assert_eq!(handle(describe_request()).result.unwrap(), json!(claims));
}

#[test]
fn describe_precedes_initialize_without_constructing_the_guest() {
    assert_description_matches_session(|request| {
        morphir_extension_sdk::__dispatch_request::<MetadataOnly>(&request, &[], &[])
    });
}

#[test]
fn existing_instance_dispatch_uses_the_same_metadata() {
    assert_description_matches_session(|request| {
        morphir_extension_sdk::__dispatch_request_with(&MetadataOnly, &request, &[], &[])
    });
}

#[test]
fn native_role_dispatch_describes_registered_capabilities() {
    let extension = NativeExtension::builder(DocFrontend)
        .with_frontend()
        .finish()
        .unwrap();
    assert_eq!(extension.info().types, [ExtensionType::Frontend]);
    assert_description_matches_session(|request| extension.protocol().handle(request));
}

#[test]
fn describe_validates_params_and_protocol_overlap() {
    for (params, code) in [
        (json!({}), -32602),
        (json!({"protocolVersions":["9.0"]}), -32011),
    ] {
        let request = ExtensionRequest::new("morphir.extension.describe", params, 1).unwrap();
        let response =
            morphir_extension_sdk::__dispatch_request::<MetadataOnly>(&request, &[], &[]);
        assert_eq!(response.error.unwrap().code, code);
    }
}
