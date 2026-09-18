//! SHA-256 digest helper shared across the binding (prelude content digest,
//! source/interface digests in later tasks).

use sha2::{Digest, Sha256};

/// Hex-encodes the SHA-256 digest of `bytes`, prefixed with `sha256:`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(7 + digest.len() * 2);
    out.push_str("sha256:");
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
