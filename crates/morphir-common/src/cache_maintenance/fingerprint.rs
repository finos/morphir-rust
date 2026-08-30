use cap_std::fs::Metadata;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub(crate) fn metadata_hasher(metadata: &Metadata) -> DefaultHasher {
    let mut hasher = DefaultHasher::new();
    metadata.is_dir().hash(&mut hasher);
    metadata.is_file().hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    metadata.modified().ok().hash(&mut hasher);
    hash_native_metadata(metadata, &mut hasher);
    hasher
}

pub(crate) fn metadata_fingerprint(metadata: &Metadata) -> u64 {
    metadata_hasher(metadata).finish()
}

#[cfg(unix)]
fn hash_native_metadata(metadata: &Metadata, hasher: &mut DefaultHasher) {
    use cap_std::fs::MetadataExt;

    metadata.dev().hash(hasher);
    metadata.ino().hash(hasher);
    metadata.mode().hash(hasher);
    metadata.mtime().hash(hasher);
    metadata.mtime_nsec().hash(hasher);
}

#[cfg(windows)]
fn hash_native_metadata(metadata: &Metadata, hasher: &mut DefaultHasher) {
    use cap_std::fs::MetadataExt;

    metadata.file_attributes().hash(hasher);
    metadata.creation_time().hash(hasher);
    metadata.last_write_time().hash(hasher);
    metadata.file_size().hash(hasher);
}

#[cfg(not(any(unix, windows)))]
fn hash_native_metadata(_metadata: &Metadata, _hasher: &mut DefaultHasher) {}

pub(crate) fn finish<T: Hash>(mut hasher: DefaultHasher, children: &[(T, u64)]) -> u64 {
    for (name, fingerprint) in children {
        name.hash(&mut hasher);
        fingerprint.hash(&mut hasher);
    }
    hasher.finish()
}
