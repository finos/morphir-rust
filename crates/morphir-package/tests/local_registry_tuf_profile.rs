use morphir_package::local_registry::{TufRole, tuf::decode_profile};
use serde_json::{Value, json};

fn metadata(role: TufRole) -> Value {
    let name = match role {
        TufRole::Root => "root",
        TufRole::Timestamp => "timestamp",
        TufRole::Snapshot => "snapshot",
        TufRole::Targets => "targets",
    };
    let mut signed = json!({
        "_type": name, "spec_version": "1.0.36", "version": 1,
        "expires": "2030-01-01T00:00:00Z"
    });
    match role {
        TufRole::Root => {
            let id = "a".repeat(64);
            signed["consistent_snapshot"] = json!(true);
            signed["keys"] = json!({id.clone(): {
                "keytype": "ed25519", "scheme": "ed25519",
                "keyval": {"public": "b".repeat(64)}
            }});
            signed["roles"] = ["root", "timestamp", "snapshot", "targets"]
                .into_iter()
                .map(|name| (name.to_owned(), json!({"keyids": [id], "threshold": 1})))
                .collect();
        }
        TufRole::Timestamp => signed["meta"] = json!({"snapshot.json": link()}),
        TufRole::Snapshot => signed["meta"] = json!({"targets.json": link()}),
        TufRole::Targets => signed["targets"] = json!({"records/release.json": target()}),
    }
    json!({"signed": signed, "signatures": [{"keyid": "a".repeat(64), "sig": "c".repeat(128)}]})
}

fn target() -> Value {
    json!({"length": 0, "hashes": {"sha256": "d".repeat(64), "sha512": "e".repeat(128)}})
}

fn link() -> Value {
    let mut value = target();
    value["version"] = json!(1);
    value
}

fn decode(value: &Value, role: TufRole) -> bool {
    decode_profile(&serde_json::to_vec(value).unwrap(), role).is_ok()
}

#[test]
fn accepts_four_role_shapes_without_claiming_authentication() {
    for role in [
        TufRole::Root,
        TufRole::Timestamp,
        TufRole::Snapshot,
        TufRole::Targets,
    ] {
        let value = metadata(role);
        let bytes = serde_json::to_vec_pretty(&value).unwrap();
        let decoded = decode_profile(&bytes, role).unwrap();
        assert_eq!(decoded.bytes(), bytes);
        assert_eq!(decoded.document(), &value);
        assert_eq!(format!("{:?}", decoded.role()), format!("{role:?}"));
    }
}

#[test]
fn rejects_non_profile_common_fields() {
    for role in [
        TufRole::Root,
        TufRole::Timestamp,
        TufRole::Snapshot,
        TufRole::Targets,
    ] {
        for (field, wrong) in [
            ("_type", json!("other")),
            ("spec_version", json!("1.0.35")),
            ("version", json!(0)),
            ("version", json!(-1)),
            ("version", json!("1")),
            ("expires", json!(null)),
        ] {
            let mut value = metadata(role);
            value["signed"][field] = wrong;
            assert!(
                !decode(&value, role),
                "accepted invalid {field} for {role:?}"
            );
        }
        for field in ["_type", "spec_version", "version", "expires"] {
            let mut value = metadata(role);
            value["signed"].as_object_mut().unwrap().remove(field);
            assert!(!decode(&value, role), "accepted missing {field}");
        }
    }
}

#[test]
fn rejects_duplicate_decoded_keys_bom_and_float_extensions() {
    let valid = serde_json::to_string(&metadata(TufRole::Targets)).unwrap();
    for text in [
        valid.replacen("\"version\":1", "\"version\":1,\"\\u0076ersion\":1", 1),
        format!("\u{feff}{valid}"),
        valid.replacen("\"version\":1", "\"version\":1,\"extra\":1e0", 1),
        valid.replacen("\"version\":1", "\"version\":1.0", 1),
    ] {
        assert!(decode_profile(text.as_bytes(), TufRole::Targets).is_err());
    }
}

#[test]
fn preserves_unknown_fields_unicode_and_exact_large_integers() {
    let mut value = metadata(TufRole::Targets);
    let large: Value = serde_json::from_str("184467440737095516170000000000001").unwrap();
    value["signed"]["version"] = large.clone();
    value["signed"]["extra"] = json!({"e\u{301}": ["é", "e\u{301}", null, false, large]});
    value["outside"] = json!({"unknown": -17});
    let bytes = serde_json::to_vec(&value).unwrap();
    assert_eq!(
        decode_profile(&bytes, TufRole::Targets).unwrap().document(),
        &value
    );
}

#[test]
fn requires_profile_root_keys_roles_and_consistent_snapshots() {
    let role = TufRole::Root;
    let id = "a".repeat(64);
    let mutations = [
        ("/signed/consistent_snapshot", json!(false)),
        ("/signed/roles/root/threshold", json!(0)),
        ("/signed/roles/root/threshold", json!(2)),
        (
            "/signed/roles/root/threshold",
            serde_json::from_str("184467440737095516160").unwrap(),
        ),
        ("/signed/roles/root/keyids", json!([])),
        ("/signed/roles/root/keyids", json!(["not-hex"])),
    ];
    for (pointer, wrong) in mutations {
        let mut value = metadata(role);
        *value.pointer_mut(pointer).unwrap() = wrong;
        assert!(!decode(&value, role), "accepted {pointer}");
    }
    for (field, wrong) in [("keytype", json!("rsa")), ("scheme", json!("ed25519ph"))] {
        let mut value = metadata(role);
        value["signed"]["keys"][&id][field] = wrong;
        assert!(!decode(&value, role));
    }
    for public in ["a".repeat(62), "g".repeat(64)] {
        let mut value = metadata(role);
        value["signed"]["keys"][&id]["keyval"]["public"] = json!(public);
        assert!(!decode(&value, role));
    }
    let mut value = metadata(role);
    value["signed"]["roles"]
        .as_object_mut()
        .unwrap()
        .remove("snapshot");
    assert!(!decode(&value, role));
}

#[test]
fn requires_exact_parent_membership_and_complete_links() {
    for (role, name) in [
        (TufRole::Timestamp, "snapshot.json"),
        (TufRole::Snapshot, "targets.json"),
    ] {
        for field in ["version", "length", "hashes"] {
            let mut value = metadata(role);
            value["signed"]["meta"][name]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(!decode(&value, role));
        }
        for (field, wrong) in [
            ("version", json!(0)),
            ("length", json!(-1)),
            ("length", json!("0")),
        ] {
            let mut value = metadata(role);
            value["signed"]["meta"][name][field] = wrong;
            assert!(!decode(&value, role));
        }
        let mut value = metadata(role);
        value["signed"]["meta"]["other.json"] = link();
        assert!(!decode(&value, role));
        value["signed"]["meta"]
            .as_object_mut()
            .unwrap()
            .remove(name);
        assert!(!decode(&value, role));
    }
}

#[test]
fn validates_supported_hash_widths_and_target_lengths() {
    let role = TufRole::Targets;
    for (algorithm, hash) in [
        ("sha256", "0".repeat(62)),
        ("sha512", "0".repeat(126)),
        ("sha256", "x".repeat(64)),
    ] {
        let mut value = metadata(role);
        value["signed"]["targets"]["records/release.json"]["hashes"][algorithm] = json!(hash);
        assert!(!decode(&value, role));
    }
    for field in ["length", "hashes"] {
        let mut value = metadata(role);
        value["signed"]["targets"]["records/release.json"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(!decode(&value, role));
    }
    let mut value = metadata(role);
    value["signed"]["targets"]["records/release.json"]["hashes"]
        .as_object_mut()
        .unwrap()
        .remove("sha256");
    assert!(!decode(&value, role));
}

#[test]
fn rejects_delegations_and_malformed_signature_fields() {
    let role = TufRole::Targets;
    for delegation in [json!(null), json!({"keys": {}, "roles": []})] {
        let mut value = metadata(role);
        value["signed"]["delegations"] = delegation;
        assert!(!decode(&value, role));
    }
    for signature in [
        json!({}),
        json!({"keyid": "aa", "sig": 1}),
        json!({"keyid": "gg", "sig": "aa"}),
    ] {
        let mut value = metadata(role);
        value["signatures"] = json!([signature]);
        assert!(!decode(&value, role));
    }
    let mut value = metadata(role);
    value["signatures"] = json!([]);
    assert!(
        decode(&value, role),
        "signature quorum belongs to authentication"
    );
}

#[test]
fn signature_and_role_key_counts_accept_boundary_reject_first_excess() {
    for count in [64, 65] {
        let mut value = metadata(TufRole::Targets);
        value["signatures"] = Value::Array(vec![value["signatures"][0].clone(); count]);
        assert_eq!(decode(&value, TufRole::Targets), count == 64);
        let mut value = metadata(TufRole::Root);
        value["signed"]["roles"]["root"]["keyids"] = json!(vec!["a".repeat(64); count]);
        assert_eq!(decode(&value, TufRole::Root), count == 64);
    }
}

#[test]
fn root_key_and_target_counts_accept_boundary_reject_first_excess() {
    for count in [256, 257] {
        let mut value = metadata(TufRole::Root);
        let key = value["signed"]["keys"]["a".repeat(64)].clone();
        value["signed"]["keys"] = (0..count)
            .map(|i| (format!("{i:064x}"), key.clone()))
            .collect();
        assert_eq!(decode(&value, TufRole::Root), count == 256);
    }
    for count in [8192, 8193] {
        let mut value = metadata(TufRole::Targets);
        value["signed"]["targets"] = (0..count)
            .map(|i| (format!("records/r{i}.json"), target()))
            .collect();
        assert_eq!(decode(&value, TufRole::Targets), count == 8192);
    }
}

#[test]
fn document_byte_limits_accept_boundary_reject_first_excess() {
    for (role, maximum) in [
        (TufRole::Root, 1_048_576),
        (TufRole::Timestamp, 1_048_576),
        (TufRole::Snapshot, 1_048_576),
        (TufRole::Targets, 16_777_216),
    ] {
        let mut bytes = serde_json::to_vec(&metadata(role)).unwrap();
        bytes.resize(maximum, b' ');
        assert!(decode_profile(&bytes, role).is_ok());
        bytes.push(b' ');
        assert!(decode_profile(&bytes, role).is_err());
    }
}

#[test]
fn ignored_extension_depth_accepts_64_and_rejects_65() {
    for depth in [64, 65] {
        let mut value = metadata(TufRole::Targets);
        let mut extension = json!("leaf");
        for _ in 2..depth {
            extension = json!([extension]);
        }
        value["signed"]["extension"] = extension;
        assert_eq!(decode(&value, TufRole::Targets), depth == 64);
    }
}
