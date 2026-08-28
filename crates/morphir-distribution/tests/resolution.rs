use morphir_distribution::{
    ArtifactFilename, Channel, ExtensionHistory, ExtensionId, Platform, Selection, Sha256Digest,
    resolve,
};
use semver::Version;

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn release(version: &str, channels: &[&str], platform: (&str, &str)) -> String {
    serde_json::json!({
        "schemaVersion": 1,
        "id": "morphir-elm",
        "name": "Morphir Elm",
        "version": version,
        "channels": channels,
        "mepVersions": ["0.1"],
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
    for invalid in ["", "Morphir Elm", "../morphir-elm", "morphir/elm", "-elm"] {
        assert!(ExtensionId::parse(invalid).is_err(), "accepted {invalid:?}");
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
fn artifact_filename_is_one_portable_path_component() {
    assert_eq!(
        ArtifactFilename::parse("morphir-elm.exe").unwrap().as_str(),
        "morphir-elm.exe"
    );
    for invalid in ["", ".", "..", "bin/morphir-elm", "bin\\morphir-elm"] {
        assert!(
            ArtifactFilename::parse(invalid).is_err(),
            "accepted {invalid:?}"
        );
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

    assert_eq!(selected.release().version, Version::parse("1.1.0").unwrap());
    assert_eq!(selected.artifact().platform.os(), "linux");
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
            selected.release().version,
            Version::parse("1.1.0-preview.2").unwrap()
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
        nightly.release().version,
        Version::parse("1.1.0-preview.2").unwrap()
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
    assert_eq!(selected.release().version, exact);
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
