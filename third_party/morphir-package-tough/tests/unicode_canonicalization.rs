//! Frozen code-point semantics with signatures authored independently of Tough.
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tough::schema::{key::Key, Role, Root, Signed, Snapshot, Targets, Timestamp};

#[allow(dead_code)]
#[path = "storage_support/fixtures.rs"]
mod fixtures;

fn canonical_fixture(value: &Value) -> Vec<u8> {
    // These signed fixtures use integers and strings without ASCII controls.
    // serde_json sorts object members here but never normalizes Unicode.
    let mut value = value.clone();
    value.sort_all_objects();
    serde_json::to_vec(&value).unwrap()
}
fn sign(body: Value, seed: u8, keyid: &str) -> Vec<u8> {
    let key = ed25519_zebra::SigningKey::from([seed; 32]);
    let signature = key.sign(&canonical_fixture(&body));
    canonical_fixture(
        &json!({"signed": body, "signatures": [{"keyid": keyid, "sig": hex::encode(signature.to_bytes())}]}),
    )
}
fn authority() -> Signed<Root> {
    serde_json::from_slice(&fixtures::root(1, 18)).unwrap()
}
fn check_role<T: Role + DeserializeOwned>(wire: &[u8], seed: u8, keyid: &str) {
    let mut document: Value = serde_json::from_slice(wire).unwrap();
    document["signed"]["extension"] = json!({"e\u{301}":"A\u{30a}", "é":"distinct"});
    document["signed"]["large"] =
        serde_json::from_str("184467440737095516160000000000000000000").unwrap();
    let body = document["signed"].clone();
    let signed: Signed<T> = serde_json::from_slice(&sign(body.clone(), seed, keyid)).unwrap();
    assert_eq!(
        signed.signed.canonical_form().unwrap(),
        canonical_fixture(&body)
    );
    authority().signed.verify_role(&signed).unwrap();
}
#[test]
fn every_role_preserves_unknown_unicode_keys_and_values_in_signed_bytes() {
    let root = authority();
    let root_id = hex::encode(root.signed.roles[&tough::schema::RoleType::Root].keyids[0].as_ref());
    let online_id =
        hex::encode(root.signed.roles[&tough::schema::RoleType::Targets].keyids[0].as_ref());
    let (timestamp, snapshot, targets) = fixtures::view(1, 18);
    check_role::<Root>(&fixtures::root(1, 18), 17, &root_id);
    check_role::<Timestamp>(&timestamp, 18, &online_id);
    check_role::<Snapshot>(&snapshot, 18, &online_id);
    check_role::<Targets>(&targets, 18, &online_id);
}
#[test]
fn changing_normalization_form_without_resigning_is_rejected() {
    let root = authority();
    let keyid =
        hex::encode(root.signed.roles[&tough::schema::RoleType::Targets].keyids[0].as_ref());
    let (_, _, targets) = fixtures::view(1, 18);
    let mut document: Value = serde_json::from_slice(&targets).unwrap();
    document["signed"]["extension"] = json!("é");
    let wire = sign(document["signed"].clone(), 18, &keyid);
    let original: Signed<Targets> = serde_json::from_slice(&wire).unwrap();
    root.signed.verify_role(&original).unwrap();
    let mut altered: Value = serde_json::from_slice(&wire).unwrap();
    altered["signed"]["extension"] = json!("e\u{301}");
    let altered: Signed<Targets> = serde_json::from_slice(&canonical_fixture(&altered)).unwrap();
    assert!(root.signed.verify_role(&altered).is_err());
}
#[test]
fn key_ids_preserve_unicode_in_unknown_fields() {
    let root: Value = serde_json::from_slice(&fixtures::root(1, 18)).unwrap();
    let mut key = root["signed"]["keys"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    key["extension"] = json!({"e\u{301}":"A\u{30a}","é":"distinct"});
    let expected = Sha256::digest(canonical_fixture(&key));
    let decoded: Key = serde_json::from_slice(&canonical_fixture(&key)).unwrap();
    assert_eq!(decoded.key_id().unwrap().as_ref(), expected.as_slice());
}
#[test]
fn control_escaping_sorting_and_exact_unknown_integers_are_unchanged() {
    let body = concat!(
        "{\"_type\":\"targets\",\"expires\":\"2100-01-01T00:00:00Z\",",
        "\"extension\":{\"é\":\"\\u0000\\t\\n\\r\\\\\\\"/\",\"é\":true},",
        "\"large\":184467440737095516160000000000000000000,",
        "\"spec_version\":\"1.0.36\",\"targets\":{},\"version\":1}"
    );
    let targets: Targets = serde_json::from_str(body).unwrap();
    let expected = concat!(
        "{\"_type\":\"targets\",\"expires\":\"2100-01-01T00:00:00Z\",",
        "\"extension\":{\"é\":\"\0\t\n\r\\\\\\\"/\",\"é\":true},",
        "\"large\":184467440737095516160000000000000000000,",
        "\"spec_version\":\"1.0.36\",\"targets\":{},\"version\":1}"
    );
    assert_eq!(targets.canonical_form().unwrap(), expected.as_bytes());
}
