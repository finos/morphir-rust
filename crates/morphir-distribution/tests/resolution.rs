use morphir_distribution::{
    ArtifactFilename, Channel, DistributionError, ExtensionHistory, ExtensionId, Platform,
    RelativeArtifactPath, Selection, Sha256Digest, ToolId, resolve,
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
        "schemaVersion": 1,
        "id": "morphir-elm",
        "name": "Morphir Elm",
        "version": version,
        "channels": channels,
        "mepVersions": mep_versions,
        "capabilities": ["frontend"],
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

#[test]
fn portable_extension_identity_rejects_path_like_values() {
    assert_eq!(
        ExtensionId::parse("morphir-elm").unwrap().as_str(),
        "morphir-elm"
    );
    for invalid in [
        "",
        "Morphir Elm",
        "../morphir-elm",
        "morphir/elm",
        "-elm",
        "con",
        "com1",
    ] {
        assert!(ExtensionId::parse(invalid).is_err(), "accepted {invalid:?}");
    }
}

#[test]
fn portable_tool_identity_rejects_windows_device_names() {
    assert_eq!(ToolId::parse("desktop").unwrap().as_str(), "desktop");
    for invalid in ["con", "aux", "nul", "com1", "lpt9"] {
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
    assert_eq!(selected.artifact().platform().os(), "linux");
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
