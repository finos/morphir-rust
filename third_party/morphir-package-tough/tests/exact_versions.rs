use tough::schema::{Signed, Targets};

#[test]
fn signed_version_above_u64_survives_parse_and_serialize() {
    let raw = r#"{"signed":{"_type":"targets","spec_version":"1.0.0","version":18446744073709551616,"expires":"2035-01-01T00:00:00Z","targets":{}},"signatures":[]}"#;
    let parsed: Signed<Targets> = serde_json::from_str(raw).unwrap();
    let encoded = serde_json::to_value(parsed).unwrap();
    assert_eq!(
        encoded["signed"]["version"].to_string(),
        "18446744073709551616"
    );
}

#[test]
fn rejects_non_positive_and_non_integer_versions() {
    for version in [
        "0",
        "-1",
        "1.5",
        "1e2",
        "\"1\"",
        "null",
        "true",
        "[]",
        "{}",
        "1.0",
        "1e0",
        r#"{"$serde_json::private::Number":"1"}"#,
        r#"{"$serde_json::private::RawValue":"1"}"#,
    ] {
        let raw = format!(
            r#"{{"signed":{{"_type":"targets","spec_version":"1.0.0","version":{version},"expires":"2035-01-01T00:00:00Z","targets":{{}}}},"signatures":[]}}"#
        );
        assert!(
            serde_json::from_slice::<Signed<Targets>>(raw.as_bytes()).is_err(),
            "accepted {}",
            version
        );
    }
}

#[test]
fn exact_order_successor_and_predecessor_cross_machine_boundaries() {
    use tough::schema::Version;
    for (before, after) in [
        ("1", "2"),
        ("9", "10"),
        ("99", "100"),
        ("9007199254740991", "9007199254740992"),
        ("18446744073709551615", "18446744073709551616"),
        (
            "99999999999999999999999999999999999999999999999999",
            "100000000000000000000000000000000000000000000000000",
        ),
    ] {
        let before: Version = before.parse().unwrap();
        let after: Version = after.parse().unwrap();
        assert!(before < after);
        assert_eq!(before.successor(), after);
        assert_eq!(after.predecessor().as_ref(), Some(&before));
        let json = serde_json::to_string(&after).unwrap();
        assert_eq!(json, after.to_string());
        assert_eq!(serde_json::from_str::<Version>(&json).unwrap(), after);
    }
    assert_eq!(Version::new(1).unwrap().predecessor(), None);
    for input in [
        "", "0", "00", "01", "+1", "-1", " 1", "1 ", "1.0", "1e2", "١",
    ] {
        assert!(input.parse::<Version>().is_err(), "accepted {}", input);
    }
}

#[test]
fn every_role_and_metafile_preserves_exact_versions_and_filenames() {
    use tough::schema::{Role, Root, Snapshot, Timestamp};
    let number = "18446744073709551616";
    let role = |kind: &str, rest: &str| {
        format!(
            r#"{{"_type":"{kind}","spec_version":"1.0.0","version":{number},"expires":"2035-01-01T00:00:00Z",{rest}}}"#
        )
    };
    let root: Root = serde_json::from_str(&role(
        "root",
        r#""keys":{},"roles":{},"consistent_snapshot":true"#,
    ))
    .unwrap();
    let targets: Targets = serde_json::from_str(&role("targets", r#""targets":{}"#)).unwrap();
    let snapshot: Snapshot = serde_json::from_str(&role(
        "snapshot",
        &format!(r#""meta":{{"targets.json":{{"version":{number}}}}}"#),
    ))
    .unwrap();
    let timestamp: Timestamp = serde_json::from_str(&role(
        "timestamp",
        &format!(r#""meta":{{"snapshot.json":{{"version":{number}}}}}"#),
    ))
    .unwrap();
    assert_eq!(root.filename(true), format!("{number}.root.json"));
    assert_eq!(targets.filename(true), format!("{number}.targets.json"));
    assert_eq!(snapshot.filename(true), format!("{number}.snapshot.json"));
    assert_eq!(timestamp.filename(true), "timestamp.json");
    assert_eq!(snapshot.meta["targets.json"].version.to_string(), number);
    assert_eq!(timestamp.meta["snapshot.json"].version.to_string(), number);
    for value in [
        serde_json::to_value(root).unwrap(),
        serde_json::to_value(targets).unwrap(),
        serde_json::to_value(snapshot).unwrap(),
        serde_json::to_value(timestamp).unwrap(),
    ] {
        assert_eq!(value["version"].to_string(), number);
    }
}

#[test]
fn normalized_upstream_test_keys_preserve_original_public_keys() {
    use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
    for (original, normalized) in [
        (
            include_bytes!("data/targetskey").as_slice(),
            include_bytes!("data/targetskey.pkcs8").as_slice(),
        ),
        (
            include_bytes!("data/targetskey-1").as_slice(),
            include_bytes!("data/targetskey-1.pkcs8").as_slice(),
        ),
    ] {
        assert_eq!(&normalized[16..48], &original[16..48]);
        let pair = Ed25519KeyPair::from_pkcs8(normalized).unwrap();
        assert_eq!(pair.public_key().as_ref(), &original[53..85]);
    }
}
