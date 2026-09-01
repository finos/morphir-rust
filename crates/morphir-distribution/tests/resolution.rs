use morphir_distribution::{
    ArtifactFilename, Capability, Channel, DistributionError, ExtensionHistory, ExtensionId,
    Platform, RelativeArtifactPath, ReleaseRecord, SchemaVersion, Selection, Sha256Digest, ToolId,
    resolve,
};
use morphir_extension_sdk::protocol::MEP_VERSION;
use semver::Version;

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn release(version: &str, channels: &[&str], platform: (&str, &str)) -> String {
    release_with_mep(version, channels, platform, &[MEP_VERSION])
}

fn release_with_mep(
    version: &str,
    channels: &[&str],
    platform: (&str, &str),
    mep_versions: &[&str],
) -> String {
    serde_json::json!({
        "schemaVersion": "1.0",
        "id": "morphir-elm",
        "name": "Morphir Elm",
        "version": version,
        "channels": channels,
        "mepVersions": mep_versions,
        "capabilities": ["frontend"],
        "frontend": {
            "languages": [{"id": "elm", "fileExtensions": [".elm"]}],
            "irVersions": ["4"]
        },
        "artifacts": [{
            "runtime": "process",
            "platform": { "os": platform.0, "arch": platform.1 },
            "source": { "kind": "local-file", "path": "artifacts/morphir-elm" },
            "sha256": DIGEST,
            "filename": "morphir-elm",
            "args": ["serve"],
            "executable": true
        }]
    })
    .to_string()
}

fn portable_wasm_release() -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": "1.0",
        "id": "morphir-avro",
        "name": "Morphir Avro",
        "version": "0.1.0",
        "channels": ["stable"],
        "mepVersions": [MEP_VERSION],
        "capabilities": ["backend"],
        "backend": { "targets": ["avro"], "irVersions": ["3", "4"] },
        "artifacts": [{
            "runtime": "wasm",
            "source": {
                "kind": "local-file",
                "path": "artifacts/morphir_avro_extension.wasm"
            },
            "sha256": DIGEST,
            "filename": "morphir_avro_extension.wasm"
        }]
    })
}

fn portable_workspace_release() -> serde_json::Value {
    let mut release = portable_wasm_release();
    release["id"] = serde_json::json!("morphir-workspace-provider");
    release["name"] = serde_json::json!("Morphir Workspace Provider");
    release["capabilities"] = serde_json::json!(["workspace"]);
    release.as_object_mut().unwrap().remove("backend");
    release
}

fn frontend_release() -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": "1.0",
        "id": "morphir-gleam",
        "name": "Installed Gleam",
        "version": "1.0.0",
        "channels": ["stable"],
        "mepVersions": [MEP_VERSION],
        "capabilities": ["frontend"],
        "frontend": {
            "languages": [{"id": "gleam", "fileExtensions": [".gleam"]}],
            "irVersions": ["4"]
        },
        "artifacts": [{
            "runtime": "process",
            "platform": { "os": "linux", "arch": "x86_64" },
            "source": { "kind": "local-file", "path": "artifacts/morphir-gleam" },
            "sha256": DIGEST,
            "filename": "morphir-gleam",
            "args": ["serve"],
            "executable": true
        }]
    })
}

fn parse_error(record: &serde_json::Value) -> String {
    ExtensionHistory::parse_jsonl(record.to_string().as_bytes())
        .unwrap_err()
        .to_string()
}

#[test]
fn portable_extension_identity_rejects_path_like_values() {
    assert_eq!(
        ExtensionId::parse("morphir-elm").unwrap().as_str(),
        "morphir-elm"
    );
    assert!(ExtensionId::parse("e".repeat(238)).is_ok());
    for invalid in [
        "",
        "Morphir Elm",
        "../morphir-elm",
        "morphir/elm",
        "-elm",
        "con",
        "com1",
        &"e".repeat(239),
    ] {
        assert!(ExtensionId::parse(invalid).is_err(), "accepted {invalid:?}");
    }
}

#[test]
fn portable_tool_identity_rejects_windows_device_names() {
    assert_eq!(ToolId::parse("desktop").unwrap().as_str(), "desktop");
    assert!(ToolId::parse("t".repeat(182)).is_ok());
    for invalid in ["con", "aux", "nul", "com1", "lpt9", &"t".repeat(183)] {
        assert!(ToolId::parse(invalid).is_err(), "accepted {invalid:?}");
    }
}

#[test]
fn channels_parse_the_supported_spellings() {
    for spelling in ["stable", "preview", "insiders", "preview/nightly"] {
        let channel = Channel::parse(spelling).unwrap();
        assert_eq!(channel.as_str(), spelling);
    }
    assert!(Channel::parse("beta").is_err());
    assert!(Channel::parse("preview/../nightly").is_err());
}

#[test]
fn sha256_digest_has_a_canonical_lowercase_encoding() {
    let digest = Sha256Digest::parse(&DIGEST.to_uppercase()).unwrap();
    assert_eq!(digest.to_string(), DIGEST);
    assert!(Sha256Digest::parse("abcd").is_err());
    assert!(Sha256Digest::parse(&"z".repeat(64)).is_err());
}

#[test]
fn sha256_digest_rejects_non_ascii_without_panicking() {
    let non_ascii = format!("{}a", "aé".repeat(21));
    assert_eq!(non_ascii.len(), 64);
    let parsed = std::panic::catch_unwind(|| Sha256Digest::parse(&non_ascii));
    assert!(parsed.is_ok(), "digest parser panicked on non-ASCII input");
    assert!(parsed.unwrap().is_err());
}

#[test]
fn artifact_filename_is_one_portable_path_component() {
    assert_eq!(
        ArtifactFilename::parse("morphir-elm.exe").unwrap().as_str(),
        "morphir-elm.exe"
    );
    for invalid in [
        "",
        ".",
        "..",
        "bin/morphir-elm",
        "bin\\morphir-elm",
        "morphir:elm",
        "morphir?.exe",
        "morphir-elm.",
        "morphir-elm ",
        "CON",
        "con.exe",
        "LPT9.log",
        "COM¹.exe",
        "LPT²",
        "com³.txt",
        "CONIN$",
        "conout$.log",
        "CON .txt",
        "COM1 .exe",
        "CONIN$ .json",
        &"a".repeat(256),
    ] {
        assert!(
            ArtifactFilename::parse(invalid).is_err(),
            "accepted {invalid:?}"
        );
    }
}

#[test]
fn equal_semver_precedence_with_different_build_metadata_is_rejected() {
    let first = release("1.0.0+linux", &["stable"], ("linux", "x86_64"));
    let second = release("1.0.0+rebuilt", &["stable"], ("linux", "x86_64"));
    for bytes in [
        format!("{first}\n{second}\n"),
        format!("{second}\n{first}\n"),
    ] {
        let error = ExtensionHistory::parse_jsonl(bytes.as_bytes()).unwrap_err();
        assert!(error.to_string().contains("equal semantic precedence"));
    }
}

#[test]
fn jsonl_history_parses_schema_versioned_records_and_hashes_exact_bytes() {
    let bytes = format!(
        "{}\n{}\n",
        release("1.0.0", &["stable"], ("linux", "x86_64")),
        release("1.1.0-preview.1", &["preview"], ("linux", "x86_64"))
    );
    let history = ExtensionHistory::parse_jsonl(bytes.as_bytes()).unwrap();

    assert_eq!(history.extension_id().as_str(), "morphir-elm");
    assert_eq!(history.releases().len(), 2);
    assert_eq!(
        history.revision(),
        &Sha256Digest::of_bytes(bytes.as_bytes())
    );
}

#[test]
fn release_records_reject_malformed_schema_versions() {
    for version in [
        serde_json::json!(1),
        serde_json::json!("1"),
        serde_json::json!("1.0.0"),
        serde_json::json!("01.0"),
    ] {
        let mut record = portable_wasm_release();
        record["schemaVersion"] = version;

        assert!(serde_json::from_value::<ReleaseRecord>(record).is_err());
    }
}

#[test]
fn release_records_report_well_formed_unsupported_schema_version_ranges() {
    for version in ["1.1", "2.0"] {
        let mut record = portable_wasm_release();
        record["schemaVersion"] = serde_json::json!(version);

        let error = serde_json::from_value::<ReleaseRecord>(record)
            .expect_err("the release record schema version should be rejected");

        assert!(
            error
                .to_string()
                .contains(&format!("schema version {version}"))
        );
        assert!(
            error
                .to_string()
                .contains("supported range is 1.0 through 1.0")
        );
    }
}

#[test]
fn jsonl_histories_reject_malformed_schema_version_wires() {
    for version in [
        serde_json::json!(1),
        serde_json::json!("1"),
        serde_json::json!("1.0.0"),
        serde_json::json!("01.0"),
    ] {
        let mut record = portable_wasm_release();
        record["schemaVersion"] = version;

        let error = ExtensionHistory::parse_jsonl(record.to_string().as_bytes())
            .expect_err("the history schema version should be rejected");

        assert!(matches!(
            error,
            DistributionError::InvalidRecord { line: 1, .. }
        ));
    }
}

#[test]
fn jsonl_histories_report_well_formed_unsupported_schema_version_ranges() {
    for version in ["1.1", "2.0"] {
        let mut record = portable_wasm_release();
        record["schemaVersion"] = serde_json::json!(version);

        let error = ExtensionHistory::parse_jsonl(record.to_string().as_bytes())
            .expect_err("the history schema version should be rejected");

        let version = version.parse::<SchemaVersion>().unwrap();
        assert!(matches!(
            error,
            DistributionError::UnsupportedSchema {
                line: 1,
                version: actual,
                minimum,
                maximum,
            } if actual == version
                && minimum == SchemaVersion::new(1, 0)
                && maximum == SchemaVersion::new(1, 0)
        ));
    }
}

#[test]
fn jsonl_history_rejects_malformed_lines_and_mixed_identities() {
    assert!(ExtensionHistory::parse_jsonl(b"not json\n").is_err());
    let mixed = format!(
        "{}\n{}\n",
        release("1.0.0", &["stable"], ("linux", "x86_64")),
        release("1.1.0", &["stable"], ("linux", "x86_64")).replace("morphir-elm", "morphir-scala")
    );
    assert!(ExtensionHistory::parse_jsonl(mixed.as_bytes()).is_err());
}

#[test]
fn index_records_reject_unknown_fields_and_empty_required_collections() {
    let base: serde_json::Value =
        serde_json::from_str(&release("1.0.0", &["stable"], ("linux", "x86_64"))).unwrap();

    let mut cases = Vec::new();
    let mut unknown_release = base.clone();
    unknown_release["mepVersion"] = serde_json::json!(["0.1"]);
    cases.push(unknown_release);
    let mut unknown_artifact = base.clone();
    unknown_artifact["artifacts"][0]["sha265"] = serde_json::json!(DIGEST);
    cases.push(unknown_artifact);
    let mut unknown_platform = base.clone();
    unknown_platform["artifacts"][0]["platform"]["architecture"] = serde_json::json!("x86_64");
    cases.push(unknown_platform);
    let mut unknown_source = base.clone();
    unknown_source["artifacts"][0]["source"]["url"] = serde_json::json!("file://outside");
    cases.push(unknown_source);
    let mut empty_name = base.clone();
    empty_name["name"] = serde_json::json!("  ");
    cases.push(empty_name);
    for field in ["mepVersions", "capabilities", "artifacts"] {
        let mut empty = base.clone();
        empty[field] = serde_json::json!([]);
        cases.push(empty);
    }

    for invalid in cases {
        assert!(
            ExtensionHistory::parse_jsonl(invalid.to_string().as_bytes()).is_err(),
            "accepted invalid record {invalid}"
        );
    }
}

#[test]
fn relative_artifact_paths_use_a_normalized_portable_utf8_grammar() {
    assert_eq!(
        RelativeArtifactPath::parse("artifacts/linux/morphir-elm")
            .unwrap()
            .as_path(),
        std::path::Path::new("artifacts/linux/morphir-elm")
    );
    for invalid in [
        "",
        "/absolute",
        "C:/absolute",
        "../outside",
        "artifacts/../outside",
        "./artifacts/tool",
        "artifacts//tool",
        "artifacts\\tool",
        "artifacts/AUX/tool",
        "artifacts/trailing./tool",
        "artifacts/a*b/tool",
    ] {
        assert!(
            RelativeArtifactPath::parse(invalid).is_err(),
            "accepted {invalid:?}"
        );
    }
}

#[test]
fn duplicate_versions_are_rejected_independent_of_record_order() {
    let first = release("1.0.0", &["stable"], ("linux", "x86_64"));
    let second = release("1.0.0", &["preview"], ("linux", "x86_64"));
    for bytes in [
        format!("{first}\n{second}\n"),
        format!("{second}\n{first}\n"),
    ] {
        let error = ExtensionHistory::parse_jsonl(bytes.as_bytes()).unwrap_err();
        assert!(error.to_string().contains("duplicate version 1.0.0"));
    }
}

#[test]
fn stable_selects_the_highest_non_prerelease_for_the_platform() {
    let bytes = [
        release("1.0.0", &["stable"], ("linux", "x86_64")),
        release("1.2.0", &["stable"], ("macos", "aarch64")),
        release("1.1.0", &["stable"], ("linux", "x86_64")),
        release(
            "2.0.0-preview.1",
            &["stable", "preview"],
            ("linux", "x86_64"),
        ),
    ]
    .join("\n");
    let history = ExtensionHistory::parse_jsonl(bytes.as_bytes()).unwrap();

    let selected = resolve(
        &history,
        &Selection::Channel(Channel::Stable),
        &Platform::new("linux", "x86_64").unwrap(),
    )
    .unwrap();

    assert_eq!(
        selected.release().version(),
        &Version::parse("1.1.0").unwrap()
    );
    assert_eq!(selected.artifact().platform().unwrap().os(), "linux");
}

#[test]
fn stable_skips_a_newer_release_with_no_host_supported_mep_version() {
    let bytes = [
        release("1.5.0", &["stable"], ("linux", "x86_64")),
        release_with_mep("2.0.0", &["stable"], ("linux", "x86_64"), &["999.0"]),
    ]
    .join("\n");
    let history = ExtensionHistory::parse_jsonl(bytes.as_bytes()).unwrap();

    let selected = resolve(
        &history,
        &Selection::Channel(Channel::Stable),
        &Platform::new("linux", "x86_64").unwrap(),
    )
    .unwrap();

    assert_eq!(
        selected.release().version(),
        &Version::parse("1.5.0").unwrap()
    );
    assert!(
        selected
            .release()
            .mep_versions()
            .iter()
            .any(|version| version == MEP_VERSION)
    );
}

#[test]
fn exact_and_channel_selection_reject_releases_with_no_supported_mep_version() {
    let bytes = release_with_mep("2.0.0", &["stable"], ("linux", "x86_64"), &["999.0"]);
    let history = ExtensionHistory::parse_jsonl(bytes.as_bytes()).unwrap();
    let platform = Platform::new("linux", "x86_64").unwrap();

    for selection in [
        Selection::Exact(Version::parse("2.0.0").unwrap()),
        Selection::Channel(Channel::Stable),
    ] {
        let error = resolve(&history, &selection, &platform).unwrap_err();
        match error {
            DistributionError::NoCompatibleMepVersion {
                selection: rejected,
                supported,
            } => {
                assert_eq!(rejected, selection.to_string());
                assert!(supported.split(", ").any(|version| version == MEP_VERSION));
            }
            other => panic!("expected NoCompatibleMepVersion, got {other}"),
        }
    }
}

#[test]
fn preview_and_insiders_resolve_the_same_preview_family_but_preserve_request() {
    let bytes = [
        release("1.0.0", &["stable"], ("linux", "x86_64")),
        release("1.1.0-preview.1", &["preview"], ("linux", "x86_64")),
        release("1.1.0-preview.2", &["preview/nightly"], ("linux", "x86_64")),
    ]
    .join("\n");
    let history = ExtensionHistory::parse_jsonl(bytes.as_bytes()).unwrap();
    let platform = Platform::new("linux", "x86_64").unwrap();

    for channel in [Channel::Preview(None), Channel::Insiders] {
        let selected = resolve(&history, &Selection::Channel(channel.clone()), &platform).unwrap();
        assert_eq!(
            selected.release().version(),
            &Version::parse("1.1.0-preview.2").unwrap()
        );
        assert_eq!(selected.selection(), &Selection::Channel(channel));
    }

    let nightly = resolve(
        &history,
        &Selection::Channel(Channel::parse("preview/nightly").unwrap()),
        &platform,
    )
    .unwrap();
    assert_eq!(
        nightly.release().version(),
        &Version::parse("1.1.0-preview.2").unwrap()
    );
}

#[test]
fn exact_selection_ignores_channels_and_selects_prereleases() {
    let bytes = release("2.0.0-rc.1", &[], ("linux", "x86_64"));
    let history = ExtensionHistory::parse_jsonl(bytes.as_bytes()).unwrap();
    let exact = Version::parse("2.0.0-rc.1").unwrap();
    let selected = resolve(
        &history,
        &Selection::Exact(exact.clone()),
        &Platform::new("linux", "x86_64").unwrap(),
    )
    .unwrap();
    assert_eq!(selected.release().version(), &exact);
}

#[test]
fn ambiguous_platform_artifacts_are_rejected() {
    let mut record: serde_json::Value =
        serde_json::from_str(&release("1.0.0", &["stable"], ("linux", "x86_64"))).unwrap();
    let duplicate = record["artifacts"][0].clone();
    record["artifacts"].as_array_mut().unwrap().push(duplicate);
    let history = ExtensionHistory::parse_jsonl(record.to_string().as_bytes()).unwrap();

    let error = resolve(
        &history,
        &Selection::Channel(Channel::Stable),
        &Platform::new("linux", "x86_64").unwrap(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("more than one artifact"));
}

#[test]
fn portable_wasm_resolves_on_linux_and_macos() {
    let record = portable_wasm_release();
    let history = ExtensionHistory::parse_jsonl(record.to_string().as_bytes()).unwrap();

    for platform in [
        Platform::new("linux", "x86_64").unwrap(),
        Platform::new("macos", "aarch64").unwrap(),
    ] {
        let selected = resolve(&history, &Selection::Channel(Channel::Stable), &platform).unwrap();
        assert_eq!(
            selected.artifact().filename().as_str(),
            "morphir_avro_extension.wasm"
        );
    }
}

#[test]
fn portable_wasm_reports_no_platform_without_panicking() {
    let record = portable_wasm_release();
    let history = ExtensionHistory::parse_jsonl(record.to_string().as_bytes()).unwrap();
    let selected = resolve(
        &history,
        &Selection::Channel(Channel::Stable),
        &Platform::new("linux", "x86_64").unwrap(),
    )
    .unwrap();

    assert!(selected.artifact().platform().is_none());
    assert!(
        serde_json::to_value(selected.artifact())
            .unwrap()
            .get("platform")
            .is_none()
    );
}

#[test]
fn portable_wasm_rejects_each_process_only_field() {
    let base = portable_wasm_release();
    let mut with_platform = base.clone();
    with_platform["artifacts"][0]["platform"] =
        serde_json::json!({ "os": "linux", "arch": "x86_64" });
    assert!(parse_error(&with_platform).contains("wasm artifacts must be portable"));

    let mut with_args = base.clone();
    with_args["artifacts"][0]["args"] = serde_json::json!(["serve"]);
    assert!(parse_error(&with_args).contains("wasm artifacts must be portable"));

    let mut executable = base;
    executable["artifacts"][0]["executable"] = serde_json::json!(true);
    assert!(parse_error(&executable).contains("wasm artifacts must be portable"));
}

#[test]
fn portable_wasm_rejects_an_explicit_null_platform() {
    let mut record = portable_wasm_release();
    record["artifacts"][0]["platform"] = serde_json::Value::Null;
    assert!(parse_error(&record).contains("wasm artifacts must be portable"));
}

#[test]
fn process_artifacts_require_a_platform() {
    let mut process: serde_json::Value =
        serde_json::from_str(&release("1.0.0", &["stable"], ("linux", "x86_64"))).unwrap();
    process["artifacts"][0]
        .as_object_mut()
        .unwrap()
        .remove("platform");
    assert!(parse_error(&process).contains("process artifacts require a platform"));

    process["artifacts"][0]["platform"] = serde_json::Value::Null;
    assert!(parse_error(&process).contains("process artifacts require a platform"));
}

#[test]
fn schema_1_0_accepts_process_artifacts() {
    let record = release("1.0.0", &["stable"], ("linux", "x86_64"));
    let history = ExtensionHistory::parse_jsonl(record.as_bytes()).unwrap();
    let selected = resolve(
        &history,
        &Selection::Channel(Channel::Stable),
        &Platform::new("linux", "x86_64").unwrap(),
    )
    .unwrap();

    assert_eq!(
        selected.release().schema_version(),
        SchemaVersion::new(1, 0)
    );
    assert_eq!(
        selected.artifact().runtime(),
        morphir_distribution::ArtifactRuntime::Process
    );
    assert_eq!(selected.artifact().args(), ["serve"]);
    assert!(selected.artifact().executable());
}

#[test]
fn schema_1_0_backend_metadata_requires_exactly_the_backend_capability() {
    let mut missing = portable_wasm_release();
    missing.as_object_mut().unwrap().remove("backend");
    assert!(parse_error(&missing).contains("backend metadata is required"));

    let mut unexpected = portable_wasm_release();
    unexpected["capabilities"] = serde_json::json!(["frontend"]);
    assert!(parse_error(&unexpected).contains("backend metadata requires"));

    let mut null_without_capability = portable_wasm_release();
    null_without_capability["capabilities"] = serde_json::json!(["frontend"]);
    null_without_capability["backend"] = serde_json::Value::Null;
    assert!(parse_error(&null_without_capability).contains("backend metadata requires"));

    let mut null_with_capability = portable_wasm_release();
    null_with_capability["backend"] = serde_json::Value::Null;
    assert!(parse_error(&null_with_capability).contains("backend metadata is required"));
}

#[test]
fn backend_metadata_rejects_empty_or_duplicate_values() {
    let base = portable_wasm_release();
    let mut empty_targets = base.clone();
    empty_targets["backend"]["targets"] = serde_json::json!([]);
    assert!(parse_error(&empty_targets).contains("backend targets"));

    let mut duplicate_targets = base.clone();
    duplicate_targets["backend"]["targets"] = serde_json::json!(["avro", "avro"]);
    assert!(parse_error(&duplicate_targets).contains("backend targets"));

    let mut empty_ir_versions = base.clone();
    empty_ir_versions["backend"]["irVersions"] = serde_json::json!([]);
    assert!(parse_error(&empty_ir_versions).contains("backend IR versions"));

    let mut duplicate_ir_versions = base;
    duplicate_ir_versions["backend"]["irVersions"] = serde_json::json!(["3", "3"]);
    assert!(parse_error(&duplicate_ir_versions).contains("backend IR versions"));
}

#[test]
fn backend_metadata_rejects_blank_values() {
    for (field, value, expected_error) in [
        ("targets", "", "backend targets"),
        ("targets", "   ", "backend targets"),
        ("irVersions", "", "backend IR versions"),
        ("irVersions", "   ", "backend IR versions"),
    ] {
        let mut record = portable_wasm_release();
        record["backend"][field] = serde_json::json!([value]);
        assert!(
            parse_error(&record).contains(expected_error),
            "accepted blank {field} entry {value:?}"
        );
    }
}

#[test]
fn backend_metadata_rejects_surrounding_whitespace_and_trimmed_duplicates() {
    for (field, values, expected_error) in [
        ("targets", vec![" avro"], "backend targets"),
        ("targets", vec!["avro", "avro "], "backend targets"),
        ("irVersions", vec!["4 "], "backend IR versions"),
        ("irVersions", vec!["4", " 4"], "backend IR versions"),
    ] {
        let mut record = portable_wasm_release();
        record["backend"][field] = serde_json::json!(values);
        assert!(
            parse_error(&record).contains(expected_error),
            "accepted whitespace-padded {field} entries"
        );
    }
}

#[test]
fn backend_metadata_is_exposed_by_the_release_domain() {
    let record = portable_wasm_release();
    let history = ExtensionHistory::parse_jsonl(record.to_string().as_bytes()).unwrap();
    let backend = history.releases()[0].backend().unwrap();

    assert_eq!(backend.targets(), ["avro"]);
    assert_eq!(backend.ir_versions(), ["3", "4"]);
    assert!(
        backend.generate(),
        "omitted generate must remain compatible"
    );
}

#[test]
fn frontend_metadata_is_exposed_by_the_release_domain() {
    let history = ExtensionHistory::parse_jsonl(frontend_release().to_string().as_bytes()).unwrap();
    let frontend = history.releases()[0].frontend().unwrap();

    assert_eq!(frontend.languages().len(), 1);
    assert_eq!(frontend.languages()[0].id(), "gleam");
    assert_eq!(frontend.languages()[0].file_extensions(), [".gleam"]);
    assert_eq!(frontend.ir_versions(), ["4"]);
    assert!(frontend.compile(), "omitted compile must default to true");
}

#[test]
fn schema_1_0_frontend_metadata_requires_exactly_the_frontend_capability() {
    let mut missing = frontend_release();
    missing.as_object_mut().unwrap().remove("frontend");
    assert!(parse_error(&missing).contains("frontend metadata is required"));

    let mut unexpected = frontend_release();
    unexpected["capabilities"] = serde_json::json!(["validator"]);
    assert!(parse_error(&unexpected).contains("frontend metadata requires"));

    let mut null_without_capability = frontend_release();
    null_without_capability["capabilities"] = serde_json::json!(["validator"]);
    null_without_capability["frontend"] = serde_json::Value::Null;
    assert!(parse_error(&null_without_capability).contains("frontend metadata requires"));

    let mut null_with_capability = frontend_release();
    null_with_capability["frontend"] = serde_json::Value::Null;
    assert!(parse_error(&null_with_capability).contains("frontend metadata is required"));
}

#[test]
fn frontend_languages_must_be_non_empty_unique_trimmed_records() {
    let mut no_languages = frontend_release();
    no_languages["frontend"]["languages"] = serde_json::json!([]);
    assert!(parse_error(&no_languages).contains("frontend languages"));

    for languages in [
        serde_json::json!([{"id": "", "fileExtensions": [".gleam"]}]),
        serde_json::json!([{"id": " gleam", "fileExtensions": [".gleam"]}]),
        serde_json::json!([
            {"id": "gleam", "fileExtensions": [".gleam"]},
            {"id": "gleam", "fileExtensions": [".g"]}
        ]),
        serde_json::json!([
            {"id": "gleam", "fileExtensions": [".gleam"]},
            {"id": "gleam ", "fileExtensions": [".g"]}
        ]),
    ] {
        let mut record = frontend_release();
        record["frontend"]["languages"] = languages;
        assert!(parse_error(&record).contains("frontend languages"));
    }
}

#[test]
fn frontend_file_extensions_must_be_non_empty_unique_trimmed_dot_prefixed_values() {
    for extensions in [
        serde_json::json!([]),
        serde_json::json!([""]),
        serde_json::json!(["gleam"]),
        serde_json::json!([" .gleam"]),
        serde_json::json!([".gleam", ".gleam"]),
        serde_json::json!([".gleam", ".gleam "]),
    ] {
        let mut record = frontend_release();
        record["frontend"]["languages"][0]["fileExtensions"] = extensions;
        assert!(parse_error(&record).contains("frontend file extensions"));
    }
}

#[test]
fn frontend_ir_versions_must_be_non_empty_unique_trimmed_values() {
    for versions in [
        serde_json::json!([]),
        serde_json::json!([""]),
        serde_json::json!([" 4"]),
        serde_json::json!(["4", "4"]),
        serde_json::json!(["4", "4 "]),
    ] {
        let mut record = frontend_release();
        record["frontend"]["irVersions"] = versions;
        assert!(parse_error(&record).contains("frontend IR versions"));
    }
}

#[test]
fn schema_1_0_release_can_declare_frontend_and_backend_metadata() {
    let mut record = frontend_release();
    record["capabilities"] = serde_json::json!(["frontend", "backend"]);
    record["frontend"]["compile"] = serde_json::json!(false);
    record["backend"] = serde_json::json!({"targets": ["avro"], "irVersions": ["4"]});

    let release: ReleaseRecord = serde_json::from_value(record).unwrap();
    assert!(!release.frontend().unwrap().compile());
    assert_eq!(release.backend().unwrap().targets(), ["avro"]);
}

#[test]
fn schema_1_0_accepts_and_exposes_workspace_capabilities() {
    let record = portable_workspace_release();
    let history = ExtensionHistory::parse_jsonl(record.to_string().as_bytes()).unwrap();

    assert_eq!(
        history.releases()[0].capabilities(),
        [Capability::Workspace]
    );
}

#[test]
fn schema_1_0_rejects_unknown_capabilities() {
    let mut record = portable_workspace_release();
    record["capabilities"] = serde_json::json!(["arbitrary-filesystem"]);

    assert!(parse_error(&record).contains("unknown variant"));
}

#[test]
fn portable_wasm_matching_artifacts_are_rejected_as_ambiguous() {
    let mut record = portable_wasm_release();
    let duplicate = record["artifacts"][0].clone();
    record["artifacts"].as_array_mut().unwrap().push(duplicate);
    let history = ExtensionHistory::parse_jsonl(record.to_string().as_bytes()).unwrap();

    let error = resolve(
        &history,
        &Selection::Channel(Channel::Stable),
        &Platform::new("linux", "x86_64").unwrap(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("more than one artifact"));
}

#[test]
fn host_process_and_portable_wasm_artifacts_are_rejected_as_ambiguous() {
    let mut record = portable_wasm_release();
    record["artifacts"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "runtime": "process",
            "platform": { "os": "linux", "arch": "x86_64" },
            "source": { "kind": "local-file", "path": "artifacts/morphir-avro" },
            "sha256": DIGEST,
            "filename": "morphir-avro",
            "args": [],
            "executable": false
        }));
    let history = ExtensionHistory::parse_jsonl(record.to_string().as_bytes()).unwrap();

    let error = resolve(
        &history,
        &Selection::Channel(Channel::Stable),
        &Platform::new("linux", "x86_64").unwrap(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("more than one artifact"));
}
