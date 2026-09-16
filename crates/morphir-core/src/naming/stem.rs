//! Document-tree file-stem escaping and length truncation.
//!
//! A [`Name`] projects onto a filename stem via [`Name::to_file_stem`]; [`file_stem`]
//! is the free-function form of that projection. [`escaped_path`] extends the
//! projection across a [`Path`], joining each segment's escaped stem with `/`.
//!
//! A path that exceeds the filesystem's length budget is shortened by
//! [`truncate_stem`]: it keeps a prefix of the escaped stem, trims a trailing
//! separator that the cut may have exposed, and appends a short content hash so
//! two names that share a long common prefix still truncate to distinct stems.

use sha2::{Digest, Sha256};

use super::{Name, Path};

/// The filename stem `name` projects onto in a document tree.
///
/// See [`Name::to_file_stem`] for the escaping rule: an initialism segment
/// carries a `_` prefix, and a stem that collides with a Windows reserved
/// device name carries a `_` suffix.
pub fn file_stem(name: &Name) -> String {
    name.to_file_stem()
}

/// The escaped filesystem path `path` projects onto, joining each segment's
/// [`file_stem`] with `/`.
pub fn escaped_path(path: &Path) -> String {
    path.segments
        .iter()
        .map(file_stem)
        .collect::<Vec<_>>()
        .join("/")
}

/// Shorten an escaped stem to fit within `available` characters.
///
/// Keeps the first `available - 10` characters of `escaped`, trims a trailing
/// run of `-` or `_` that the cut exposed, and appends `__` followed by the
/// first 8 hex characters of the SHA-256 digest of the untruncated `escaped`
/// stem. The hash keeps stems that share a long common prefix distinct after
/// truncation.
pub fn truncate_stem(escaped: &str, available: usize) -> String {
    let keep = available.saturating_sub(10);
    let prefix: String = escaped.chars().take(keep).collect();
    let trimmed = prefix.trim_end_matches(['-', '_']);

    let mut hasher = Sha256::new();
    hasher.update(escaped.as_bytes());
    let digest = hasher.finalize();
    let hash_hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    let short_hash = &hash_hex[..8];

    format!("{trimmed}__{short_hash}")
}
