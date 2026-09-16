//! Exact-byte and normalized-metadata SHA-256 digests.

use sha2::{Digest as _, Sha256};
use std::fmt;

/// A SHA-256 digest. Display uses `sha256:` and lowercase hexadecimal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest([u8; 32]);

impl Digest {
    /// Hash bytes exactly, including BOMs, whitespace and line endings.
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self::of_parts(&[bytes])
    }
    pub(crate) fn of_parts(parts: &[&[u8]]) -> Self {
        let mut hash = Sha256::new();
        for part in parts {
            hash.update(part);
        }
        Self(hash.finalize().into())
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("sha256:")?;
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}
