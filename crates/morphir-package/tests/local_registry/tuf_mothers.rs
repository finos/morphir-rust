//! Independent ASCII/integer fixture signer; never calls the production canonicalizer.
use ed25519_zebra::{SigningKey, VerificationKeyBytes};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
pub fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}
pub fn canonical(value: &Value) -> Vec<u8> {
    let mut value = value.clone();
    value.sort_all_objects();
    serde_json::to_vec(&value).unwrap()
}
pub fn key(seed: u8) -> Value {
    json!({"keytype":"ed25519","scheme":"ed25519","keyval":{"public":hex(VerificationKeyBytes::from(&SigningKey::from([seed;32])).as_ref())}})
}
pub fn id(key: &Value) -> String {
    hex(Sha256::digest(canonical(key)))
}
pub fn sign(body: Value, signers: &[(u8, Value)]) -> Vec<u8> {
    let signatures: Vec<_> = signers.iter().map(|(seed,key)| json!({"keyid":id(key),"sig":hex(SigningKey::from([*seed;32]).sign(&canonical(&body)).to_bytes())})).collect();
    let mut bytes = b"\n ".to_vec();
    bytes.extend(canonical(&json!({"signed":body,"signatures":signatures})));
    bytes.push(b'\n');
    bytes
}
pub fn root_body(version: u64, keys: &[(u8, Value)], threshold: u64) -> Value {
    let map: serde_json::Map<_, _> = keys.iter().map(|(_, key)| (id(key), key.clone())).collect();
    let ids: Vec<_> = keys.iter().map(|(_, key)| id(key)).collect();
    let role = json!({"keyids":ids,"threshold":threshold});
    json!({"_type":"root","spec_version":"1.0.36","version":version,"expires":"2100-01-01T00:00:00Z","consistent_snapshot":true,"keys":map,"roles":{"root":role,"timestamp":role,"snapshot":role,"targets":role}})
}
pub fn root(version: u64, seed: u8) -> Vec<u8> {
    let keys = [(seed, key(seed))];
    sign(root_body(version, &keys, 1), &keys)
}
pub fn targets_body(version: u64) -> Value {
    json!({"_type":"targets","spec_version":"1.0.36","version":version,"expires":"2100-01-01T00:00:00Z","targets":{}})
}
pub fn targets(version: u64, seed: u8) -> Vec<u8> {
    sign(targets_body(version), &[(seed, key(seed))])
}
pub fn meta(bytes: &[u8], version: u64) -> Value {
    json!({"version":version,"length":bytes.len(),"hashes":{"sha256":hex(Sha256::digest(bytes))}})
}
pub fn snapshot(targets: &[u8], seed: u8) -> Vec<u8> {
    sign(
        json!({"_type":"snapshot","spec_version":"1.0.36","version":1,"expires":"2100-01-01T00:00:00Z","meta":{"targets.json":meta(targets,1)}}),
        &[(seed, key(seed))],
    )
}
pub fn timestamp(snapshot: &[u8], seed: u8) -> Vec<u8> {
    sign(
        json!({"_type":"timestamp","spec_version":"1.0.36","version":1,"expires":"2100-01-01T00:00:00Z","meta":{"snapshot.json":meta(snapshot,1)}}),
        &[(seed, key(seed))],
    )
}
