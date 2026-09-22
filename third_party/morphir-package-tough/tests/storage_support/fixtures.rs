//! Independent signer for small ASCII/integer TUF fixtures.
use ed25519_zebra::{SigningKey, VerificationKeyBytes};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
fn key(seed: u8) -> Value {
    json!({"keytype":"ed25519","scheme":"ed25519","keyval":{"public":hex::encode(VerificationKeyBytes::from(&SigningKey::from([seed;32])).as_ref())}})
}
fn bytes(value: &Value) -> Vec<u8> {
    let mut value = value.clone();
    value.sort_all_objects();
    serde_json::to_vec(&value).unwrap()
}
fn id(seed: u8) -> String {
    hex::encode(Sha256::digest(bytes(&key(seed))))
}
fn signed(body: Value, seed: u8) -> Vec<u8> {
    let signature = SigningKey::from([seed; 32]).sign(&bytes(&body));
    // Surround with harmless whitespace to catch accidental reserialization.
    let mut output = b"\n ".to_vec();
    output.extend(bytes(&json!({"signed":body,"signatures":[{"keyid":id(seed),"sig":hex::encode(signature.to_bytes())}]})));
    output.push(b'\n');
    output
}
pub fn root(version: u64, role_seed: u8) -> Vec<u8> {
    let root_role = json!({"keyids":[id(17)],"threshold":1});
    let role = json!({"keyids":[id(role_seed)],"threshold":1});
    let mut keys = serde_json::Map::new();
    keys.insert(id(17), key(17));
    keys.insert(id(role_seed), key(role_seed));
    signed(
        json!({"_type":"root","spec_version":"1.0.36","version":version,"expires":"2100-01-01T00:00:00Z","consistent_snapshot":true,"keys":keys,"roles":{"root":root_role,"timestamp":role,"snapshot":role,"targets":role}}),
        17,
    )
}
fn meta(document: &[u8], version: u64) -> Value {
    json!({"version":version,"length":document.len(),"hashes":{"sha256":hex::encode(Sha256::digest(document))}})
}
pub fn view(version: u64, seed: u8) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let targets = signed(
        json!({"_type":"targets","spec_version":"1.0.36","version":version,"expires":"2100-01-01T00:00:00Z","targets":{}}),
        seed,
    );
    let snapshot = signed(
        json!({"_type":"snapshot","spec_version":"1.0.36","version":version,"expires":"2100-01-01T00:00:00Z","meta":{"targets.json":meta(&targets,version)}}),
        seed,
    );
    let timestamp = signed(
        json!({"_type":"timestamp","spec_version":"1.0.36","version":version,"expires":"2100-01-01T00:00:00Z","meta":{"snapshot.json":meta(&snapshot,version)}}),
        seed,
    );
    (timestamp, snapshot, targets)
}
pub fn write_view(directory: &Path, version: u64) {
    let (timestamp, snapshot, targets) = view(version, 18);
    fs::write(directory.join("timestamp.json"), timestamp).unwrap();
    fs::write(directory.join(format!("{version}.snapshot.json")), snapshot).unwrap();
    fs::write(directory.join(format!("{version}.targets.json")), targets).unwrap();
}
pub fn create(directory: &Path) {
    fs::create_dir_all(directory).unwrap();
    fs::write(directory.join("1.root.json"), root(1, 17)).unwrap();
    fs::write(directory.join("2.root.json"), root(2, 18)).unwrap();
    write_view(directory, 1);
}

fn signed_many(body: Value, seeds: &[u8]) -> Vec<u8> {
    let canonical = bytes(&body);
    let signatures = seeds
        .iter()
        .map(|seed| {
            let signature = SigningKey::from([*seed; 32]).sign(&canonical);
            json!({"keyid":id(*seed),"sig":hex::encode(signature.to_bytes())})
        })
        .collect::<Vec<_>>();
    let mut output = b"\n ".to_vec();
    output.extend(bytes(&json!({"signed":body,"signatures":signatures})));
    output.push(b'\n');
    output
}

/// Root changes only the online timestamp/snapshot threshold, keeping both keys.
pub fn threshold_root(version: u64, threshold: u64) -> Vec<u8> {
    role_threshold_root(version, threshold, threshold)
}

pub fn role_threshold_root(
    version: u64,
    timestamp_threshold: u64,
    snapshot_threshold: u64,
) -> Vec<u8> {
    let root_role = json!({"keyids":[id(17)],"threshold":1});
    let timestamp = json!({"keyids":[id(18),id(19)],"threshold":timestamp_threshold});
    let snapshot = json!({"keyids":[id(18),id(19)],"threshold":snapshot_threshold});
    let targets = json!({"keyids":[id(18)],"threshold":1});
    let mut keys = serde_json::Map::new();
    for seed in [17, 18, 19] {
        keys.insert(id(seed), key(seed));
    }
    signed_many(
        json!({"_type":"root","spec_version":"1.0.36","version":version,"expires":"2100-01-01T00:00:00Z","consistent_snapshot":true,"keys":keys,"roles":{"root":root_role,"timestamp":timestamp,"snapshot":snapshot,"targets":targets}}),
        &[17],
    )
}

pub fn write_threshold_view(directory: &Path, version: u64, signers: &[u8]) {
    write_threshold_view_versions(directory, version, version, signers);
}

pub fn write_threshold_view_versions(
    directory: &Path,
    timestamp_version: u64,
    version: u64,
    signers: &[u8],
) {
    let targets = signed_many(
        json!({"_type":"targets","spec_version":"1.0.36","version":version,"expires":"2100-01-01T00:00:00Z","targets":{}}),
        &[18],
    );
    let snapshot = signed_many(
        json!({"_type":"snapshot","spec_version":"1.0.36","version":version,"expires":"2100-01-01T00:00:00Z","meta":{"targets.json":meta(&targets,version)}}),
        signers,
    );
    let timestamp = signed_many(
        json!({"_type":"timestamp","spec_version":"1.0.36","version":timestamp_version,"expires":"2100-01-01T00:00:00Z","meta":{"snapshot.json":meta(&snapshot,version)}}),
        signers,
    );
    fs::write(directory.join("timestamp.json"), timestamp).unwrap();
    fs::write(directory.join(format!("{version}.snapshot.json")), snapshot).unwrap();
    fs::write(directory.join(format!("{version}.targets.json")), targets).unwrap();
}

/// Independently sign a timestamp with separately selected version, link and expiry.
pub fn timestamp(
    version: u64,
    snapshot_version: u64,
    snapshot: &[u8],
    expires: &str,
    seed: u8,
) -> Vec<u8> {
    signed(
        json!({"_type":"timestamp","spec_version":"1.0.36","version":version,"expires":expires,"meta":{"snapshot.json":meta(snapshot,snapshot_version)}}),
        seed,
    )
}
