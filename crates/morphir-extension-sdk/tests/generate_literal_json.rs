use morphir_extension_sdk::protocol::{ExtensionRequest, methods};
use morphir_extension_sdk::{
    Backend, BackendCapability, Extension, ExtensionCapabilities, ExtensionInfo, GenerateRequest,
    GenerateResult, NativeExtension,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// Run with --ignored --nocapture to compare flat and deeply nested opaque data.
/// Timing is diagnostic rather than a machine-dependent CI pass threshold.
#[test]
#[ignore = "manual literal JSON scaling measurement"]
fn literal_json_scaling_measurement() {
    for depth in [0, 120] {
        let content = "x".repeat(4 * 1024 * 1024);
        let ir = format!("{}\"{content}\"{}", "[".repeat(depth), "]".repeat(depth));
        let source = format!(r#"{{"ir":{ir},"target":"literal-json","options":{{}}}}"#);
        let started = std::time::Instant::now();
        let decoded: GenerateRequest = serde_json::from_str(std::hint::black_box(&source)).unwrap();
        let elapsed = started.elapsed();
        let mut value = &decoded.ir;
        for _ in 0..depth {
            value = &value[0];
        }
        assert_eq!(value.as_str(), Some(content.as_str()));
        eprintln!(
            "literal-json depth={depth} bytes={} elapsed_us={}",
            source.len(),
            elapsed.as_micros()
        );
    }
}

#[test]
fn container_materialization_matches_json_scalars_and_member_semantics() {
    // Delimiters inside escaped strings are data. Duplicate members retain the
    // active serde_json map semantics, including its configured key ordering.
    let ir = r#" {
        "z" : [ {}, [], "commas, brackets[] and braces{}", "quote\" slash\\" ],
        "a" : { "雪" : [ true, false, null, -0, 1e-20, 18446744073709551615 ] },
        "duplicate" : 1,
        "duplicate" : { "\\u0041" : "\u0041", "colon:" : [ [ [ "end" ] ] ] }
    } "#;
    let source = format!(r#"{{"ir":{ir},"target":"literal-json","options":{{}}}}"#);
    let decoded: GenerateRequest = serde_json::from_str(&source).unwrap();
    let expected: Value = serde_json::from_str(ir).unwrap();
    assert_eq!(decoded.ir, expected);
    assert_eq!(decoded.ir.to_string(), expected.to_string());
}

#[derive(Default)]
struct RecordingBackend(Arc<Mutex<Vec<GenerateRequest>>>);

impl Extension for RecordingBackend {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "literal-json-backend".into(),
            ..Default::default()
        }
    }

    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities {
            backend: Some(BackendCapability {
                targets: vec!["literal-json".into()],
                ir_versions: vec!["4".into()],
                generate: true,
            }),
            ..Default::default()
        }
    }
}

impl Backend for RecordingBackend {
    fn target_languages() -> Vec<String> {
        vec!["literal-json".into()]
    }

    fn generate(&self, request: GenerateRequest) -> morphir_extension_sdk::Result<GenerateResult> {
        self.0.lock().unwrap().push(request);
        Ok(GenerateResult {
            success: true,
            artifacts: vec![],
            diagnostics: vec![],
        })
    }
}

fn initialized_backend() -> (NativeExtension, Arc<Mutex<Vec<GenerateRequest>>>) {
    let backend = RecordingBackend::default();
    let requests = backend.0.clone();
    let guest = NativeExtension::backend_only(backend).unwrap();
    let initialized = guest.protocol().handle(
        ExtensionRequest::new(
            methods::INITIALIZE,
            json!({"protocolVersions":["0.1"],"host":{"name":"literal-json-test","version":"1"}}),
            1,
        )
        .unwrap(),
    );
    assert!(initialized.error.is_none(), "{:?}", initialized.error);
    (guest, requests)
}

#[test]
fn native_protocol_preserves_literal_ir_and_options_objects() {
    let (guest, requests) = initialized_backend();
    let ir = json!({"nested":[{"$serde_json::private::Number":"1"},42,true,null]});
    let option = json!({"$serde_json::private::RawValue":"{\"x\":1,\"x\":2}"});
    let response = guest.protocol().handle(
        ExtensionRequest::new(
            methods::GENERATE,
            json!({"ir":ir,"target":"literal-json","options":{"literal":option}}),
            2,
        )
        .unwrap(),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    let recorded = requests.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].ir, ir);
    assert_eq!(recorded[0].options["literal"], option);
    assert!(matches!(recorded[0].ir, Value::Object(_)));
}

#[test]
fn wire_requests_preserve_escaped_marker_keys_and_nested_values() {
    let raw = r#"{"jsonrpc":"2.0","id":2,"method":"morphir.backend.generate","params":{"ir":{"\u0024serde_json::private::Number":"not a number","nested":[{"\u0024serde_json::private::RawValue":"{\"x\":1,\"x\":2}"},false,7]},"target":"literal-json","options":{"literal":{"$serde_json::private::RawValue":"true"}}}}"#;
    let request: ExtensionRequest = serde_json::from_str(raw).unwrap();
    assert_eq!(
        request.params["ir"]["$serde_json::private::Number"],
        "not a number"
    );
    assert_eq!(
        request.params["ir"]["nested"][0]["$serde_json::private::RawValue"],
        r#"{"x":1,"x":2}"#
    );
    assert_eq!(
        request.params["options"]["literal"]["$serde_json::private::RawValue"],
        "true"
    );
}

#[test]
fn existing_wire_container_depth_bounds_include_typed_wrappers() {
    for (depth, accepted) in [(125, true), (126, false)] {
        let ir = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
        let raw = format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"morphir.backend.generate","params":{{"ir":{ir},"target":"literal-json"}}}}"#
        );
        assert_eq!(
            serde_json::from_str::<ExtensionRequest>(&raw).is_ok(),
            accepted,
            "request depth {depth}"
        );
    }
    for (depth, accepted) in [(126, true), (127, false)] {
        let ir = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
        let raw = format!(r#"{{"ir":{ir},"target":"literal-json"}}"#);
        assert_eq!(
            serde_json::from_str::<GenerateRequest>(&raw).is_ok(),
            accepted,
            "generate depth {depth}"
        );
    }
}

#[test]
fn generation_keeps_outer_validation_default_options_and_numeric_values() {
    let (guest, requests) = initialized_backend();
    for (index, params) in [
        json!(null),
        json!([]),
        json!({"ir":{}}),
        json!({"target":"literal-json"}),
        json!({"ir":{},"target":7}),
        json!({"ir":{},"target":"literal-json","options":null}),
        json!({"ir":{},"target":"literal-json","options":[]}),
    ]
    .into_iter()
    .enumerate()
    {
        let response = guest
            .protocol()
            .handle(ExtensionRequest::new(methods::GENERATE, params, index as u64 + 2).unwrap());
        assert_eq!(response.error.unwrap().code, -32602);
    }
    assert!(requests.lock().unwrap().is_empty());
    let numbers = [
        "18446744073709551615",
        "-0.0",
        "1e0",
        "0.123456789012345678901234567890",
        "1e-20",
    ]
    .map(|token| serde_json::from_str::<serde_json::Number>(token).unwrap());
    let ir = Value::Array(numbers.iter().cloned().map(Value::Number).collect());
    let response = guest.protocol().handle(
        ExtensionRequest::new(
            methods::GENERATE,
            json!({"ir":ir,"target":"future-target","future":true}),
            20,
        )
        .unwrap(),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    let recorded = requests.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].options.is_empty());
    assert_eq!(recorded[0].target, "future-target");
    let actual = recorded[0].ir.as_array().unwrap();
    for (value, expected) in actual.iter().zip(numbers) {
        assert_eq!(value.as_number().unwrap().to_string(), expected.to_string());
    }
    assert_eq!(actual[0].as_u64(), Some(u64::MAX));
}

#[test]
fn wire_ir_retains_the_active_json_map_ordering() {
    let raw = r#"{"jsonrpc":"2.0","id":2,"method":"morphir.backend.generate","params":{"ir":{"z":1,"a":2,"middle":{"$serde_json::private::RawValue":"true"}},"target":"literal-json"}}"#;
    let request: ExtensionRequest = serde_json::from_str(raw).unwrap();
    let expected = json!({"z":1,"a":2,"middle":{"$serde_json::private::RawValue":"true"}});
    assert_eq!(request.params["ir"].to_string(), expected.to_string());
}

#[test]
fn raw_wire_numbers_keep_the_active_parser_numeric_fidelity() {
    for number in [
        "18446744073709551615",
        "-0.0",
        "1e0",
        "0.123456789012345678901234567890",
        "1e-20",
    ] {
        let raw = format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"morphir.backend.generate","params":{{"ir":{number},"target":"literal-json"}}}}"#
        );
        let request: ExtensionRequest = serde_json::from_str(&raw).unwrap();
        let expected: serde_json::Number = serde_json::from_str(number).unwrap();
        assert_eq!(
            request.params["ir"].as_number().unwrap().to_string(),
            expected.to_string()
        );
    }
}

#[test]
fn deeper_constructed_ir_returns_invalid_params_before_backend_execution() {
    let (guest, requests) = initialized_backend();
    let ir = (0..127).fold(json!(0), |value, _| Value::Array(vec![value]));
    let response = guest.protocol().handle(
        ExtensionRequest::new(
            methods::GENERATE,
            json!({"ir":ir,"target":"literal-json"}),
            2,
        )
        .unwrap(),
    );
    let error = response.error.unwrap();
    assert_eq!(error.code, -32602);
    assert!(error.message.contains("recursion limit exceeded"));
    assert!(error.message.len() < 256);
    assert!(requests.lock().unwrap().is_empty());
}
