use super::CacheEntry;
use same_file::Handle;

pub(crate) struct PinnedCacheEntry {
    entry: CacheEntry,
    handle: Option<Handle>,
    fingerprint: Option<u64>,
}

impl PinnedCacheEntry {
    pub(super) fn new(entry: CacheEntry, handle: Option<Handle>, fingerprint: Option<u64>) -> Self {
        Self {
            entry,
            handle,
            fingerprint,
        }
    }

    pub(crate) fn entry(&self) -> &CacheEntry {
        &self.entry
    }

    pub(crate) fn handle(&self) -> Option<&Handle> {
        self.handle.as_ref()
    }

    pub(crate) fn fingerprint(&self) -> Option<u64> {
        self.fingerprint
    }

    pub(crate) fn into_entry(self) -> CacheEntry {
        self.entry
    }
}
