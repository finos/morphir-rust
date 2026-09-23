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
//! A budget too small to hold that hash and a character of the name is refused
//! rather than overrun — see [`MIN_TRUNCATED_STEM_BUDGET`].

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

/// The smallest `available` budget [`truncate_stem`] can shorten a stem into.
///
/// A truncated stem is `__` plus eight hex digits of the content hash, which is
/// ten characters that carry nothing of the name, plus at least one character of
/// the name itself. Anything smaller has no truncation to offer: the answer
/// would be the hash alone, which no longer reads as the name it stands for.
pub const MIN_TRUNCATED_STEM_BUDGET: usize = 11;

/// Shorten an escaped stem to fit within `available` characters, or `None` when
/// `available` is below [`MIN_TRUNCATED_STEM_BUDGET`].
///
/// Keeps the first `available - 10` characters of `escaped`, trims a trailing
/// run of `-` or `_` that the cut exposed, and appends `__` followed by the
/// first 8 hex characters of the SHA-256 digest of the untruncated `escaped`
/// stem. The hash keeps stems that share a long common prefix distinct after
/// truncation.
///
/// The result always fits in `available` characters. A budget that cannot hold
/// the hash and a character of the name is refused rather than answered with a
/// stem that overruns it, so a caller holding a real path budget has to say what
/// it does about a name it has no room for.
pub fn truncate_stem(escaped: &str, available: usize) -> Option<String> {
    if available < MIN_TRUNCATED_STEM_BUDGET {
        return None;
    }
    let keep = available - 10;
    let prefix: String = escaped.chars().take(keep).collect();
    let trimmed = prefix.trim_end_matches(['-', '_']);

    Some(format!("{trimmed}__{}", short_hash(escaped)))
}

/// Whether `stem` is the file stem of the name that escapes to `escaped`: the escaped stem
/// itself, or a cut [`truncate_stem`] could have made of it under some budget.
///
/// A reader that finds a cut stem cannot recover the name from it, but it can check a name the
/// file states against the stem the file is under.
pub fn is_stem_of(stem: &str, escaped: &str) -> bool {
    if stem == escaped {
        return true;
    }
    let Some((kept, hash)) = stem.rsplit_once("__") else {
        return false;
    };
    !kept.is_empty() && escaped.starts_with(kept) && hash == short_hash(escaped)
}

/// The first eight hex digits of the SHA-256 digest of `escaped`.
fn short_hash(escaped: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(escaped.as_bytes());
    let digest = hasher.finalize();
    digest
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
