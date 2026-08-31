use super::*;
use morphir_extension_sdk::{BackendCapability, ExtensionInfo, ExtensionType};
use tokio::io::{BufReader, duplex};

#[test]
fn discovered_process_launch_retains_exact_negotiation_metadata() {
    let discovered = ExtensionInfo {
        id: "morphir-elm".into(),
        name: "Morphir Elm".into(),
        version: "3.2.1".into(),
        types: vec![ExtensionType::Frontend],
        ..ExtensionInfo::default()
    };
    let launch =
        ProcessLaunch::from_discovered(discovered.clone(), "/verified/morphir-elm", "/workspace");

    assert_eq!(launch.extension_id, discovered.id);
    let retained = launch
        .discovered
        .expect("verified launches should retain discovery metadata");
    assert_eq!(retained.name, discovered.name);
    assert_eq!(retained.version, discovered.version);
    assert_eq!(retained.types, discovered.types);
}

#[test]
fn compatibility_initialization_rejects_locked_backend_capability_drift() {
    let discovered = ExtensionInfo {
        id: "example".into(),
        name: "Example".into(),
        version: "1.0.0".into(),
        types: vec![ExtensionType::Backend],
        ..ExtensionInfo::default()
    };
    let locked_backend = BackendCapability {
        targets: vec!["avro".into()],
        ir_versions: vec!["3".into(), "4.0.0".into()],
        generate: true,
    };
    let expected = ExpectedExtension::discovered_with_capabilities(
        discovered.clone(),
        ExtensionCapabilities {
            backend: Some(locked_backend.clone()),
            ..ExtensionCapabilities::default()
        },
    );
    let initialized = InitializeResult {
        protocol_version: "0.1".into(),
        extension: discovered,
        capabilities: ExtensionCapabilities {
            backend: Some(BackendCapability {
                generate: false,
                ..locked_backend
            }),
            ..ExtensionCapabilities::default()
        },
    };

    let error = validate_compatibility_initialization(expected, &["0.1".into()], initialized)
        .expect_err("compatibility sessions must enforce discovery-time capability locks");

    assert!(
        error
            .to_string()
            .contains("backend capabilities disagreed with discovery"),
        "{error}"
    );
}

#[test]
fn compatibility_negotiation_retains_disabled_generate_support() {
    let extension = ExtensionInfo {
        id: "example".into(),
        name: "Example".into(),
        version: "1.0.0".into(),
        types: vec![ExtensionType::Backend],
        ..ExtensionInfo::default()
    };
    let initialized = InitializeResult {
        protocol_version: "0.1".into(),
        extension: extension.clone(),
        capabilities: ExtensionCapabilities {
            backend: Some(BackendCapability {
                targets: vec!["avro".into()],
                ir_versions: vec!["4".into()],
                generate: false,
            }),
            ..ExtensionCapabilities::default()
        },
    };

    let negotiated = validate_compatibility_initialization(
        ExpectedExtension::discovered(extension),
        &["0.1".into()],
        initialized,
    )
    .unwrap();

    assert!(!negotiated.supports_method(methods::GENERATE));
}

#[tokio::test]
async fn compatibility_invoke_rejects_unsafe_generated_artifacts() {
    let error = validate_compatibility_method_result(
        methods::GENERATE,
        &serde_json::json!({}),
        serde_json::json!({
            "success": true,
            "artifacts": [{"path": "../../escape.avsc", "content": "{}"}],
            "diagnostics": []
        }),
    )
    .await
    .expect_err("compatibility results must use the shared artifact validator");

    assert!(error.to_string().contains("artifact path"), "{error}");
}

#[tokio::test]
async fn verified_bytes_stage_under_the_explicit_managed_directory() {
    let root = tempfile::tempdir().unwrap();
    let staging_directory = root.path().join("managed-staging");
    fs::create_dir(&staging_directory).unwrap();
    let launch = ProcessLaunch::from_verified_bytes_in(
        ExtensionInfo {
            id: "example".into(),
            ..ExtensionInfo::default()
        },
        OsStr::new("example"),
        b"#!/bin/sh\n",
        &staging_directory,
        root.path(),
    );

    let (program, _retained_directory) = prepare_program(&launch.program).await.unwrap();

    assert!(program.starts_with(&staging_directory));
}

#[test]
fn verified_bytes_reject_path_components_without_writing_outside_staging() {
    let root = tempfile::tempdir().unwrap();
    let staging_directory = root.path().join("managed-staging");
    let absolute_escape = root.path().join("absolute-escape");

    for filename in [
        OsString::from("../relative-escape"),
        OsString::from("nested/escape"),
        absolute_escape.clone().into_os_string(),
    ] {
        let error = stage_verified_program(
            filename,
            Arc::from(&b"#!/bin/sh\n"[..]),
            Some(staging_directory.clone()),
        )
        .expect_err("verified process filename must be a single basename");

        assert!(error.to_string().contains("single filename"), "{error}");
    }
    assert!(!absolute_escape.exists());
    assert!(!staging_directory.join("relative-escape").exists());
    assert!(!staging_directory.join("nested/escape").exists());
}

#[tokio::test]
async fn content_length_frames_round_trip_formatted_json() {
    let (mut writer, reader) = duplex(1024);
    let value = serde_json::json!({ "message": "line one\nline two" });
    let expected = value.clone();
    let writing = tokio::spawn(async move { write_frame(&mut writer, &value).await });
    let body = read_frame(&mut BufReader::new(reader))
        .await
        .expect("the frame should parse");
    writing.await.expect("the writer task should join").unwrap();

    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        expected
    );
}

#[tokio::test]
async fn stdout_logs_are_rejected_as_protocol_headers() {
    let (mut writer, reader) = duplex(1024);
    writer.write_all(b"accidental log line\n").await.unwrap();
    drop(writer);

    let error = read_frame(&mut BufReader::new(reader))
        .await
        .expect_err("stdout logs must not be treated as protocol data");
    assert!(
        error
            .to_string()
            .contains("Invalid extension protocol header")
    );
}

#[test]
fn stderr_capture_retains_only_the_bounded_tail() {
    let mut output = b"old diagnostics".to_vec();
    append_bounded_tail(&mut output, b"new diagnostics", 16);

    assert_eq!(output, b"snew diagnostics");

    append_bounded_tail(&mut output, b"0123456789abcdefghijkl", 16);
    assert_eq!(output, b"6789abcdefghijkl");
}
